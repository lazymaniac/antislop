use crate::{
    Result, require,
    runtime::{Command, Runtime},
    storage::Store,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{BufReader, Write};
use std::os::{
    fd::AsRawFd,
    unix::{
        fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(crate) mod management_projection;
mod projection;
pub(crate) mod task_projection;

pub const PROTOCOL_VERSION: u32 = 2;
pub const DOCUMENT_CHUNK: usize = 256 * 1024;
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u32,
    pub request_id: String,
    pub idempotency_key: Option<String>,
    pub operation: Operation,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Mutate {
        command: Box<Command>,
    },
    Snapshot {
        run_id: String,
    },
    List,
    ListPage {
        after: String,
    },
    /// Which binary this daemon runs and whether it is executing work.
    Identity,
    /// Stop an idle daemon so a frontend from another build can replace it.
    Shutdown,
    Read {
        run_id: String,
        document: String,
        offset: usize,
        expected_hash: Option<String>,
    },
    Events {
        after: u64,
    },
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub version: u32,
    pub request_id: String,
    pub data: Option<Value>,
    pub error: Option<String>,
}
/// Identifies the binary file a daemon was started from. A frontend compares
/// it with its own executable so it never keeps talking to a daemon left
/// running from an older (or debug) build.
pub fn build_identity(executable: &Path) -> Result<String> {
    let path = executable.canonicalize()?;
    let metadata = std::fs::metadata(&path)?;
    let modified = metadata
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    Ok(crate::hash(format!(
        "{}\0{}\0{modified}",
        path.display(),
        metadata.len()
    )))
}
fn daemon_build() -> &'static str {
    static BUILD: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    BUILD.get_or_init(|| {
        std::env::current_exe()
            .and_then(|exe| build_identity(&exe).map_err(std::io::Error::other))
            .unwrap_or_default()
    })
}
static SHUTDOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
fn uid() -> u32 {
    // SAFETY: geteuid has no arguments or failure modes.
    unsafe { libc::geteuid() }
}
pub fn private_directory(path: &Path) -> Result<()> {
    if !path.exists() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)?;
    }
    let metadata = std::fs::symlink_metadata(path)?;
    require(
        metadata.is_dir()
            && !metadata.file_type().is_symlink()
            && metadata.uid() == uid()
            && metadata.permissions().mode() & 0o077 == 0,
        "state directory must be owned by current user with mode 0700",
    )
}
use std::os::unix::fs::DirBuilderExt;
struct DaemonLock {
    _file: std::fs::File,
    socket: PathBuf,
}
impl Drop for DaemonLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket);
    }
}
fn lock(state: &Path) -> Result<DaemonLock> {
    private_directory(state)?;
    let lock_path = state.join("daemon.lock");
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(lock_path)?;
    // SAFETY: flock operates on the owned descriptor. The file stays open for the daemon lifetime.
    require(
        unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
        "another daemon owns this state directory",
    )?;
    let socket = state.join("architect.sock");
    if let Ok(metadata) = std::fs::symlink_metadata(&socket) {
        require(
            metadata.file_type().is_socket() && metadata.uid() == uid(),
            "refusing to replace unexpected socket path",
        )?;
        require(
            UnixStream::connect(&socket).is_err(),
            "socket already has a listener",
        )?;
        std::fs::remove_file(&socket)?;
    }
    Ok(DaemonLock {
        _file: file,
        socket,
    })
}
struct PendingSocket {
    path: PathBuf,
    identity: (u64, u64),
}
impl Drop for PendingSocket {
    fn drop(&mut self) {
        if let Ok(metadata) = self.path.symlink_metadata()
            && metadata.file_type().is_socket()
            && (metadata.dev(), metadata.ino()) == self.identity
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
fn prepare_listener(socket: &Path) -> Result<(UnixListener, PendingSocket)> {
    // Keep the temporary basename as short as architect.sock for Unix path limits.
    let pending_path = socket.with_file_name(format!(".s{}", &crate::id()[..12]));
    let listener = UnixListener::bind(&pending_path)?;
    let metadata = pending_path.symlink_metadata()?;
    let pending = PendingSocket {
        path: pending_path,
        identity: (metadata.dev(), metadata.ino()),
    };
    std::fs::set_permissions(&pending.path, std::fs::Permissions::from_mode(0o600))?;
    Ok((listener, pending))
}
fn publish_listener(pending: &PendingSocket, socket: &Path) -> Result<()> {
    match socket.symlink_metadata() {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(error.into()),
        Ok(_) => {
            return Err(crate::Error::Invalid(
                "socket path appeared before publication".into(),
            ));
        }
    }
    // The state lock is held and its directory is private. Publish only the
    // already-private socket, so clients never observe bind-before-chmod.
    std::fs::rename(&pending.path, socket)?;
    Ok(())
}
pub fn serve(state: &Path) -> Result<()> {
    let guard = lock(state)?;
    // All database and journal files inherit private permissions, including on restart.
    let db = state.join("state.db");
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&db)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    let runtime = Arc::new(Runtime::new(Store::open(&db)?)?);
    daemon_build();
    let (listener, pending) = prepare_listener(&guard.socket)?;
    publish_listener(&pending, &guard.socket)?;
    eprintln!("Architect daemon ready at {:?}", guard.socket);
    for stream in listener.incoming() {
        let mut stream = stream?;
        let runtime = runtime.clone();
        let socket = guard.socket.clone();
        std::thread::spawn(move || {
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
            let response = match read_request(&stream) {
                Ok(request) => handle(&runtime, request),
                Err(error) => Response {
                    version: PROTOCOL_VERSION,
                    request_id: String::new(),
                    data: None,
                    error: Some(error.to_string()),
                },
            };
            let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(5)));
            if let Ok(mut bytes) = serde_json::to_vec(&response) {
                if bytes.len() + 1 > crate::transport::MAX_FRAME {
                    bytes = serde_json::to_vec(&Response {
                        version: PROTOCOL_VERSION,
                        request_id: response.request_id.chars().take(256).collect(),
                        data: None,
                        error: Some(
                            "response exceeds frame limit; use paginated document reads".into(),
                        ),
                    })
                    .expect("serializable error");
                }
                bytes.push(b'\n');
                let _ = stream.write_all(&bytes);
            }
            if SHUTDOWN.load(std::sync::atomic::Ordering::SeqCst) {
                // The flock is released by process exit; the store lock is held.
                let _ = std::fs::remove_file(&socket);
                std::process::exit(0);
            }
        });
    }
    Ok(())
}
fn read_request(stream: &UnixStream) -> Result<Request> {
    crate::transport::read(&mut BufReader::new(stream))?
        .ok_or_else(|| crate::Error::Invalid("empty request".into()))
}
pub fn handle(runtime: &Arc<Runtime>, request: Request) -> Response {
    let result = (|| -> Result<Value> {
        require(
            request.version == PROTOCOL_VERSION,
            "unsupported IPC version",
        )?;
        require(
            !request.request_id.trim().is_empty() && request.request_id.len() <= 256,
            "request ID must contain 1 to 256 bytes",
        )?;
        match request.operation {
            Operation::Mutate { command } => {
                let key = request
                    .idempotency_key
                    .ok_or_else(|| crate::Error::Invalid("idempotency key required".into()))?;
                require(!key.trim().is_empty(), "idempotency key required")?;
                if let Command::AcceptTeam { run_id, .. } = command.as_ref() {
                    let (run, document_hash) = {
                        let store = runtime.store.lock().expect("store mutex");
                        if let Some(run) =
                            store.cached(&key, &serde_json::to_vec(command.as_ref())?)?
                        {
                            return Ok(summary(&run));
                        }
                        (store.get(run_id)?, store.run_chunk(run_id, 0, 1)?.0)
                    };
                    let view = runtime
                        .management_projection
                        .view(run, document_hash, true)?;
                    if !view.document().is_some_and(|d| d.prepared_team.is_some()) {
                        let store = runtime.store.lock().expect("store mutex");
                        if let Some(run) =
                            store.cached(&key, &serde_json::to_vec(command.as_ref())?)?
                        {
                            return Ok(summary(&run));
                        }
                        return Err(crate::Error::Invalid("team acceptance is being authenticated; refresh its preparation state before approving; no mutation admitted".into()));
                    }
                }
                if let Command::RecoverOrchestration { run_id, .. } = command.as_ref() {
                    let (run, document_hash) = {
                        let store = runtime.store.lock().expect("store mutex");
                        if let Some(run) =
                            store.cached(&key, &serde_json::to_vec(command.as_ref())?)?
                        {
                            return Ok(summary(&run));
                        }
                        (store.get(run_id)?, store.run_chunk(run_id, 0, 1)?.0)
                    };
                    let view = runtime
                        .management_projection
                        .view(run, document_hash, true)?;
                    if !view.document().is_some_and(|d| d.count > 0) {
                        let store = runtime.store.lock().expect("store mutex");
                        if let Some(run) =
                            store.cached(&key, &serde_json::to_vec(command.as_ref())?)?
                        {
                            return Ok(summary(&run));
                        }
                        return Err(crate::Error::Invalid("management recovery is not ready; refresh its preparation state before approving; no mutation admitted".into()));
                    }
                }
                Ok(summary(&runtime.dispatch_ipc(&key, *command)?))
            }
            Operation::Snapshot { run_id } => {
                let (run, document_hash, data_directory, sequence) = {
                    let store = runtime.store.lock().expect("store mutex");
                    let (run, document_hash) = store.snapshot(&run_id)?;
                    (
                        run,
                        document_hash,
                        store.data_directory()?,
                        store.sequence()?,
                    )
                };
                let management =
                    runtime
                        .management_projection
                        .view(run.clone(), document_hash.clone(), true)?;
                let tasks =
                    runtime
                        .task_projection
                        .view(run.clone(), document_hash.clone(), true)?;
                let reviewed = run.orchestration_rounds.last().filter(|round| {
                    round.status == crate::orchestration::planning::RoundStatus::Reviewed
                        && round
                            .output
                            .as_ref()
                            .and_then(|raw| {
                                serde_json::from_str::<crate::orchestration::planning::Response>(
                                    raw,
                                )
                                .ok()
                            })
                            .is_some_and(|r| r.team_plan_json.is_some())
                });
                let intake = crate::runtime::intake::pending(&run);
                let prepare_workspace = crate::workspace::managed::pending(&run, &data_directory)
                    .ok()
                    .map(|p| p.scope_hash.clone());
                Ok(
                    json!({"reviewed_intake_proposal":intake,"document_hash":document_hash,"document_hash_verified":true,"scope_hashes":{"start":run.scope_hash("start"),"accept":run.scope_hash("accept"),"repair":run.scope_hash("repair"),"reconcile":run.scope_hash("reconcile"),
                        "orchestrate":crate::orchestration::planning::start_scope(&run)?,"accept_team":reviewed.map(|r|crate::orchestration::planning::accept_scope(&run,r)).transpose()?,
                        "review_diff":crate::runtime::diff_review::scope(&run).unwrap_or_default(),
                        "step_team":run.team_step_scope().unwrap_or_default()},
                    "team_stage_results":stage_result_summary(&crate::runtime::team::stage::pending(&run)),
                    "task_result_restore_count":tasks.document().map_or(0, |d|d.count("result_restores")),
                    "workspace_preparation_scope":prepare_workspace,
                    "workspace_activation_scope":crate::workspace::managed::activation_scope(&run).ok(),
                    "source_delivery_scope":crate::runtime::delivery::pending(&run).ok().map(|p|&p.scope_hash),
                    "source_delivery_inspection_available":crate::runtime::delivery::eligible(&run).is_ok(),
                    "source_delivery_recovery":crate::runtime::delivery::recovery(&run).ok(),
                    "task_effect_reconciliation_count":tasks.document().map_or(0, |d|d.count("effect_reconciliations")),
                    "wave_effect_reconciliation_count":tasks.document().map_or(0, |d|d.count("wave_effect_reconciliations")),
                    "task_source_refreshes":tasks.document().map(|d|&d.packages["source_refreshes"]).cloned().unwrap_or_else(||json!({})),
                    "task_verification_retries":tasks.document().map(|d|&d.packages["verification_retries"]).cloned().unwrap_or_else(||json!({})),
                    "task_recovery_state":tasks.state(),
                    "task_recovery_document_hash":tasks.document().map(|d|&d.hash),
                    "task_session_resume_count":tasks.document().map_or(0, |d|d.count("native_resumes")),
                    "task_session_replacement_count":tasks.document().map_or(0, |d|d.count("replacements")),
                    "task_check_restore_count":tasks.document().map_or(0, |d|d.count("check_restores")),
                    "orchestration_recovery_count":management.document().map_or(0, |d|d.count),
                    "orchestration_recovery_state":management.state(),
                    "orchestration_recovery_document_hash":management.document().map(|d|&d.hash),
                    "team_acceptance_ready":management.document().is_some_and(|d|d.prepared_team.is_some()),
                    "reviewed_team_round":reviewed.map(|r|&r.id),
                    "run":summary(&run),"recovery":recovery_snapshot(&run),"sequence":sequence}),
                )
            }
            Operation::List => list_page(runtime, ""),
            Operation::ListPage { after } => list_page(runtime, &after),
            Operation::Identity => Ok(json!({
                "build": daemon_build(),
                "pid": std::process::id(),
                "busy": runtime.busy(),
            })),
            Operation::Shutdown => {
                let store = runtime.store.lock().expect("store mutex");
                require(
                    !runtime.busy(),
                    "daemon has active work; restart it after the work finishes",
                )?;
                // Keep the store locked until exit so no mutation starts meanwhile.
                std::mem::forget(store);
                SHUTDOWN.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(json!({"stopping": true}))
            }
            Operation::Read {
                run_id,
                document,
                offset,
                expected_hash,
            } => {
                let store = runtime.store.lock().expect("store mutex");
                if document == "run" {
                    let (digest, total, chunk) = store.run_chunk_bound(
                        &run_id,
                        offset,
                        DOCUMENT_CHUNK,
                        expected_hash.as_deref(),
                    )?;
                    require(
                        expected_hash.as_ref().is_none_or(|h| h == &digest),
                        "document changed; restart from offset zero",
                    )?;
                    require(
                        offset == 0 || expected_hash.is_some(),
                        "continuation requires document hash",
                    )?;
                    let end = offset + chunk.len();
                    return Ok(json!({"hash":digest,"total_bytes":total,"offset":offset,
                        "next_offset":if end < total { Some(end) } else { None },"chunk":chunk}));
                }
                let run = store.get(&run_id)?;
                let (run_document_hash, _, _) = store.run_chunk(&run_id, 0, 1)?;
                drop(store);
                if document == "orchestration-recovery" {
                    let view =
                        runtime
                            .management_projection
                            .view(run, run_document_hash, offset == 0)?;
                    let document = view.document().ok_or_else(|| {
                        crate::Error::Invalid(
                            "management recovery is being checked; refresh the snapshot".into(),
                        )
                    })?;
                    return document_page(
                        &document.text,
                        &document.hash,
                        offset,
                        expected_hash.as_deref(),
                    );
                }
                if document == "task-recovery" {
                    let view = runtime
                        .task_projection
                        .view(run, run_document_hash, offset == 0)?;
                    let document = view.document().ok_or_else(|| {
                        crate::Error::Invalid(
                            "task recovery is being checked; refresh the snapshot".into(),
                        )
                    })?;
                    return document_page(
                        &document.text,
                        &document.hash,
                        offset,
                        expected_hash.as_deref(),
                    );
                }
                let text = if document == "stage-results" {
                    serde_json::to_string(
                        &json!({"run_document_hash":run_document_hash,"packages":crate::runtime::team::stage::pending(&run)}),
                    )?
                } else {
                    document_text(&run, &document)?
                };
                let digest = crate::hash(&text);
                document_page(&text, &digest, offset, expected_hash.as_deref())
            }
            Operation::Events { after } => {
                let store = runtime.store.lock().expect("store mutex");
                let events = store.events(after)?;
                let cursor = events.last().map_or(after, |e| e.seq);
                Ok(json!({"events":events,"next_cursor":cursor}))
            }
        }
    })();
    match result {
        Ok(data) => Response {
            version: PROTOCOL_VERSION,
            request_id: request.request_id,
            data: Some(data),
            error: None,
        },
        Err(error) => Response {
            version: PROTOCOL_VERSION,
            request_id: request.request_id,
            data: None,
            error: Some(error.to_string()),
        },
    }
}
fn document_page(
    text: &str,
    digest: &str,
    offset: usize,
    expected_hash: Option<&str>,
) -> Result<Value> {
    require(
        expected_hash.is_none_or(|h| h == digest),
        "document changed; restart from offset zero",
    )?;
    require(
        offset == 0 || expected_hash.is_some(),
        "continuation requires document hash",
    )?;
    require(
        offset <= text.len() && text.is_char_boundary(offset),
        "invalid document offset",
    )?;
    let mut end = offset.saturating_add(DOCUMENT_CHUNK).min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    Ok(
        json!({"hash":digest,"total_bytes":text.len(),"offset":offset,
        "next_offset":if end < text.len() { Some(end) } else { None },"chunk":&text[offset..end]}),
    )
}
pub fn call(state: &Path, request: &Request) -> Result<Response> {
    private_directory(state)?;
    let path = state.join("architect.sock");
    let metadata = std::fs::symlink_metadata(&path)?;
    require(
        metadata.file_type().is_socket() && metadata.uid() == uid() && metadata.mode() & 0o077 == 0,
        "unsafe IPC socket",
    )?;
    let mut stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(std::time::Duration::from_secs(5)))?;
    let mut bytes = serde_json::to_vec(request)?;
    require(
        bytes.len() < crate::transport::MAX_FRAME,
        "request exceeds transport limit",
    )?;
    bytes.push(b'\n');
    stream.write_all(&bytes)?;
    let response: Response = crate::transport::read(&mut BufReader::new(stream))?
        .ok_or_else(|| crate::Error::Invalid("daemon disconnected".into()))?;
    require(
        response.version == PROTOCOL_VERSION && response.request_id == request.request_id,
        "mismatched IPC response",
    )?;
    Ok(response)
}

/// No source, artifact, evidence or report bodies are copied into control replies.
fn summary(run: &crate::domain::Run) -> Value {
    json!({"id":run.id,"spec_revision":run.spec_revision,"stage":run.stage,
        "status":run.status,"paused_from":run.paused_from,"attempt":run.attempt,
        "active_session":run.active_session,"reconciliation_required":run.reconciliation_required,
        "diagnostic":run.diagnostic.as_ref().map(|s| s.chars().take(4096).collect::<String>()),
        "spec_hash":crate::hash(serde_json::to_vec(&run.spec).expect("spec serialization")),
        "input_hash":crate::hash(serde_json::to_vec(&run.input_snapshot).expect("input serialization")),
        "counts":{"artifacts":run.artifacts.len(),"reviews":run.reviews.len(),
            "evidence":run.evidence.len(),"approvals":run.approvals.len(),
            "messages":run.communication_ledger.messages.len(),
            "orchestration_rounds":run.orchestration_rounds.len(),
            "requirements":run.spec.requirements.len(),"input_files":run.input_snapshot.len(),
            "diff_comments":run.diff_comments.len()},
        "latest_artifact":run.artifacts.last().map(|a| json!({"id":a.id,"hash":a.hash,"document":format!("artifact:{}",a.id)})),
        "document":"run"})
}
fn recovery_snapshot(run: &crate::domain::Run) -> Value {
    if !run.reconciliation_required {
        return Value::Null;
    }
    match crate::recovery::observe(run) {
        Ok(observed) => {
            let unsettled_container = run.check_resources.records().any(|r| r.unresolved());
            let unsettled_delivery = run
                .delivery
                .as_ref()
                .is_some_and(|d| d.state == crate::runtime::delivery::State::Unknown);
            json!({"observed_input_hash":observed.hash,"inputs_readable":observed.readable.is_some(),"pending_edits":run.pending_edits.len(),
                "inputs":observed.inputs,"reconciliation_available":!unsettled_container && !unsettled_delivery,
                "unsettled_check_container":unsettled_container,"unsettled_source_delivery":unsettled_delivery})
        }
        Err(error) => json!({"error":error.to_string()}),
    }
}
fn list_page(runtime: &Arc<Runtime>, after: &str) -> Result<Value> {
    let store = runtime.store.lock().expect("store mutex");
    let mut runs = store.page(after, 101)?;
    let more = runs.len() > 100;
    runs.truncate(100);
    let cursor = if more {
        runs.last().map(|r| &r.id)
    } else {
        None
    };
    Ok(
        json!({"runs":runs.iter().map(summary).collect::<Vec<_>>(),"next_cursor":cursor,"sequence":store.sequence()?}),
    )
}
fn stage_result_summary(
    packages: &std::collections::BTreeMap<String, crate::runtime::team::stage::PendingResult>,
) -> serde_json::Value {
    json!(packages.iter().map(|(key, package)| (key.clone(), json!({"stage":package.stage,"task_count":package.task_ids.len(),"scope_hash":package.scope_hash}))).collect::<std::collections::BTreeMap<_,_>>())
}

fn document_text(run: &crate::domain::Run, name: &str) -> Result<String> {
    if name == "run" {
        return Ok(serde_json::to_string(run)?);
    }
    if name == "spec" {
        return Ok(serde_json::to_string(&run.spec)?);
    }
    if name == "diff" {
        let diff = run.current_diff()?;
        return Ok(serde_json::to_string(&diff)?);
    }
    if let Some(id) = name.strip_prefix("message:") {
        let message = run
            .communication_ledger
            .messages
            .get(id)
            .ok_or_else(|| crate::Error::Invalid("unknown message document".into()))?;
        let body = run
            .message_bodies
            .get(id)
            .ok_or_else(|| crate::Error::Invalid("message body is missing".into()))?;
        require(
            message.content_ref.document == name && message.content_ref.hash == crate::hash(body),
            "message body does not match its durable reference",
        )?;
        return Ok(body.clone());
    }
    if let Some(id) = name.strip_prefix("artifact:") {
        return run
            .artifacts
            .iter()
            .find(|a| a.id == id)
            .map(|a| a.content.clone())
            .ok_or_else(|| crate::Error::Invalid("unknown artifact document".into()));
    }
    if let Some(id) = name.strip_prefix("evidence:") {
        return run
            .evidence
            .iter()
            .find(|e| e.id == id)
            .map(|e| e.output.clone())
            .ok_or_else(|| crate::Error::Invalid("unknown evidence document".into()));
    }
    Err(crate::Error::Invalid("unknown document".into()))
}
/// Consistent document download. A concurrent mutation fails explicitly rather
/// than combining chunks from different snapshots.
pub fn read_document(state: &Path, run_id: &str, document: &str) -> Result<String> {
    let mut offset = 0;
    let mut expected_hash = None;
    let mut output = String::new();
    loop {
        let response = call(
            state,
            &Request {
                version: PROTOCOL_VERSION,
                request_id: crate::id(),
                idempotency_key: None,
                operation: Operation::Read {
                    run_id: run_id.into(),
                    document: document.into(),
                    offset,
                    expected_hash: expected_hash.clone(),
                },
            },
        )?;
        if let Some(error) = response.error {
            return Err(crate::Error::Invalid(error));
        }
        let data = response
            .data
            .ok_or_else(|| crate::Error::Invalid("missing document chunk".into()))?;
        let chunk = data["chunk"]
            .as_str()
            .ok_or_else(|| crate::Error::Invalid("missing document bytes".into()))?;
        expected_hash = Some(
            data["hash"]
                .as_str()
                .ok_or_else(|| crate::Error::Invalid("missing document hash".into()))?
                .to_owned(),
        );
        output.push_str(chunk);
        if data["next_offset"].is_null() {
            break;
        }
        offset = usize::try_from(
            data["next_offset"]
                .as_u64()
                .ok_or_else(|| crate::Error::Invalid("invalid chunk cursor".into()))?,
        )
        .map_err(|_| crate::Error::Invalid("invalid chunk cursor".into()))?;
    }
    require(
        expected_hash.as_ref() == Some(&crate::hash(&output)),
        "document hash mismatch",
    )?;
    Ok(output)
}

#[cfg(test)]
mod stage_result_tests {
    use super::*;
    use crate::runtime::team::stage::{PendingResult, ResultBinding};
    use std::collections::BTreeMap;
    #[test]
    fn stage_result_control_summary_excludes_unbounded_result_and_dependency_bodies() {
        let dependencies = (0..1000)
            .map(|i| (format!("task-{i}"), "a".repeat(64)))
            .collect::<BTreeMap<_, _>>();
        let results = (0..100)
            .map(|i| ResultBinding {
                task_id: format!("task-{i}"),
                task_attempt: 1,
                task_hash: "b".repeat(64),
                artifact_id: format!("artifact-{i}"),
                artifact_hash: "c".repeat(64),
                review_hash: "d".repeat(64),
                dependency_hashes: dependencies.clone(),
            })
            .collect::<Vec<_>>();
        let package = PendingResult {
            stage: crate::team::TaskStage::Integration,
            task_ids: results.iter().map(|r| r.task_id.clone()).collect(),
            results,
            scope_hash: "e".repeat(64),
        };
        let packages = BTreeMap::from([("integration".into(), package)]);
        assert!(serde_json::to_vec(&packages).unwrap().len() > crate::transport::MAX_FRAME);
        let summary = stage_result_summary(&packages);
        assert!(serde_json::to_vec(&summary).unwrap().len() < 1024);
        assert_eq!(summary["integration"]["task_count"], 100);
        assert_eq!(
            summary["integration"]["scope_hash"],
            packages["integration"].scope_hash
        );
        assert!(summary["integration"].get("results").is_none());
        assert!(summary["integration"].get("task_ids").is_none());
    }
}

#[cfg(test)]
mod socket_publication_tests {
    use super::*;
    #[test]
    fn socket_publication_is_private_before_visibility_and_keeps_listener_reachable() {
        let state = tempfile::tempdir().unwrap();
        std::fs::set_permissions(state.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        private_directory(state.path()).unwrap();
        let socket = state.path().join("architect.sock");
        let (listener, pending) = prepare_listener(&socket).unwrap();
        assert!(!socket.exists());
        assert_eq!(
            pending.path.symlink_metadata().unwrap().mode() & 0o777,
            0o600
        );
        publish_listener(&pending, &socket).unwrap();
        assert_eq!(socket.symlink_metadata().unwrap().mode() & 0o777, 0o600);
        drop(pending);
        let _client = UnixStream::connect(&socket).unwrap();
        let _accepted = listener.accept().unwrap();
        assert_eq!(std::fs::read_dir(state.path()).unwrap().count(), 1);
    }
    #[test]
    fn socket_publication_refuses_an_unexpected_path_and_cleans_only_owned_pending_socket() {
        let state = tempfile::tempdir().unwrap();
        std::fs::set_permissions(state.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let socket = state.path().join("architect.sock");
        let (_listener, pending) = prepare_listener(&socket).unwrap();
        let pending_path = pending.path.clone();
        std::fs::write(&socket, "keep user file").unwrap();
        assert!(publish_listener(&pending, &socket).is_err());
        drop(pending);
        assert!(!pending_path.exists());
        assert_eq!(std::fs::read_to_string(socket).unwrap(), "keep user file");
    }
}

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
    Mutate { command: Box<Command> },
    Snapshot { run_id: String },
    List,
    ListPage { after: String },
    Read {
        run_id: String,
        document: String,
        offset: usize,
        expected_hash: Option<String>,
    },
    Events { after: u64 },
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub version: u32,
    pub request_id: String,
    pub data: Option<Value>,
    pub error: Option<String>,
}
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
    let listener = UnixListener::bind(&guard.socket)?;
    std::fs::set_permissions(&guard.socket, std::fs::Permissions::from_mode(0o600))?;
    eprintln!("Architect daemon ready at {:?}", guard.socket);
    for stream in listener.incoming() {
        let mut stream = stream?;
        let runtime = runtime.clone();
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
                        version: PROTOCOL_VERSION, request_id: response.request_id.chars().take(256).collect(),
                        data: None, error: Some("response exceeds frame limit; use paginated document reads".into()),
                    }).expect("serializable error");
                }
                bytes.push(b'\n');
                let _ = stream.write_all(&bytes);
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
        require(!request.request_id.trim().is_empty() && request.request_id.len() <= 256, "request ID must contain 1 to 256 bytes")?;
        match request.operation {
            Operation::Mutate { command } => {
                let key = request
                    .idempotency_key
                    .ok_or_else(|| crate::Error::Invalid("idempotency key required".into()))?;
                Ok(summary(&runtime.dispatch(&key, *command)?))
            }
            Operation::Snapshot { run_id } => {
                let store = runtime.store.lock().expect("store mutex");
                let run = store.get(&run_id)?;
                Ok(
                    json!({"scope_hashes":{"start":run.scope_hash("start"),"accept":run.scope_hash("accept"),"repair":run.scope_hash("repair"),"reconcile":run.scope_hash("reconcile")},
                    "run":summary(&run),"recovery":recovery_snapshot(&run),"sequence":store.sequence()?}),
                )
            }
            Operation::List => list_page(runtime, ""),
            Operation::ListPage { after } => list_page(runtime, &after),
            Operation::Read { run_id, document, offset, expected_hash } => {
                let store = runtime.store.lock().expect("store mutex");
                if document == "run" {
                    let (digest, total, chunk) = store.run_chunk(&run_id, offset, DOCUMENT_CHUNK)?;
                    require(expected_hash.as_ref().is_none_or(|h| h == &digest), "document changed; restart from offset zero")?;
                    require(offset == 0 || expected_hash.is_some(), "continuation requires document hash")?;
                    let end = offset + chunk.len();
                    return Ok(json!({"hash":digest,"total_bytes":total,"offset":offset,
                        "next_offset":if end < total { Some(end) } else { None },"chunk":chunk}));
                }
                let run = store.get(&run_id)?;
                let text = document_text(&run, &document)?;
                let digest = crate::hash(&text);
                require(expected_hash.as_ref().is_none_or(|h| h == &digest), "document changed; restart from offset zero")?;
                require(offset == 0 || expected_hash.is_some(), "continuation requires document hash")?;
                require(offset <= text.len() && text.is_char_boundary(offset), "invalid document offset")?;
                let mut end = offset.saturating_add(DOCUMENT_CHUNK).min(text.len());
                while !text.is_char_boundary(end) { end -= 1; }
                Ok(json!({"hash":digest,"total_bytes":text.len(),"offset":offset,
                    "next_offset":if end < text.len() { Some(end) } else { None },"chunk":&text[offset..end]}))
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
    require(bytes.len() < crate::transport::MAX_FRAME, "request exceeds transport limit")?;
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
            "requirements":run.spec.requirements.len(),"input_files":run.input_snapshot.len()},
        "latest_artifact":run.artifacts.last().map(|a| json!({"id":a.id,"hash":a.hash,"document":format!("artifact:{}",a.id)})),
        "document":"run"})
}
fn recovery_snapshot(run: &crate::domain::Run) -> Value {
    if !run.reconciliation_required { return Value::Null; }
    match crate::recovery::observe(run) {
        Ok(observed) => json!({"observed_input_hash":observed.hash,"inputs_readable":observed.readable.is_some(),"pending_edits":run.pending_edits.len()}),
        Err(error) => json!({"error":error.to_string()}),
    }
}
fn list_page(runtime: &Arc<Runtime>, after: &str) -> Result<Value> {
    let store = runtime.store.lock().expect("store mutex");
    let mut runs = store.page(after, 101)?;
    let more = runs.len() > 100;
    runs.truncate(100);
    let cursor = if more { runs.last().map(|r| &r.id) } else { None };
    Ok(json!({"runs":runs.iter().map(summary).collect::<Vec<_>>(),"next_cursor":cursor,"sequence":store.sequence()?}))
}
fn document_text(run: &crate::domain::Run, name: &str) -> Result<String> {
    if name == "run" { return Ok(serde_json::to_string(run)?); }
    if name == "spec" { return Ok(serde_json::to_string(&run.spec)?); }
    if let Some(id) = name.strip_prefix("artifact:") {
        return run.artifacts.iter().find(|a| a.id == id).map(|a| a.content.clone())
            .ok_or_else(|| crate::Error::Invalid("unknown artifact document".into()));
    }
    if let Some(id) = name.strip_prefix("evidence:") {
        return run.evidence.iter().find(|e| e.id == id).map(|e| e.output.clone())
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
        let response = call(state, &Request { version: PROTOCOL_VERSION, request_id:crate::id(),
            idempotency_key:None, operation:Operation::Read { run_id:run_id.into(), document:document.into(), offset, expected_hash:expected_hash.clone() } })?;
        if let Some(error) = response.error { return Err(crate::Error::Invalid(error)); }
        let data = response.data.ok_or_else(|| crate::Error::Invalid("missing document chunk".into()))?;
        let chunk = data["chunk"].as_str().ok_or_else(|| crate::Error::Invalid("missing document bytes".into()))?;
        expected_hash = Some(data["hash"].as_str().ok_or_else(|| crate::Error::Invalid("missing document hash".into()))?.to_owned());
        output.push_str(chunk);
        if data["next_offset"].is_null() { break; }
        offset = usize::try_from(data["next_offset"].as_u64().ok_or_else(|| crate::Error::Invalid("invalid chunk cursor".into()))?)
            .map_err(|_| crate::Error::Invalid("invalid chunk cursor".into()))?;
    }
    require(expected_hash.as_ref() == Some(&crate::hash(&output)), "document hash mismatch")?;
    Ok(output)
}

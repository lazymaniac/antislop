//! The daemon owns a project's `Core`, so work goes on after a frontend
//! detaches. One request per connection, one JSON line each way. Every reply
//! is small; `events` is a long poll that returns as soon as something new is
//! recorded.

use super::Core;
use crate::{Error, Result, config::Config, require};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

/// Creates `path` if needed and insists it is a real directory private to
/// the current user.
pub fn private_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    if !path.exists() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)?;
    }
    let metadata = std::fs::symlink_metadata(path)?;
    // SAFETY: geteuid has no arguments or failure modes.
    let uid = unsafe { libc::geteuid() };
    require(
        metadata.is_dir()
            && !metadata.file_type().is_symlink()
            && metadata.uid() == uid
            && metadata.permissions().mode() & 0o077 == 0,
        &format!(
            "{} must be a directory owned by the current user with mode 0700",
            path.display()
        ),
    )
}

/// Identifies a build of the executable: path, size and modification time.
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

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    /// Which binary the daemon runs and whether it is executing work.
    Identity,
    GetConfig,
    SetConfig {
        toml: String,
        expected_revision: String,
    },
    /// Stops an idle daemon so a frontend from another build can replace it.
    Shutdown,
    ForceShutdown,
    Conversations,
    Start,
    StartIn {
        project: String,
    },
    Send {
        conversation: String,
        text: String,
    },
    SendAddressed {
        conversation: String,
        text: String,
        to: Option<String>,
    },
    /// Starts exactly the plan the user saw.
    Accept {
        conversation: String,
        plan: String,
    },
    AcceptScoped {
        conversation: String,
        plan: String,
        scope_hash: Option<String>,
    },
    /// Rejects a proposed plan and asks the orchestrator to revise it.
    Reject {
        conversation: String,
        plan: String,
        reason: Option<String>,
    },
    RejectScoped {
        conversation: String,
        plan: String,
        reason: Option<String>,
        scope_hash: Option<String>,
    },
    /// Runs again what the plan left unfinished, through the same sessions.
    Resume {
        conversation: String,
        plan: String,
    },
    ResumeScoped {
        conversation: String,
        plan: String,
        scope_hash: Option<String>,
    },
    Cancel {
        conversation: String,
    },
    CancelScoped {
        conversation: String,
        scope_hash: Option<String>,
    },
    /// Stops starting new plan work, or continues it.
    Pause {
        conversation: String,
        paused: bool,
    },
    /// Runs the conversation on by itself, or under the user, from now on.
    Mode {
        conversation: String,
        autonomous: bool,
    },
    /// Unblocks a task that stopped: see [`super::RecoveryAction`].
    Recover(super::TaskRecoveryRequest),
    /// The user finished commenting on the plan: the orchestrator answers
    /// the draft comments together.
    ReviewComments {
        conversation: String,
        plan: String,
    },
    /// Paginated read of all stored plans for a conversation.
    Plans {
        conversation: String,
        offset: Option<usize>,
        limit: Option<usize>,
    },
    /// Detailed history of a single plan.
    Plan {
        conversation: String,
        plan: String,
    },
    /// Comments of a conversation, optionally filtered by plan and/or status.
    Comments {
        conversation: String,
        plan: Option<String>,
        status: Option<String>,
    },
    /// The user's comment on one element of the team panel or a diff.
    Comment {
        conversation: String,
        target: String,
        text: String,
    },
    /// The user's comment scoped to a specific plan.
    CommentScoped {
        conversation: String,
        plan: Option<String>,
        target: String,
        text: String,
    },
    /// Resolves, reopens (`open`) or deletes a comment.
    CommentStatus {
        conversation: String,
        id: u64,
        status: String,
    },
    /// A writer's change as files and numbered lines.
    Diff {
        conversation: String,
        plan: String,
        id: String,
    },
    /// Reviews a writer's change now; findings go to the orchestrator.
    Review {
        conversation: String,
        plan: String,
        id: String,
    },
    ReviewScoped {
        conversation: String,
        plan: String,
        id: String,
        scope_hash: Option<String>,
    },
    /// One task of a plan with its latest report and verdict.
    Task {
        plan: String,
        id: String,
    },
    /// Inspector snapshot and checks without a Git freshness fingerprint.
    TaskChecks {
        conversation: Option<String>,
        plan: String,
        id: String,
    },
    TaskScoped {
        conversation: Option<String>,
        plan: String,
        id: String,
    },
    Events {
        conversation: String,
        after: u64,
        wait_ms: u64,
    },
    EventsPaged {
        conversation: String,
        after: u64,
        wait_ms: u64,
        limit: Option<usize>,
    },
    Sessions {
        conversation: String,
    },
    History {
        conversation: String,
        after: u64,
        limit: Option<usize>,
    },
    Mail {
        conversation: String,
    },
    TrustStatus,
    Trust {
        trusted: bool,
    },
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum WireRequest {
    Identity,
    GetConfig,
    SetConfig {
        toml: String,
        expected_revision: String,
    },
    Shutdown,
    ForceShutdown,
    Conversations,
    Start {
        #[serde(default)]
        project: Option<String>,
    },
    Send {
        conversation: String,
        text: String,
        #[serde(default)]
        to: Option<String>,
    },
    Accept {
        conversation: String,
        plan: String,
        #[serde(default)]
        scope_hash: Option<String>,
    },
    Reject {
        conversation: String,
        plan: String,
        #[serde(default)]
        reason: Option<String>,
        #[serde(default)]
        scope_hash: Option<String>,
    },
    Resume {
        conversation: String,
        plan: String,
        #[serde(default)]
        scope_hash: Option<String>,
    },
    ReviewComments {
        conversation: String,
        plan: String,
    },
    Recover {
        conversation: String,
        plan: String,
        task: String,
        action: super::RecoveryAction,
        #[serde(default)]
        consent_files: Option<Vec<String>>,
        freshness_token: String,
        #[serde(default)]
        message: Option<String>,
    },
    Cancel {
        conversation: String,
        #[serde(default)]
        scope_hash: Option<String>,
    },
    Pause {
        conversation: String,
        paused: bool,
    },
    Mode {
        conversation: String,
        autonomous: bool,
    },
    Plans {
        conversation: String,
        #[serde(default)]
        offset: Option<usize>,
        #[serde(default)]
        limit: Option<usize>,
    },
    Plan {
        conversation: String,
        plan: String,
    },
    Comments {
        conversation: String,
        #[serde(default)]
        plan: Option<String>,
        #[serde(default)]
        status: Option<String>,
    },
    Comment {
        conversation: String,
        #[serde(default)]
        plan: Option<String>,
        target: String,
        text: String,
    },
    CommentStatus {
        conversation: String,
        id: u64,
        status: String,
    },
    Diff {
        conversation: String,
        plan: String,
        id: String,
    },
    Review {
        conversation: String,
        plan: String,
        id: String,
        #[serde(default)]
        scope_hash: Option<String>,
    },
    TaskChecks {
        #[serde(default)]
        conversation: Option<String>,
        plan: String,
        id: String,
    },
    Task {
        #[serde(default)]
        conversation: Option<String>,
        plan: String,
        id: String,
    },
    Events {
        conversation: String,
        after: u64,
        wait_ms: u64,
        #[serde(default)]
        limit: Option<usize>,
    },
    Sessions {
        conversation: String,
    },
    History {
        conversation: String,
        #[serde(default)]
        after: u64,
        #[serde(default)]
        limit: Option<usize>,
    },
    Mail {
        conversation: String,
    },
    TrustStatus,
    Trust {
        #[serde(default = "default_true")]
        trusted: bool,
    },
}

impl From<Request> for WireRequest {
    fn from(req: Request) -> Self {
        match req {
            Request::Identity => WireRequest::Identity,
            Request::GetConfig => WireRequest::GetConfig,
            Request::SetConfig {
                toml,
                expected_revision,
            } => WireRequest::SetConfig {
                toml,
                expected_revision,
            },
            Request::Shutdown => WireRequest::Shutdown,
            Request::ForceShutdown => WireRequest::ForceShutdown,
            Request::Conversations => WireRequest::Conversations,
            Request::Start => WireRequest::Start { project: None },
            Request::StartIn { project } => WireRequest::Start {
                project: Some(project),
            },
            Request::Send { conversation, text } => WireRequest::Send {
                conversation,
                text,
                to: None,
            },
            Request::SendAddressed {
                conversation,
                text,
                to,
            } => WireRequest::Send {
                conversation,
                text,
                to,
            },
            Request::Accept { conversation, plan } => WireRequest::Accept {
                conversation,
                plan,
                scope_hash: None,
            },
            Request::AcceptScoped {
                conversation,
                plan,
                scope_hash,
            } => WireRequest::Accept {
                conversation,
                plan,
                scope_hash,
            },
            Request::Reject {
                conversation,
                plan,
                reason,
            } => WireRequest::Reject {
                conversation,
                plan,
                reason,
                scope_hash: None,
            },
            Request::RejectScoped {
                conversation,
                plan,
                reason,
                scope_hash,
            } => WireRequest::Reject {
                conversation,
                plan,
                reason,
                scope_hash,
            },
            Request::Resume { conversation, plan } => WireRequest::Resume {
                conversation,
                plan,
                scope_hash: None,
            },
            Request::ResumeScoped {
                conversation,
                plan,
                scope_hash,
            } => WireRequest::Resume {
                conversation,
                plan,
                scope_hash,
            },
            Request::ReviewComments { conversation, plan } => {
                WireRequest::ReviewComments { conversation, plan }
            }
            Request::Recover(req) => WireRequest::Recover {
                conversation: req.conversation,
                plan: req.plan,
                task: req.task,
                action: req.action,
                consent_files: req.consent_files,
                freshness_token: req.freshness_token,
                message: req.message,
            },
            Request::Cancel { conversation } => WireRequest::Cancel {
                conversation,
                scope_hash: None,
            },
            Request::CancelScoped {
                conversation,
                scope_hash,
            } => WireRequest::Cancel {
                conversation,
                scope_hash,
            },
            Request::Pause {
                conversation,
                paused,
            } => WireRequest::Pause {
                conversation,
                paused,
            },
            Request::Mode {
                conversation,
                autonomous,
            } => WireRequest::Mode {
                conversation,
                autonomous,
            },
            Request::Plans {
                conversation,
                offset,
                limit,
            } => WireRequest::Plans {
                conversation,
                offset,
                limit,
            },
            Request::Plan { conversation, plan } => WireRequest::Plan { conversation, plan },
            Request::Comments {
                conversation,
                plan,
                status,
            } => WireRequest::Comments {
                conversation,
                plan,
                status,
            },
            Request::Comment {
                conversation,
                target,
                text,
            } => WireRequest::Comment {
                conversation,
                plan: None,
                target,
                text,
            },
            Request::CommentScoped {
                conversation,
                plan,
                target,
                text,
            } => WireRequest::Comment {
                conversation,
                plan,
                target,
                text,
            },
            Request::CommentStatus {
                conversation,
                id,
                status,
            } => WireRequest::CommentStatus {
                conversation,
                id,
                status,
            },
            Request::Diff {
                conversation,
                plan,
                id,
            } => WireRequest::Diff {
                conversation,
                plan,
                id,
            },
            Request::Review {
                conversation,
                plan,
                id,
            } => WireRequest::Review {
                conversation,
                plan,
                id,
                scope_hash: None,
            },
            Request::ReviewScoped {
                conversation,
                plan,
                id,
                scope_hash,
            } => WireRequest::Review {
                conversation,
                plan,
                id,
                scope_hash,
            },
            Request::TaskChecks {
                conversation,
                plan,
                id,
            } => WireRequest::TaskChecks {
                conversation,
                plan,
                id,
            },
            Request::Task { plan, id } => WireRequest::Task {
                conversation: None,
                plan,
                id,
            },
            Request::TaskScoped {
                conversation,
                plan,
                id,
            } => WireRequest::Task {
                conversation,
                plan,
                id,
            },
            Request::Events {
                conversation,
                after,
                wait_ms,
            } => WireRequest::Events {
                conversation,
                after,
                wait_ms,
                limit: None,
            },
            Request::EventsPaged {
                conversation,
                after,
                wait_ms,
                limit,
            } => WireRequest::Events {
                conversation,
                after,
                wait_ms,
                limit,
            },
            Request::Sessions { conversation } => WireRequest::Sessions { conversation },
            Request::History {
                conversation,
                after,
                limit,
            } => WireRequest::History {
                conversation,
                after,
                limit,
            },
            Request::Mail { conversation } => WireRequest::Mail { conversation },
            Request::TrustStatus => WireRequest::TrustStatus,
            Request::Trust { trusted } => WireRequest::Trust { trusted },
        }
    }
}

impl From<WireRequest> for Request {
    fn from(wire: WireRequest) -> Self {
        match wire {
            WireRequest::Identity => Request::Identity,
            WireRequest::GetConfig => Request::GetConfig,
            WireRequest::SetConfig {
                toml,
                expected_revision,
            } => Request::SetConfig {
                toml,
                expected_revision,
            },
            WireRequest::Shutdown => Request::Shutdown,
            WireRequest::ForceShutdown => Request::ForceShutdown,
            WireRequest::Conversations => Request::Conversations,
            WireRequest::Start { project } => match project {
                Some(project) => Request::StartIn { project },
                None => Request::Start,
            },
            WireRequest::Send {
                conversation,
                text,
                to,
            } => {
                if to.is_some() {
                    Request::SendAddressed {
                        conversation,
                        text,
                        to,
                    }
                } else {
                    Request::Send { conversation, text }
                }
            }
            WireRequest::Accept {
                conversation,
                plan,
                scope_hash,
            } => {
                if scope_hash.is_some() {
                    Request::AcceptScoped {
                        conversation,
                        plan,
                        scope_hash,
                    }
                } else {
                    Request::Accept { conversation, plan }
                }
            }
            WireRequest::Reject {
                conversation,
                plan,
                reason,
                scope_hash,
            } => {
                if scope_hash.is_some() {
                    Request::RejectScoped {
                        conversation,
                        plan,
                        reason,
                        scope_hash,
                    }
                } else {
                    Request::Reject {
                        conversation,
                        plan,
                        reason,
                    }
                }
            }
            WireRequest::Resume {
                conversation,
                plan,
                scope_hash,
            } => {
                if scope_hash.is_some() {
                    Request::ResumeScoped {
                        conversation,
                        plan,
                        scope_hash,
                    }
                } else {
                    Request::Resume { conversation, plan }
                }
            }
            WireRequest::ReviewComments { conversation, plan } => {
                Request::ReviewComments { conversation, plan }
            }
            WireRequest::Recover {
                conversation,
                plan,
                task,
                action,
                consent_files,
                freshness_token,
                message,
            } => Request::Recover(super::TaskRecoveryRequest {
                conversation,
                plan,
                task,
                action,
                consent_files,
                freshness_token,
                message,
            }),
            WireRequest::Cancel {
                conversation,
                scope_hash,
            } => {
                if scope_hash.is_some() {
                    Request::CancelScoped {
                        conversation,
                        scope_hash,
                    }
                } else {
                    Request::Cancel { conversation }
                }
            }
            WireRequest::Pause {
                conversation,
                paused,
            } => Request::Pause {
                conversation,
                paused,
            },
            WireRequest::Mode {
                conversation,
                autonomous,
            } => Request::Mode {
                conversation,
                autonomous,
            },
            WireRequest::Plans {
                conversation,
                offset,
                limit,
            } => Request::Plans {
                conversation,
                offset,
                limit,
            },
            WireRequest::Plan { conversation, plan } => Request::Plan { conversation, plan },
            WireRequest::Comments {
                conversation,
                plan,
                status,
            } => Request::Comments {
                conversation,
                plan,
                status,
            },
            WireRequest::Comment {
                conversation,
                plan,
                target,
                text,
            } => {
                if plan.is_some() {
                    Request::CommentScoped {
                        conversation,
                        plan,
                        target,
                        text,
                    }
                } else {
                    Request::Comment {
                        conversation,
                        target,
                        text,
                    }
                }
            }
            WireRequest::CommentStatus {
                conversation,
                id,
                status,
            } => Request::CommentStatus {
                conversation,
                id,
                status,
            },
            WireRequest::Diff {
                conversation,
                plan,
                id,
            } => Request::Diff {
                conversation,
                plan,
                id,
            },
            WireRequest::Review {
                conversation,
                plan,
                id,
                scope_hash,
            } => {
                if scope_hash.is_some() {
                    Request::ReviewScoped {
                        conversation,
                        plan,
                        id,
                        scope_hash,
                    }
                } else {
                    Request::Review {
                        conversation,
                        plan,
                        id,
                    }
                }
            }
            WireRequest::TaskChecks {
                conversation,
                plan,
                id,
            } => Request::TaskChecks {
                conversation,
                plan,
                id,
            },
            WireRequest::Task {
                conversation,
                plan,
                id,
            } => {
                if conversation.is_some() {
                    Request::TaskScoped {
                        conversation,
                        plan,
                        id,
                    }
                } else {
                    Request::Task { plan, id }
                }
            }
            WireRequest::Events {
                conversation,
                after,
                wait_ms,
                limit,
            } => {
                if limit.is_some() {
                    Request::EventsPaged {
                        conversation,
                        after,
                        wait_ms,
                        limit,
                    }
                } else {
                    Request::Events {
                        conversation,
                        after,
                        wait_ms,
                    }
                }
            }
            WireRequest::Sessions { conversation } => Request::Sessions { conversation },
            WireRequest::History {
                conversation,
                after,
                limit,
            } => Request::History {
                conversation,
                after,
                limit,
            },
            WireRequest::Mail { conversation } => Request::Mail { conversation },
            WireRequest::TrustStatus => Request::TrustStatus,
            WireRequest::Trust { trusted } => Request::Trust { trusted },
        }
    }
}

impl Serialize for Request {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        WireRequest::from(self.clone()).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Request {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        WireRequest::deserialize(deserializer).map(Request::from)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Response {
    data: Option<Value>,
    error: Option<String>,
}

/// Longest a single `events` call waits for something new.
const LONGEST_WAIT: Duration = Duration::from_secs(20);
/// Events per reply, and their combined size, so replies stay small.
const EVENTS_PER_REPLY: usize = 500;
const REPLY_BYTES: usize = 1024 * 1024;

/// How many of the (non-empty, ordered) events fit one reply: payload
/// bytes add up until `reply_bytes` is crossed, but the first event is
/// always taken whole — even one oversized historical event must not
/// stall the cursor, so a reply's size has no strict ceiling deducible
/// from `reply_bytes` alone; the cursor, not the size, bounds a reply.
/// Everything left over waits for the next poll, which the returned
/// `next_after`/`has_more` invite.
pub fn reply_event_count(events: &[super::store::Event], reply_bytes: usize) -> usize {
    let mut bytes = 0;
    let mut taken = 0;
    for event in events {
        bytes += event.payload.to_string().len();
        if taken > 0 && bytes > reply_bytes {
            break;
        }
        taken += 1;
    }
    taken
}

/// `<state>/core.sock`, or, when that path is too long for a Unix socket, a
/// short one named after the state directory in a private directory in /tmp.
pub fn socket(state: &Path) -> PathBuf {
    let socket = state.join("core.sock");
    if socket.as_os_str().len() < 100 {
        return socket;
    }
    // SAFETY: geteuid has no arguments or failure modes.
    let user = unsafe { libc::geteuid() };
    let hash = crate::hash(state.to_string_lossy().as_bytes());
    PathBuf::from(format!("/tmp/antislop-{user}")).join(format!("{}.sock", &hash[..16]))
}

pub fn serve(project: &Path, config_path: &Path, state: &Path) -> Result<()> {
    // The version of the configuration this daemon runs with, taken before
    // reading it so an edit made meanwhile still counts as newer.
    let loaded = modified(config_path);
    let project = project.canonicalize()?;
    // Check the user-owned registry before opening any project state. A
    // project database cannot grant trust, even for the internal entrypoint.
    let registry = super::trust::Registry::from_env()?;
    require(
        registry.is_trusted(&project),
        &format!(
            "{} is not a trusted directory: run antislop there in a terminal first, or pass --trust",
            project.display()
        ),
    )?;
    if let Err(error) = cleanup_config_temporaries(config_path) {
        eprintln!("could not clean abandoned configuration writes: {error}");
    }
    let core = Core::open(state, Config::load(config_path)?)?;
    core.config_modified.store(loaded, Ordering::SeqCst);
    let config_path = config_path.canonicalize()?;
    let socket = socket(state);
    if let Some(parent) = socket.parent() {
        private_directory(parent)?;
    }
    // The state lock is held, so a socket file left here is stale.
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket)?;
    eprintln!("antislop core ready at {}", socket.display());
    // A daemon whose state was deleted (or whose socket was taken over) has
    // nobody to serve: it leaves once idle instead of lingering.
    {
        let core = core.clone();
        let database = state.join("core.db");
        let socket = socket.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(2));
                if !(database.exists() && socket.exists()) && !core.busy() {
                    std::process::exit(0);
                }
            }
        });
    }
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let core = core.clone();
        let project = project.clone();
        let config_path = config_path.clone();
        let socket = socket.clone();
        std::thread::spawn(move || {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let request = crate::transport::read::<Request>(&mut BufReader::new(&stream))
                .and_then(|request| request.ok_or_else(|| Error::Invalid("empty request".into())));
            let shutdown = matches!(request, Ok(Request::Shutdown | Request::ForceShutdown));
            let response =
                match request.and_then(|request| handle(&core, &project, &config_path, request)) {
                    Ok(data) => Response {
                        data: Some(data),
                        error: None,
                    },
                    Err(error) => Response {
                        data: None,
                        error: Some(error.to_string()),
                    },
                };
            let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
            if let Ok(mut bytes) = serde_json::to_vec(&response) {
                bytes.push(b'\n');
                let _ = stream.write_all(&bytes);
            }
            if shutdown && response.error.is_none() {
                let _ = std::fs::remove_file(&socket);
                std::process::exit(0);
            }
        });
    }
    Ok(())
}

fn handle(core: &Arc<Core>, project: &Path, config: &Path, request: Request) -> Result<Value> {
    require(
        !core.state.join("stopped").exists()
            || matches!(
                request,
                Request::Identity | Request::Shutdown | Request::ForceShutdown
            ),
        "the daemon is stopping",
    )?;
    Ok(match request {
        Request::GetConfig => {
            let _guard = core
                .config_update
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            {
                let toml = std::fs::read_to_string(config)?;
                json!({"revision": crate::hash(&toml), "toml": toml, "path": config})
            }
        }
        Request::SetConfig {
            toml,
            expected_revision,
        } => {
            let parsed = Config::from_toml(&toml)?;
            let _guard = core
                .config_update
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            require(
                crate::hash(std::fs::read(config)?) == expected_revision,
                "configuration changed: reload before saving; draft preserved",
            )?;
            let prepared = PreparedConfigWrite::prepare(config, &toml)?;
            let mut applied = false;
            let mut warnings = Vec::new();
            let result = core.store.publish_config_with_event(config, || {
                prepared.publish()?;
                core.replace_config(parsed);
                core.config_modified
                    .store(modified(config), Ordering::SeqCst);
                applied = true;
                Ok(())
            });
            if let Err(error) = result {
                if !applied {
                    return Err(error);
                }
                warnings.push(ConfigUpdateWarning {
                    stage: ConfigUpdateStage::EventCommit,
                    message: error.to_string(),
                });
            }
            if let Err(error) = prepared.sync_directory() {
                warnings.push(ConfigUpdateWarning {
                    stage: ConfigUpdateStage::DirectorySync,
                    message: error.to_string(),
                });
            }
            let status = if warnings.is_empty() {
                ConfigUpdateStatus::Applied
            } else {
                ConfigUpdateStatus::AppliedWithWarnings
            };
            serde_json::to_value(ConfigUpdateReply {
                toml,
                path: config.to_path_buf(),
                status,
                warnings,
            })?
        }
        Request::Identity => {
            let _guard = core
                .config_update
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            json!({
                "build": build(),
                "busy": core.busy(),
                "pid": std::process::id(),
                "project": project,
                "config": config,
                "config_modified": core.config_modified.load(Ordering::SeqCst),
            })
        }
        Request::Shutdown => {
            require(
                !core.busy(),
                "the daemon is executing work; use antislop stop --force to cancel it",
            )?;
            std::fs::write(core.state.join("stopped"), [])?;
            Value::Null
        }
        Request::ForceShutdown => {
            std::fs::write(core.state.join("stopped"), [])?;
            let deadline = Instant::now() + Duration::from_secs(5);
            while core.busy() && Instant::now() < deadline {
                for conversation in core.conversations()? {
                    core.cancel(&conversation.id);
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            // End any CLI still alive before recording interrupted work.
            for (pid, started_ms, provider) in core.store.running_processes()? {
                super::agent::reap(pid, started_ms, &provider);
            }
            core.store.interrupt_running_turns()?;
            core.store.interrupt_unfinished_tasks()?;
            Value::Null
        }
        Request::Conversations => json!(core.conversations()?),
        Request::Start => json!(core.start(project)?),
        Request::StartIn {
            project: target_project,
        } => {
            let target_path = Path::new(&target_project).canonicalize()?;
            require(
                super::trust::Registry::from_env()?.is_trusted(&target_path),
                &format!(
                    "{} is not a trusted directory: run antislop there in a terminal first",
                    target_path.display()
                ),
            )?;
            core.store.trust_project(&target_path.to_string_lossy())?;
            json!(core.start(&target_path)?)
        }
        Request::Send { conversation, text } => {
            validate_project_conversation(core, project, &conversation)?;
            core.submit_addressed(&conversation, &text, None)?
        }
        Request::SendAddressed {
            conversation,
            text,
            to,
        } => {
            validate_project_conversation(core, project, &conversation)?;
            core.submit_addressed(&conversation, &text, to.as_deref())?
        }
        Request::Accept { conversation, plan } => {
            validate_project_conversation(core, project, &conversation)?;
            validate_conversation_plan(core, &conversation, &plan)?;
            json!(core.accept_plan(&conversation, &plan)?.len())
        }
        Request::AcceptScoped {
            conversation,
            plan,
            scope_hash,
        } => {
            validate_project_conversation(core, project, &conversation)?;
            if let Some(expected) = &scope_hash {
                match core.plan_scope_hash(&conversation, &plan) {
                    Ok(current) => require(expected == &current, "stale_scope")?,
                    Err(_) => return Err(Error::Invalid("stale_scope".into())),
                }
            }
            validate_conversation_plan(core, &conversation, &plan)?;
            json!(core.accept_plan(&conversation, &plan)?.len())
        }
        Request::Reject {
            conversation,
            plan,
            reason,
        } => {
            validate_project_conversation(core, project, &conversation)?;
            validate_conversation_plan(core, &conversation, &plan)?;
            core.reject_plan(&conversation, &plan, reason.as_deref())?;
            Value::Null
        }
        Request::RejectScoped {
            conversation,
            plan,
            reason,
            scope_hash,
        } => {
            validate_project_conversation(core, project, &conversation)?;
            if let Some(expected) = &scope_hash {
                match core.plan_scope_hash(&conversation, &plan) {
                    Ok(current) => require(expected == &current, "stale_scope")?,
                    Err(_) => return Err(Error::Invalid("stale_scope".into())),
                }
            }
            validate_conversation_plan(core, &conversation, &plan)?;
            core.reject_plan(&conversation, &plan, reason.as_deref())?;
            Value::Null
        }
        Request::ReviewComments { conversation, plan } => {
            validate_project_conversation(core, project, &conversation)?;
            validate_conversation_plan(core, &conversation, &plan)?;
            json!(core.review_comments(&conversation, &plan)?)
        }
        Request::Recover(req) => {
            validate_project_conversation(core, project, &req.conversation)?;
            validate_conversation_plan(core, &req.conversation, &req.plan)?;
            validate_plan_task(core, &req.plan, &req.task)?;
            json!(core.recover_task(req)?)
        }
        Request::Resume { conversation, plan } => {
            validate_project_conversation(core, project, &conversation)?;
            validate_conversation_plan(core, &conversation, &plan)?;
            json!(core.resume_plan(&conversation, &plan)?.len())
        }
        Request::ResumeScoped {
            conversation,
            plan,
            scope_hash,
        } => {
            validate_project_conversation(core, project, &conversation)?;
            if let Some(expected) = &scope_hash {
                match core.plan_scope_hash(&conversation, &plan) {
                    Ok(current) => require(expected == &current, "stale_scope")?,
                    Err(_) => return Err(Error::Invalid("stale_scope".into())),
                }
            }
            validate_conversation_plan(core, &conversation, &plan)?;
            json!(core.resume_plan(&conversation, &plan)?.len())
        }
        Request::Cancel { conversation } => {
            validate_project_conversation(core, project, &conversation)?;
            core.cancel(&conversation);
            Value::Null
        }
        Request::CancelScoped {
            conversation,
            scope_hash,
        } => {
            validate_project_conversation(core, project, &conversation)?;
            if let Some(expected) = &scope_hash {
                match core.conversation_scope_hash(&conversation) {
                    Ok(current) => require(expected == &current, "stale_scope")?,
                    Err(_) => return Err(Error::Invalid("stale_scope".into())),
                }
            }
            core.cancel(&conversation);
            Value::Null
        }
        Request::Pause {
            conversation,
            paused,
        } => {
            validate_project_conversation(core, project, &conversation)?;
            core.pause(&conversation, paused)?;
            Value::Null
        }
        Request::Mode {
            conversation,
            autonomous,
        } => {
            validate_project_conversation(core, project, &conversation)?;
            core.set_mode(&conversation, autonomous)?;
            Value::Null
        }
        Request::Plans {
            conversation,
            offset,
            limit,
        } => {
            validate_project_conversation(core, project, &conversation)?;
            let offset = offset.unwrap_or(0);
            let limit = limit.unwrap_or(50);
            json!(core.plans(&conversation, offset, limit)?)
        }
        Request::Plan { conversation, plan } => {
            validate_project_conversation(core, project, &conversation)?;
            validate_conversation_plan(core, &conversation, &plan)?;
            json!(core.plan_history(&conversation, &plan)?)
        }
        Request::Comments {
            conversation,
            plan,
            status,
        } => {
            validate_project_conversation(core, project, &conversation)?;
            if let Some(p) = &plan {
                validate_conversation_plan(core, &conversation, p)?;
            }
            let filter_status = match status.as_deref() {
                Some(s) if s.eq_ignore_ascii_case("all") => None,
                Some(s) => Some(s),
                None => Some("open"),
            };
            let comments = match &plan {
                Some(p) => core
                    .store
                    .comments_by_plan(&conversation, p, filter_status)?,
                None => core.store.comments_all(&conversation, filter_status)?,
            };
            json!(comments)
        }
        Request::Comment {
            conversation,
            target,
            text,
        } => {
            validate_project_conversation(core, project, &conversation)?;
            json!(core.comment(&conversation, &target, &text)?)
        }
        Request::CommentScoped {
            conversation,
            plan,
            target,
            text,
        } => {
            validate_project_conversation(core, project, &conversation)?;
            if let Some(p) = &plan {
                validate_conversation_plan(core, &conversation, p)?;
            }
            json!(core.comment_scoped(&conversation, plan.as_deref(), &target, &text)?)
        }
        Request::CommentStatus {
            conversation,
            id,
            status,
        } => {
            validate_project_conversation(core, project, &conversation)?;
            json!(core.set_comment(&conversation, id, &status)?)
        }
        Request::Task { plan, id } => {
            let conv = core.store.plan_conversation(&plan)?;
            validate_project_conversation(core, project, &conv)?;
            validate_plan_task(core, &plan, &id)?;
            json!(core.task(&plan, &id)?)
        }
        Request::TaskScoped {
            conversation,
            plan,
            id,
        } => {
            if let Some(conv) = &conversation {
                validate_project_conversation(core, project, conv)?;
                validate_conversation_plan(core, conv, &plan)?;
            } else {
                let conv = core.store.plan_conversation(&plan)?;
                validate_project_conversation(core, project, &conv)?;
            }
            validate_plan_task(core, &plan, &id)?;
            json!(core.task(&plan, &id)?)
        }
        Request::TaskChecks {
            conversation,
            plan,
            id,
        } => {
            if let Some(conv) = &conversation {
                validate_project_conversation(core, project, conv)?;
                validate_conversation_plan(core, conv, &plan)?;
            } else {
                let conv = core.store.plan_conversation(&plan)?;
                validate_project_conversation(core, project, &conv)?;
            }
            validate_plan_task(core, &plan, &id)?;
            json!(core.task_checks(&plan, &id)?)
        }
        Request::Diff {
            conversation,
            plan,
            id,
        } => {
            validate_project_conversation(core, project, &conversation)?;
            validate_conversation_plan(core, &conversation, &plan)?;
            validate_plan_task(core, &plan, &id)?;
            core.diff(&conversation, &plan, &id)?
        }
        Request::Review {
            conversation,
            plan,
            id,
        } => {
            validate_project_conversation(core, project, &conversation)?;
            validate_conversation_plan(core, &conversation, &plan)?;
            validate_plan_task(core, &plan, &id)?;
            core.review_change(&conversation, &plan, &id)?;
            Value::Null
        }
        Request::ReviewScoped {
            conversation,
            plan,
            id,
            scope_hash,
        } => {
            validate_project_conversation(core, project, &conversation)?;
            if let Some(expected) = &scope_hash {
                match core.diff_scope_hash(&conversation, &plan, &id) {
                    Ok(current) => require(expected == &current, "stale_scope")?,
                    Err(_) => return Err(Error::Invalid("stale_scope".into())),
                }
            }
            validate_conversation_plan(core, &conversation, &plan)?;
            validate_plan_task(core, &plan, &id)?;
            core.review_change(&conversation, &plan, &id)?;
            Value::Null
        }
        Request::Events {
            conversation,
            after,
            wait_ms,
        } => handle_events(core, project, &conversation, after, wait_ms, None)?,
        Request::EventsPaged {
            conversation,
            after,
            wait_ms,
            limit,
        } => handle_events(core, project, &conversation, after, wait_ms, limit)?,
        Request::Sessions { conversation } => {
            validate_project_conversation(core, project, &conversation)?;
            json!(core.sessions(&conversation)?)
        }
        Request::History {
            conversation,
            after,
            limit,
        } => {
            validate_project_conversation(core, project, &conversation)?;
            json!(core.history(&conversation, after, limit)?)
        }
        Request::Mail { conversation } => {
            validate_project_conversation(core, project, &conversation)?;
            json!(core.store.mail(&conversation)?)
        }
        Request::TrustStatus => {
            json!({
                "project": project.to_string_lossy(),
                "trusted": super::trust::Registry::from_env()?.is_trusted(project),
            })
        }
        Request::Trust { trusted } => {
            let registry = super::trust::Registry::from_env()?;
            if trusted {
                registry.trust(project)?;
            } else {
                registry.untrust(project)?;
            }
            json!({
                "project": project.to_string_lossy(),
                "trusted": trusted,
            })
        }
    })
}

fn validate_project_conversation(core: &Core, project: &Path, conversation: &str) -> Result<()> {
    let conv = core.store.conversation(conversation)?;
    require(
        // The projects this daemon was asked to serve (`StartIn`) are in its
        // own state; the user's list of trusted directories says nothing
        // about which daemon may open whose conversations.
        conv.project == project.to_string_lossy() || core.store.is_trusted(&conv.project)?,
        &format!(
            "conversation {conversation} does not belong to project {}",
            project.display()
        ),
    )?;
    Ok(())
}

fn validate_conversation_plan(core: &Core, conversation: &str, plan: &str) -> Result<()> {
    require(
        core.store
            .plan_belongs_to_conversation(conversation, plan)?,
        &format!("plan {plan} does not belong to conversation {conversation}"),
    )?;
    Ok(())
}

fn validate_plan_task(core: &Core, plan: &str, task_id: &str) -> Result<()> {
    require(
        core.store.task_in_plan(plan, task_id)?,
        &format!("task {task_id} does not belong to plan {plan}"),
    )?;
    Ok(())
}

fn handle_events(
    core: &Core,
    project: &Path,
    conversation: &str,
    after: u64,
    wait_ms: u64,
    limit: Option<usize>,
) -> Result<Value> {
    validate_project_conversation(core, project, conversation)?;
    let lim = limit.unwrap_or(EVENTS_PER_REPLY).min(EVENTS_PER_REPLY);
    let deadline = Instant::now() + Duration::from_millis(wait_ms).min(LONGEST_WAIT);
    let events = loop {
        let events = core.store.events_since(conversation, after, lim)?;
        if !events.is_empty() || Instant::now() >= deadline {
            break events;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let take = reply_event_count(&events, REPLY_BYTES);
    let events: Vec<_> = events.into_iter().take(take).collect();
    let (oldest_seq, latest_seq, total) = core.store.event_bounds(conversation)?;
    let next_after = events.last().map(|e| e.seq);
    let has_more = match next_after {
        Some(last_seq) => last_seq < latest_seq,
        None => false,
    };
    let history_unavailable = total > 0
        && (after > latest_seq || (oldest_seq > 1 && after < oldest_seq.saturating_sub(1)));
    Ok(json!({
        "events": events,
        "idle": core.is_idle(conversation),
        "autonomous": core.autonomous(conversation)?,
        "has_more": has_more,
        "history_unavailable": history_unavailable,
        "oldest_seq": oldest_seq,
        "latest_seq": latest_seq,
        "total": total,
    }))
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum ConfigUpdateStatus {
    Applied,
    AppliedWithWarnings,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum ConfigUpdateStage {
    EventCommit,
    DirectorySync,
}

#[derive(Serialize)]
struct ConfigUpdateWarning {
    stage: ConfigUpdateStage,
    message: String,
}

#[derive(Serialize)]
struct ConfigUpdateReply {
    toml: String,
    path: PathBuf,
    status: ConfigUpdateStatus,
    warnings: Vec<ConfigUpdateWarning>,
}

/// Prepare and flush file contents before acquiring the event store lock.
struct PreparedConfigWrite {
    target: PathBuf,
    temporary: PathBuf,
    directory: std::fs::File,
    // Keep the advisory lock until publication or cleanup has completed.
    lease: std::fs::File,
}

/// Serialize creation/locking with orphan scanning, including the small gap
/// between create_new and locking the new descriptor.
struct ConfigDirectoryLease(std::fs::File);

impl ConfigDirectoryLease {
    fn acquire(directory: &std::fs::File) -> Result<Self> {
        use std::os::fd::AsRawFd;
        let lease = Self(directory.try_clone()?);
        // SAFETY: this owned directory descriptor remains open until Drop.
        if unsafe { libc::flock(lease.0.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(lease)
    }
}

impl Drop for ConfigDirectoryLease {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        // SAFETY: the owned descriptor is still open while the lock releases.
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

fn config_temporary_prefix(target: &Path) -> String {
    format!(
        ".antislop-config-{}-",
        &crate::hash(target.as_os_str().as_encoded_bytes())[..16]
    )
}

/// Reap only this target's unlocked, user-owned temporary files. Active
/// writers, other configurations and symbolic links are never removed.
fn cleanup_config_temporaries(path: &Path) -> Result<usize> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let target = path.canonicalize()?;
    let directory = target
        .parent()
        .ok_or_else(|| Error::Invalid("configuration has no parent directory".into()))?;
    let directory_handle = std::fs::File::open(directory)?;
    let _lease = ConfigDirectoryLease::acquire(&directory_handle)?;
    let prefix = config_temporary_prefix(&target);
    let mut removed = 0;
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with(&prefix) || !name.ends_with(".tmp") {
            continue;
        }
        let Ok(file) = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(entry.path())
        else {
            continue;
        };
        let metadata = file.metadata()?;
        // SAFETY: geteuid has no arguments and flock receives an owned fd.
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0
        {
            continue;
        }
        let current = std::fs::symlink_metadata(entry.path())?;
        if current.dev() == metadata.dev() && current.ino() == metadata.ino() {
            std::fs::remove_file(entry.path())?;
            removed += 1;
        }
    }
    if removed > 0 {
        std::fs::File::open(directory)?.sync_all()?;
    }
    Ok(removed)
}

impl PreparedConfigWrite {
    fn prepare(path: &Path, text: &str) -> Result<Self> {
        let target = path.canonicalize()?;
        let temporary = target.with_file_name(format!(
            "{}{}.tmp",
            config_temporary_prefix(&target),
            uuid::Uuid::new_v4()
        ));
        let directory = std::fs::File::open(
            target
                .parent()
                .ok_or_else(|| Error::Invalid("configuration has no parent directory".into()))?,
        )?;
        let directory_lease = ConfigDirectoryLease::acquire(&directory)?;
        use std::os::unix::fs::OpenOptionsExt;
        let lease = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        let mut prepared = Self {
            target,
            temporary,
            directory,
            lease,
        };
        use std::os::fd::AsRawFd;
        // SAFETY: the owned file descriptor remains open for the prepared write.
        require(
            unsafe { libc::flock(prepared.lease.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
            "could not lock prepared configuration write",
        )?;
        drop(directory_lease);
        let file = &mut prepared.lease;
        file.set_permissions(std::fs::metadata(&prepared.target)?.permissions())?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        Ok(prepared)
    }

    fn publish(&self) -> Result<()> {
        std::fs::rename(&self.temporary, &self.target)?;
        Ok(())
    }

    fn sync_directory(&self) -> Result<()> {
        self.directory.sync_all()?;
        Ok(())
    }
}

impl Drop for PreparedConfigWrite {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.temporary);
    }
}

#[cfg(test)]
fn atomic_config_write(path: &Path, text: &str) -> Result<()> {
    let prepared = PreparedConfigWrite::prepare(path, text)?;
    prepared.publish()?;
    prepared.sync_directory()
}

/// Modification stamp used to detect configuration changed outside the daemon.
fn modified(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |time| time.as_nanos() as u64)
}

fn build() -> &'static str {
    static BUILD: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    BUILD.get_or_init(|| {
        std::env::current_exe()
            .and_then(|exe| build_identity(&exe).map_err(std::io::Error::other))
            .unwrap_or_default()
    })
}

/// A frontend's handle on the project's daemon.
#[derive(Clone)]
pub struct Client {
    socket: PathBuf,
    state: PathBuf,
    /// Something the user should know about the daemon they reached.
    pub notice: Option<String>,
    /// Project, configuration and state, to start the daemon again if it
    /// dies under a frontend that is following events.
    origin: Option<(PathBuf, PathBuf, PathBuf)>,
}

impl Client {
    /// A handle on a daemon already serving `state`.
    pub fn at(state: &Path) -> Self {
        Self {
            socket: socket(state),
            state: state.to_owned(),
            notice: None,
            origin: None,
        }
    }

    /// The state directory this daemon serves, including custom `--state` paths.
    pub fn state_dir(&self) -> &Path {
        &self.state
    }

    /// Stops the daemon and waits for it to release its state lock.
    /// With force, running agents are cancelled before the daemon exits.
    /// Returns false if none is running; never creates state or starts a daemon.
    pub fn stop(state: &Path, force: bool) -> Result<bool> {
        if !state.try_exists()? {
            return Ok(false);
        }
        private_directory(state)?;
        let state = state.canonicalize()?;
        let client = Self::at(&state);
        if let Some(parent) = client.socket.parent() {
            if !parent.try_exists()? {
                return Ok(false);
            }
            private_directory(parent)?;
        }
        let request = if force {
            Request::ForceShutdown
        } else {
            Request::Shutdown
        };
        match client.call(&request) {
            Ok(_) => {
                wait_unlocked(&state)?;
                Ok(true)
            }
            Err(Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                ) =>
            {
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }

    /// Connects to the project's daemon, starting it (or replacing an idle one
    /// from another build or configuration) as needed. For the same project
    /// and configuration path, a busy daemon keeps the build and configuration
    /// its work started with; the frontend reconnects and explains the delay.
    pub fn connect(project: &Path, config: &Path, state: &Path) -> Result<Self> {
        let canonical_config = config.canonicalize()?;
        let config = canonical_config.as_path();
        private_directory(state)?;
        let mut client = Self::at(state);
        client.origin = Some((project.into(), config.into(), state.into()));
        if let Some(parent) = client.socket.parent() {
            // Never talk to a socket someone else could have placed.
            private_directory(parent)?;
        }
        let executable = std::env::current_exe()?;
        if let Ok(identity) = client.call(&Request::Identity) {
            let same_build =
                identity["build"].as_str() == Some(build_identity(&executable)?.as_str());
            let same_context = identity["project"].as_str() == project.to_str()
                && identity["config"].as_str() == config.to_str();
            let config_current = identity["config_modified"].as_u64() == Some(modified(config));
            if same_context && same_build && config_current {
                return Ok(client);
            }
            if same_context && identity["busy"] == true {
                client.notice = Some(if same_build {
                    "antislop.toml changed: running work keeps its configuration, \
                     the new one applies once it finishes and you reopen"
                        .into()
                } else {
                    format!(
                        "connected to the running daemon (pid {}): running work keeps its build and configuration; \
                         the current build and configuration apply once it finishes and you reopen",
                        identity["pid"]
                    )
                });
                return Ok(client);
            }
            require(
                identity["busy"] != true,
                &format!(
                    "the daemon (pid {}) runs another project or configuration path and is executing work; reopen after it finishes or use a different --state directory",
                    identity["pid"]
                ),
            )?;
            client.call(&Request::Shutdown)?;
            wait_unlocked(state)?;
        }
        match std::fs::remove_file(state.join("stopped")) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(state.join("daemon.log"))?;
        let mut command = std::process::Command::new(executable);
        command
            .arg("__core")
            .arg(project)
            .arg(config)
            .arg(state)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(log);
        // Its own session: the daemon outlives the terminal that started it.
        // SAFETY: setsid is async-signal-safe and touches no shared state.
        unsafe {
            std::os::unix::process::CommandExt::pre_exec(&mut command, || {
                libc::setsid();
                Ok(())
            });
        }
        let mut child = command.spawn()?;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if client.call(&Request::Identity).is_ok() {
                return Ok(client);
            }
            if let Some(status) = child.try_wait()? {
                let log = std::fs::read_to_string(state.join("daemon.log")).unwrap_or_default();
                let reason = log.lines().last().unwrap_or_default().to_owned();
                return Err(Error::Invalid(format!(
                    "the daemon exited ({status}): {reason}"
                )));
            }
            require(Instant::now() < deadline, "the daemon did not start")?;
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// One request. Following events survives a daemon that died: the
    /// daemon is started again (and picks up the work) and the request,
    /// which changes nothing, is repeated once.
    pub fn call(&self, request: &Request) -> Result<Value> {
        let result = self.call_once(request);
        // A busy daemon may intentionally keep its previous build while a new
        // frontend connects. Only an unsupported operation permits fallback;
        // scope and other validation errors must reach the caller unchanged.
        let result = match (&result, request) {
            (
                Err(Error::Invalid(message)),
                Request::TaskChecks {
                    conversation,
                    plan,
                    id,
                },
            ) if message.contains("unknown variant `task_checks`") => {
                self.call_once(&Request::TaskScoped {
                    conversation: conversation.clone(),
                    plan: plan.clone(),
                    id: id.clone(),
                })
            }
            _ => result,
        };
        match (result, &self.origin) {
            (Err(error), Some((project, config, state)))
                // State the user deleted stays deleted.
                if matches!(request, Request::Events { .. } | Request::EventsPaged { .. })
                    && lost(&error)
                    && state.join("core.db").exists()
                    && !state.join("stopped").exists() =>
            {
                Self::connect(project, config, state).map_err(|_| error)?;
                self.call_once(request)
            }
            (result, _) => result,
        }
    }

    fn call_once(&self, request: &Request) -> Result<Value> {
        let mut stream = UnixStream::connect(&self.socket)?;
        let wait = match request {
            Request::Events { wait_ms, .. } | Request::EventsPaged { wait_ms, .. } => {
                Duration::from_millis(*wait_ms).min(LONGEST_WAIT)
            }
            _ => Duration::ZERO,
        };
        stream.set_read_timeout(Some(wait + Duration::from_secs(10)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        let mut bytes = serde_json::to_vec(request)?;
        bytes.push(b'\n');
        stream.write_all(&bytes)?;
        let response: Response = crate::transport::read(&mut BufReader::new(stream))?
            .ok_or_else(|| Error::Invalid(DISCONNECTED.into()))?;
        match response.error {
            Some(error) => Err(Error::Invalid(error)),
            None => Ok(response.data.unwrap_or_default()),
        }
    }
}

/// Whether a request failed because the daemon was gone, not because it
/// refused the request.
fn lost(error: &Error) -> bool {
    match error {
        Error::Io(_) => true,
        Error::Invalid(message) => message == DISCONNECTED,
        _ => false,
    }
}

const DISCONNECTED: &str = "the daemon disconnected";

/// Waits until the daemon that held `state` has exited.
fn wait_unlocked(state: &Path) -> Result<()> {
    let lock = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(state.join("core.lock"))?;
    let deadline = Instant::now() + Duration::from_secs(5);
    // SAFETY: flock on an owned descriptor, closed on return.
    while unsafe {
        libc::flock(
            std::os::fd::AsRawFd::as_raw_fd(&lock),
            libc::LOCK_EX | libc::LOCK_NB,
        )
    } != 0
    {
        require(Instant::now() < deadline, "the old daemon did not stop")?;
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}

#[cfg(test)]
mod live_config_tests {
    use super::*;

    #[test]
    fn abrupt_exit_before_and_after_config_publish_keeps_a_complete_file() {
        const CHILD: &str = "ANTISLOP_TEST_CONFIG_EXIT_PHASE";
        if let Some(phase) = std::env::var_os(CHILD) {
            let path = PathBuf::from(std::env::var_os("ANTISLOP_TEST_CONFIG_EXIT_PATH").unwrap());
            let text = std::fs::read_to_string(&path)
                .unwrap()
                .replace("max_parallel = 4", "max_parallel = 1");
            let prepared = PreparedConfigWrite::prepare(&path, &text).unwrap();
            if phase == "published" {
                prepared.publish().unwrap();
            }
            // Exit without dropping PreparedConfigWrite, as after a process
            // interruption between preparation, publication and directory sync.
            std::process::exit(0);
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let original = include_str!("../../examples/antislop.toml");
        for phase in ["prepared", "published"] {
            std::fs::write(&path, original).unwrap();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "lean::daemon::live_config_tests::abrupt_exit_before_and_after_config_publish_keeps_a_complete_file"])
                .env(CHILD, phase)
                .env("ANTISLOP_TEST_CONFIG_EXIT_PATH", &path)
                .stdout(std::process::Stdio::null())
                .status().unwrap();
            assert!(status.success());
            let expected = if phase == "prepared" {
                original.to_owned()
            } else {
                original.replace("max_parallel = 4", "max_parallel = 1")
            };
            assert_eq!(std::fs::read_to_string(&path).unwrap(), expected);
            Config::load(&path).unwrap();
            atomic_config_write(&path, original).unwrap();
            assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        }
    }

    #[test]
    fn configuration_requests_validate_before_writing_and_record_the_update() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let original = include_str!("../../examples/antislop.toml");
        std::fs::write(&path, original).unwrap();
        let core = Core::open(
            &dir.path().join(".antislop"),
            Config::from_toml(original).unwrap(),
        )
        .unwrap();
        let old = core.config_snapshot();
        let call = |request| handle(&core, dir.path(), &path, request);
        assert!(
            call(Request::SetConfig {
                toml: "bad = [".into(),
                expected_revision: crate::hash(original),
            })
            .is_err()
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        assert_eq!(*core.config_snapshot(), *old);
        let updated = original.replace("max_parallel = 4", "max_parallel = 1");
        call(Request::SetConfig {
            toml: updated.clone(),
            expected_revision: crate::hash(original),
        })
        .unwrap();
        assert_eq!(core.config_snapshot().policy.max_parallel, 1);
        assert_eq!(old.policy.max_parallel, 4);
        let stale = call(Request::SetConfig {
            toml: original.into(),
            expected_revision: crate::hash(original),
        })
        .unwrap_err();
        assert!(stale.to_string().contains("configuration changed"));
        assert_eq!(core.config_snapshot().policy.max_parallel, 1);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), updated);
        assert_eq!(call(Request::GetConfig).unwrap()["toml"], updated);
        assert_eq!(
            call(Request::Identity).unwrap()["config_modified"],
            modified(&path)
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), updated);
        let conversation = core.start(dir.path()).unwrap();
        assert!(
            core.store
                .events_since(&conversation.id, 0, 100)
                .unwrap()
                .iter()
                .any(|event| event.kind == "config_updated")
        );
        let db = rusqlite::Connection::open(core.state.join("core.db")).unwrap();
        db.execute_batch("CREATE TRIGGER reject_config_event BEFORE INSERT ON events WHEN NEW.kind='config_updated' BEGIN SELECT RAISE(FAIL,'injected log failure'); END;").unwrap();
        let failure = call(Request::SetConfig {
            toml: original.into(),
            expected_revision: crate::hash(&updated),
        })
        .unwrap_err();
        assert!(failure.to_string().contains("injected log failure"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), updated);
        assert_eq!(core.config_snapshot().policy.max_parallel, 1);
        db.execute_batch("DROP TRIGGER reject_config_event;
            CREATE TABLE deferred_failure(id TEXT REFERENCES conversations(id) DEFERRABLE INITIALLY DEFERRED);
            CREATE TRIGGER fail_config_commit AFTER INSERT ON events WHEN NEW.kind='config_updated'
            BEGIN INSERT INTO deferred_failure VALUES('missing'); END;").unwrap();
        let applied = call(Request::SetConfig {
            toml: original.into(),
            expected_revision: crate::hash(&updated),
        })
        .unwrap();
        assert_eq!(applied["status"], "applied_with_warnings");
        assert_eq!(applied["warnings"][0]["stage"], "event_commit");
        assert!(
            applied["warnings"][0]["message"]
                .as_str()
                .unwrap()
                .contains("FOREIGN KEY")
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        assert_eq!(core.config_snapshot().policy.max_parallel, 4);
    }

    #[test]
    fn orphan_cleanup_preserves_active_writes_other_targets_and_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("config.toml");
        let other = dir.path().join("other.toml");
        std::fs::write(&target, "original").unwrap();
        std::fs::write(&other, "other").unwrap();
        let mut active = PreparedConfigWrite::prepare(&target, "active").unwrap();
        let other_active = PreparedConfigWrite::prepare(&other, "other active").unwrap();
        let prefix = config_temporary_prefix(&target.canonicalize().unwrap());
        let abandoned = dir.path().join(format!("{prefix}abandoned.tmp"));
        let symlink = dir.path().join(format!("{prefix}symlink.tmp"));
        std::fs::write(&abandoned, "abandoned").unwrap();
        std::os::unix::fs::symlink(&target, &symlink).unwrap();
        assert_eq!(cleanup_config_temporaries(&target).unwrap(), 1);
        assert!(!abandoned.exists());
        assert!(active.temporary.exists());
        assert!(other_active.temporary.exists());
        assert!(
            std::fs::symlink_metadata(&symlink)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "original");
        // Simulate a crashed writer: its descriptor closes without Drop cleanup.
        let temporary = active.temporary.clone();
        let lease = std::mem::replace(&mut active.lease, std::fs::File::open(&target).unwrap());
        drop(lease);
        assert_eq!(cleanup_config_temporaries(&target).unwrap(), 1);
        assert!(!temporary.exists());
    }

    #[test]
    fn preparing_config_does_not_publish_and_drop_removes_the_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("config.toml");
        std::fs::write(&target, "old").unwrap();
        let prepared = PreparedConfigWrite::prepare(&target, "new").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "old");
        assert_eq!(std::fs::read_to_string(&prepared.temporary).unwrap(), "new");
        let temporary = prepared.temporary.clone();
        drop(prepared);
        assert!(!temporary.exists());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "old");
    }

    #[test]
    fn atomic_write_preserves_symlink_and_rejects_missing_target() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.toml");
        let link = dir.path().join("link.toml");
        std::fs::write(&target, "old").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        atomic_config_write(&link, "new").unwrap();
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
        std::fs::remove_file(&target).unwrap();
        assert!(atomic_config_write(&link, "again").is_err());
        assert!(!target.exists());
    }

    #[test]
    fn failed_atomic_rename_preserves_destination_and_cleans_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("config.toml");
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(destination.join("original"), "unchanged").unwrap();
        assert!(atomic_config_write(&destination, "replacement").is_err());
        assert_eq!(
            std::fs::read_to_string(destination.join("original")).unwrap(),
            "unchanged"
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}

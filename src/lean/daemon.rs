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

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    /// Which binary the daemon runs and whether it is executing work.
    Identity,
    /// Stops an idle daemon so a frontend from another build can replace it.
    Shutdown,
    Conversations,
    Start,
    Send {
        conversation: String,
        text: String,
    },
    /// Starts exactly the plan the user saw.
    Accept {
        conversation: String,
        plan: String,
    },
    /// Runs again what the plan left unfinished, through the same sessions.
    Resume {
        conversation: String,
        plan: String,
    },
    Cancel {
        conversation: String,
    },
    /// The user's comment on one element of the team panel or a diff.
    Comment {
        conversation: String,
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
    /// One task of a plan with its latest report and verdict.
    Task {
        plan: String,
        id: String,
    },
    Events {
        conversation: String,
        after: u64,
        wait_ms: u64,
    },
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
    let core = Core::open(state, Config::load(config_path)?)?;
    let project = project.canonicalize()?;
    let config_path = config_path.canonicalize()?;
    let socket = socket(state);
    if let Some(parent) = socket.parent() {
        private_directory(parent)?;
    }
    // The state lock is held, so a socket file left here is stale.
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket)?;
    eprintln!("antislop core ready at {}", socket.display());
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
            let shutdown = matches!(request, Ok(Request::Shutdown)) && !core.busy();
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
            if shutdown {
                let _ = std::fs::remove_file(&socket);
                std::process::exit(0);
            }
        });
    }
    Ok(())
}

fn handle(core: &Arc<Core>, project: &Path, config: &Path, request: Request) -> Result<Value> {
    Ok(match request {
        Request::Identity => json!({
            "build": build(),
            "busy": core.busy(),
            "pid": std::process::id(),
            "project": project,
            "config": config,
            "config_modified": modified(config),
        }),
        Request::Shutdown => {
            require(!core.busy(), "the daemon is executing work")?;
            Value::Null
        }
        Request::Conversations => json!(core.store.conversations()?),
        Request::Start => json!(core.start(project)?),
        Request::Send { conversation, text } => core.submit(&conversation, &text)?,
        Request::Accept { conversation, plan } => {
            json!(core.accept_plan(&conversation, &plan)?.len())
        }
        Request::Resume { conversation, plan } => {
            json!(core.resume_plan(&conversation, &plan)?.len())
        }
        Request::Cancel { conversation } => {
            core.cancel(&conversation);
            Value::Null
        }
        Request::Comment {
            conversation,
            target,
            text,
        } => json!(core.comment(&conversation, &target, &text)?),
        Request::CommentStatus {
            conversation,
            id,
            status,
        } => json!(core.set_comment(&conversation, id, &status)?),
        Request::Task { plan, id } => json!(core.task(&plan, &id)?),
        Request::Diff {
            conversation,
            plan,
            id,
        } => core.diff(&conversation, &plan, &id)?,
        Request::Review {
            conversation,
            plan,
            id,
        } => {
            core.review_change(&conversation, &plan, &id)?;
            Value::Null
        }
        Request::Events {
            conversation,
            after,
            wait_ms,
        } => {
            core.store.conversation(&conversation)?;
            let deadline = Instant::now() + Duration::from_millis(wait_ms).min(LONGEST_WAIT);
            let events = loop {
                let events = core
                    .store
                    .events_since(&conversation, after, EVENTS_PER_REPLY)?;
                if !events.is_empty() || Instant::now() >= deadline {
                    break events;
                }
                std::thread::sleep(Duration::from_millis(50));
            };
            let mut bytes = 0;
            let events: Vec<_> = events
                .into_iter()
                .enumerate()
                .take_while(|(index, event)| {
                    bytes += event.payload.to_string().len();
                    *index == 0 || bytes <= REPLY_BYTES
                })
                .map(|(_, event)| event)
                .collect();
            json!({"events": events, "idle": core.is_idle(&conversation)})
        }
    })
}

/// When the configuration file last changed, so a daemon running on an
/// older version of it is replaced.
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
}

impl Client {
    /// A handle on a daemon already serving `state`.
    pub fn at(state: &Path) -> Self {
        Self {
            socket: socket(state),
        }
    }

    /// Connects to the project's daemon, starting it (or replacing an idle one
    /// from another build) as needed.
    pub fn connect(project: &Path, config: &Path, state: &Path) -> Result<Self> {
        private_directory(state)?;
        let client = Self::at(state);
        if let Some(parent) = client.socket.parent() {
            // Never talk to a socket someone else could have placed.
            private_directory(parent)?;
        }
        let executable = std::env::current_exe()?;
        if let Ok(identity) = client.call(&Request::Identity) {
            let current = identity["build"].as_str() == Some(build_identity(&executable)?.as_str())
                && identity["project"].as_str() == project.to_str()
                && identity["config"].as_str() == config.to_str()
                && identity["config_modified"].as_u64() == Some(modified(config));
            if current {
                return Ok(client);
            }
            require(
                identity["busy"] != true,
                &format!(
                    "the daemon (pid {}) runs another build, project or configuration and is executing work; reopen after it finishes",
                    identity["pid"]
                ),
            )?;
            client.call(&Request::Shutdown)?;
            wait_unlocked(state)?;
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

    pub fn call(&self, request: &Request) -> Result<Value> {
        let mut stream = UnixStream::connect(&self.socket)?;
        let wait = match request {
            Request::Events { wait_ms, .. } => Duration::from_millis(*wait_ms).min(LONGEST_WAIT),
            _ => Duration::ZERO,
        };
        stream.set_read_timeout(Some(wait + Duration::from_secs(10)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        let mut bytes = serde_json::to_vec(request)?;
        bytes.push(b'\n');
        stream.write_all(&bytes)?;
        let response: Response = crate::transport::read(&mut BufReader::new(stream))?
            .ok_or_else(|| Error::Invalid("the daemon disconnected".into()))?;
        match response.error {
            Some(error) => Err(Error::Invalid(error)),
            None => Ok(response.data.unwrap_or_default()),
        }
    }
}

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

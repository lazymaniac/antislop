use super::*;
use crate::lean::daemon::ControlRequest;
mod launch;
use launch::{spawn_daemon, wait_for_daemon};

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
        let Some((state, client)) = stop_client(state)? else {
            return Ok(false);
        };
        send_stop(&client, &state, force)
    }

    /// Connects to the project's daemon, starting it (or replacing an idle one
    /// from another build or configuration) as needed. For the same project
    /// and configuration path, a busy daemon keeps the build and configuration
    /// its work started with; the frontend reconnects and explains the delay.
    pub fn connect(project: &Path, config: &Path, state: &Path) -> Result<Self> {
        let (mut client, config, executable) = prepare_client(project, config, state)?;
        if reuse_existing(&mut client, project, &config, state, &executable)? {
            return Ok(client);
        }
        clear_stopped(state)?;
        let mut child = spawn_daemon(project, &config, state, executable)?;
        wait_for_daemon(&client, &mut child, state)?;
        Ok(client)
    }

    /// One request. Following events survives a daemon that died: the
    /// daemon is started again (and picks up the work) and the request,
    /// which changes nothing, is repeated once.
    pub fn call(&self, request: &Request) -> Result<Value> {
        let result = self.call_once(request);
        match (result, &self.origin) {
            (Err(error), Some((project, config, state)))
                // State the user deleted stays deleted.
                if matches!(request, Request::History(crate::lean::daemon::HistoryRequest::Events { .. }) | Request::History(crate::lean::daemon::HistoryRequest::EventsPaged { .. }))
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
            Request::History(crate::lean::daemon::HistoryRequest::Events { wait_ms, .. })
            | Request::History(crate::lean::daemon::HistoryRequest::EventsPaged {
                wait_ms, ..
            }) => Duration::from_millis(*wait_ms).min(LONGEST_WAIT),
            _ => Duration::ZERO,
        };
        stream.set_read_timeout(Some(wait + Duration::from_secs(10)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        crate::transport::write(
            &mut stream,
            &json!({"protocol":PROTOCOL_VERSION,"request":request}),
        )?;
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
fn stop_client(state: &Path) -> Result<Option<(PathBuf, Client)>> {
    if !state.try_exists()? {
        return Ok(None);
    }
    private_directory(state)?;
    let state = state.canonicalize()?;
    let client = Client::at(&state);
    if let Some(parent) = client.socket.parent() {
        if !parent.try_exists()? {
            return Ok(None);
        }
        private_directory(parent)?;
    }
    Ok(Some((state, client)))
}

fn send_stop(client: &Client, state: &Path, force: bool) -> Result<bool> {
    let request = if force {
        Request::Control(crate::lean::daemon::ControlRequest::ForceShutdown)
    } else {
        Request::Control(crate::lean::daemon::ControlRequest::Shutdown)
    };
    match client.call(&request) {
        Ok(_) => {
            wait_unlocked(state)?;
            Ok(true)
        }
        Err(Error::Io(error)) if daemon_missing(&error) => Ok(false),
        Err(error) => Err(error),
    }
}

fn daemon_missing(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
    )
}

fn prepare_client(
    project: &Path,
    config: &Path,
    state: &Path,
) -> Result<(Client, PathBuf, PathBuf)> {
    let config = config.canonicalize()?;
    private_directory(state)?;
    let mut client = Client::at(state);
    client.origin = Some((project.into(), config.clone(), state.into()));
    if let Some(parent) = client.socket.parent() {
        private_directory(parent)?;
    }
    Ok((client, config, std::env::current_exe()?))
}

fn reuse_existing(
    client: &mut Client,
    project: &Path,
    config: &Path,
    state: &Path,
    executable: &Path,
) -> Result<bool> {
    let Ok(identity) = client.call(&Request::Control(ControlRequest::Identity)) else {
        return Ok(false);
    };
    let same_build = identity["build"].as_str() == Some(build_identity(executable)?.as_str());
    if same_daemon_context(&identity, project, config, same_build) {
        return Ok(true);
    }
    if reuse_busy_daemon(client, &identity, project, config, same_build) {
        return Ok(true);
    }
    require(
        identity["busy"] != true,
        &format!(
            "the daemon (pid {}) runs another project or configuration path and is executing work; reopen after it finishes or use a different --state directory",
            identity["pid"]
        ),
    )?;
    client.call(&Request::Control(ControlRequest::Shutdown))?;
    wait_unlocked(state)?;
    Ok(false)
}

fn same_daemon_context(identity: &Value, project: &Path, config: &Path, same_build: bool) -> bool {
    let same_context = identity["project"].as_str() == project.to_str()
        && identity["config"].as_str() == config.to_str();
    let config_current = identity["config_modified"].as_u64() == Some(modified(config));
    same_context && same_build && config_current
}

fn reuse_busy_daemon(
    client: &mut Client,
    identity: &Value,
    project: &Path,
    config: &Path,
    same_build: bool,
) -> bool {
    let same_context = identity["project"].as_str() == project.to_str()
        && identity["config"].as_str() == config.to_str();
    if !same_context || identity["busy"] != true {
        return false;
    }
    client.notice = Some(busy_notice(identity, same_build));
    true
}

fn busy_notice(identity: &Value, same_build: bool) -> String {
    if same_build {
        "antislop.toml changed: running work keeps its configuration, \
         the new one applies once it finishes and you reopen"
            .into()
    } else {
        format!(
            "connected to the running daemon (pid {}): running work keeps its build and configuration; \
             the current build and configuration apply once it finishes and you reopen",
            identity["pid"]
        )
    }
}

fn clear_stopped(state: &Path) -> Result<()> {
    match std::fs::remove_file(state.join("stopped")) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

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

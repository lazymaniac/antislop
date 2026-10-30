use super::super::*;

pub(super) fn spawn_daemon(
    project: &Path,
    config: &Path,
    state: &Path,
    executable: PathBuf,
) -> Result<std::process::Child> {
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
    detach_daemon(&mut command);
    command.spawn().map_err(Into::into)
}

pub(super) fn detach_daemon(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    // SAFETY: setsid is async-signal-safe and touches no shared state.
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
}

pub(super) fn wait_for_daemon(
    client: &Client,
    child: &mut std::process::Child,
    state: &Path,
) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if client
            .call(&Request::Control(
                crate::lean::daemon::ControlRequest::Identity,
            ))
            .is_ok()
        {
            return Ok(());
        }
        if let Some(status) = child.try_wait()? {
            return Err(daemon_exit_error(status, state));
        }
        require(Instant::now() < deadline, "the daemon did not start")?;
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub(super) fn daemon_exit_error(status: std::process::ExitStatus, state: &Path) -> Error {
    let log = std::fs::read_to_string(state.join("daemon.log")).unwrap_or_default();
    let reason = log.lines().last().unwrap_or_default();
    Error::Invalid(format!("the daemon exited ({status}): {reason}"))
}

use super::*;
use std::io::{Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) fn check_inputs(core: &Core, operation: &Activation) -> Result<()> {
    require(
        build_identity(&operation.executable)? == operation.build,
        "activation executable changed; request activation again",
    )?;
    require(
        crate::hash(std::fs::read(&operation.config)?) == operation.config_revision,
        "activation configuration changed; request activation again",
    )?;
    if let Some(id) = &operation.task {
        let task = core
            .store
            .task_by_id(id)?
            .ok_or_else(|| Error::Invalid("activation task was removed".into()))?;
        require(
            Some(task.input_generation) == operation.generation
                && task.status != task_status::CANCELLED,
            "activation task inputs changed or task was cancelled",
        )?;
        if let Some(conversation) = &operation.conversation {
            require(
                core.store.accepted_plan(conversation)?.as_deref() == Some(&task.plan_id),
                "activation plan changed",
            )?;
        }
    }
    Ok(())
}

struct ChildOwner {
    child: std::process::Child,
    exited: bool,
}
impl Drop for ChildOwner {
    fn drop(&mut self) {
        if !self.exited {
            // A post-spawn failure still owns the live validator. Never detach
            // it or escalate beyond SIGTERM; keep its worker until it exits.
            // SAFETY: this unreaped child owns the group allocated at spawn.
            unsafe {
                libc::kill(-(self.child.id() as i32), libc::SIGTERM);
            }
            let _ = self.child.wait();
        }
    }
}

fn tail(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(8192)))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

// Stream both pipes to disk. The status carries only paths and bounded tails.
fn execute(
    operation: &Activation,
    state: &Path,
    args: &[&std::ffi::OsStr],
    label: &str,
    cancel: &AtomicBool,
    core: Option<&Core>,
) -> Result<Value> {
    use std::os::unix::process::CommandExt;
    let logs = state.join("activation-logs");
    private_directory(&logs)?;
    // IDs are untrusted opaque keys, never path components.
    let stem = format!("{}-{label}", crate::hash(&operation.id));
    let stdout = logs.join(format!("{stem}.stdout"));
    let stderr = logs.join(format!("{stem}.stderr"));
    let output = std::fs::File::create(&stdout)?;
    let errors = std::fs::File::create(&stderr)?;
    let child = std::process::Command::new(&operation.executable)
        .args(args)
        .current_dir(&operation.project)
        .env("PATH", &operation.path)
        .env(
            crate::lean::process_tree::VALIDATOR_MARKER,
            crate::lean::process_tree::validator_marker(state, &operation.id),
        )
        .stdin(std::process::Stdio::null())
        .stdout(output)
        .stderr(errors)
        .process_group(0)
        .spawn()?;
    let mut child = ChildOwner {
        child,
        exited: false,
    };
    if let Some(core) = core {
        core.store.db().execute("INSERT OR REPLACE INTO runtime_validation_processes(operation,phase,pid,started_ms,executable) VALUES(?1,?2,?3,?4,?5)",rusqlite::params![operation.id,label,child.child.id(),crate::lean::store::now_ms(),operation.executable.to_string_lossy()])?;
    }
    let mut signalled = false;
    let result = loop {
        if cancel.load(Ordering::SeqCst) && !signalled {
            // A manual cancellation terminates this validator's whole group.
            // SAFETY: the live child owns the process group just allocated above.
            unsafe {
                libc::kill(-(child.child.id() as i32), libc::SIGTERM);
            }
            signalled = true;
        }
        if let Some(status) = child.child.try_wait()? {
            child.exited = true;
            break status;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    if let Some(core) = core {
        core.store.db().execute(
            "DELETE FROM runtime_validation_processes WHERE operation=?1 AND phase=?2",
            rusqlite::params![operation.id, label],
        )?;
    }
    require(
        !cancel.load(Ordering::SeqCst),
        "runtime activation cancelled",
    )?;
    require(
        result.success(),
        &format!(
            "runtime activation {label} failed ({result}): {}",
            tail(&stderr)?
        ),
    )?;
    let mut input = std::fs::File::open(&stdout)?;
    let mut bytes = Vec::new();
    // Protocol metadata, unlike the full on-disk log, must remain compact.
    Read::by_ref(&mut input)
        .take(65537)
        .read_to_end(&mut bytes)?;
    require(
        bytes.len() <= 65536,
        &format!(
            "runtime activation {label} returned oversized metadata; see {}",
            stdout.display()
        ),
    )?;
    serde_json::from_slice(&bytes).map_err(|e|Error::Invalid(format!("runtime activation {label} requires compatible JSON metadata; rebuild the target binary: {e}; output: {}",String::from_utf8_lossy(&bytes))))
}

pub(super) fn check_target(
    operation: &Activation,
    state: &Path,
    cancel: &AtomicBool,
    core: Option<&Core>,
) -> Result<()> {
    // --version is understood by old binaries; never probe an internal command
    // until the explicit capability handshake succeeds.
    let info = execute(
        operation,
        state,
        &["--version".as_ref(), "--json".as_ref()],
        "version",
        cancel,
        core,
    )?;
    require(
        info["name"] == "antislop"
            && info["build"] == operation.build
            && info["protocol"] == super::super::PROTOCOL_VERSION
            && info["capabilities"]
                .as_array()
                .is_some_and(|items| items.iter().any(|v| v == "runtime_preflight")),
        "target binary lacks compatible runtime_preflight capability; rebuild it before activation",
    )?;
    let checked = execute(
        operation,
        state,
        &[
            "__runtime_preflight".as_ref(),
            operation.project.as_os_str(),
            operation.config.as_os_str(),
            state.as_os_str(),
        ],
        "preflight",
        cancel,
        core,
    )?;
    require(
        checked["build"] == operation.build
            && checked["project"] == operation.project.to_string_lossy().as_ref()
            && checked["config"] == operation.config.to_string_lossy().as_ref(),
        "runtime activation preflight returned a different build or context",
    )
}

pub(super) fn run(core: Arc<Core>, mut operation: Activation, cancel: Arc<AtomicBool>) {
    operation.log = Some(
        core.state
            .join("activation-logs")
            .join(format!("{}-preflight.stderr", crate::hash(&operation.id)))
            .to_string_lossy()
            .into_owned(),
    );
    let result = check_target(&operation, &core.state, &cancel, Some(&core))
        .and_then(|()| check_inputs(&core, &operation));
    let result = (|| -> Result<()> {
        let _guard = core
            .runtime_activation_lock
            .lock()
            .expect("runtime activation mutex");
        let current = storage::load(&core.store.db(), Some(&operation.id), false)?;
        if current.is_none_or(|op| op.state != ActivationState::Validating) {
            return Ok(());
        }
        // Validate again inside the brief publication section; never execute a
        // subprocess here. Changes during a held preflight invalidate its result.
        let result = result.and_then(|()| check_inputs(&core, &operation));
        if cancel.load(Ordering::SeqCst) {
            finish(&core, operation.clone(), ActivationState::Cancelled, None)?;
        } else if let Err(error) = result {
            operation.diagnostic_tail =
                Some(crate::lean::agent::clip_tail(&error.to_string(), 8192));
            finish(
                &core,
                operation.clone(),
                ActivationState::Failed,
                Some(error.to_string()),
            )?;
        } else {
            operation.state = ActivationState::WaitingForIdle;
            save(&core, &operation)?;
            announce(&core, &operation)?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        eprintln!("runtime activation {}: {error}", operation.id);
    }
    core.runtime_activation_cancels
        .lock()
        .expect("activation cancellations")
        .remove(&operation.id);
}

pub(super) fn recover_processes(core: &Core) -> Result<()> {
    let recorded = {
        let db = core.store.db();
        db.prepare("SELECT pid,started_ms,executable FROM runtime_validation_processes")?
            .query_map([], |r| {
                Ok((
                    r.get::<_, u32>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?
    };
    for (pid, started, executable) in recorded {
        crate::lean::agent::terminate_validator(pid, started, &executable);
    }
    crate::lean::process_tree::terminate_validation_leftovers(&core.state);
    core.store
        .db()
        .execute("DELETE FROM runtime_validation_processes", [])?;
    Ok(())
}

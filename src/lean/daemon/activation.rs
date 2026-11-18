//! Durable, one-shot runtime handoff. Only PATH crosses the client boundary;
//! the successor inherits the daemon's remaining environment, not agent secrets.
use super::*;
use crate::lean::{Store, TaskContract, tasks::status as task_status};
use std::collections::BTreeSet;
use std::ffi::OsString;

const PENDING: &str = "runtime-activation.pending.json";
const LAST: &str = "runtime-activation.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Resume {
    conversation: String,
    plan: Option<String>,
    #[serde(default)]
    cancelled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Activation {
    id: String,
    executable: PathBuf,
    build: String,
    project: PathBuf,
    config: PathBuf,
    path: OsString,
    task: Option<String>,
    conversation: Option<String>,
    resume: Vec<Resume>,
    state: ActivationState,
    #[serde(default)]
    signature: String,
    #[serde(default)]
    generation: Option<i64>,
    #[serde(default)]
    config_revision: String,
    #[serde(default)]
    log: Option<String>,
    #[serde(default)]
    diagnostic_tail: Option<String>,
    error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ActivationState {
    #[serde(alias = "pending")]
    Validating,
    WaitingForIdle,
    Switching,
    Applied,
    Failed,
    Cancelled,
    Interrupted,
}
impl ActivationState {
    fn name(self) -> &'static str {
        match self {
            Self::Validating => "validating",
            Self::WaitingForIdle => "waiting_for_idle",
            Self::Switching => "switching",
            Self::Applied => "applied",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }
}
mod storage;
mod validation;
use storage::save;

fn active(core: &Core) -> Result<Option<Activation>> {
    storage::load(&core.store.db(), None, true)
}

// Validation leaves ordinary scheduling and supervision available. Only the
// final handoff closes admission, after the daemon has become idle.
pub(in crate::lean) fn pending(core: &Core) -> bool {
    active(core).map_or(true, |op| {
        op.is_some_and(|op| op.state == ActivationState::Switching)
    })
}

pub(in crate::lean) fn draining(core: &Core) -> bool {
    active(core).is_ok_and(|op| {
        op.is_some_and(|op| {
            op.state == ActivationState::WaitingForIdle || op.state == ActivationState::Switching
        })
    })
}

pub(in crate::lean) fn status(state: &Path, id: Option<&str>) -> Result<Value> {
    let database = state.join("core.db");
    let operation = if database.try_exists()? {
        let db = rusqlite::Connection::open_with_flags(
            &database,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        storage::load(&db, id, false)?
    } else {
        None
    };
    let operation = if operation.is_none() && id.is_none() {
        storage::legacy(&state.join(PENDING))?.or(storage::legacy(&state.join(LAST))?)
    } else {
        operation
    };
    match operation {
        Some(op) => Ok(serde_json::to_value(op)?),
        None if id.is_some() => Err(Error::Invalid("unknown activation request ID".into())),
        None => Ok(json!({"state":"none"})),
    }
}

fn merged_path(current: Option<OsString>, requested: Option<&str>) -> Result<OsString> {
    let mut seen = BTreeSet::new();
    let paths = current.into_iter().chain(requested.map(OsString::from));
    let mut merged = Vec::new();
    for path in paths {
        for entry in std::env::split_paths(&path) {
            // Empty entries depend on the caller's cwd and are not durable.
            if entry.is_absolute() && seen.insert(entry.clone()) {
                merged.push(entry);
            }
        }
    }
    std::env::join_paths(merged).map_err(|error| Error::Invalid(error.to_string()))
}

/// Executed by the target binary without opening or modifying runtime state.
pub fn preflight(project: &Path, config: &Path, state: &Path) -> Result<Value> {
    Store::check_runtime_repository(
        &state.join("core.db"),
        &crate::lean::paths::project_identity(project)?,
    )?;
    crate::lean::doctor::preflight(&Config::load(config)?)?;
    Ok(json!({"build":build(),"project":project.canonicalize()?,"config":config.canonicalize()?}))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn request(
    core: &Arc<Core>,
    project: &Path,
    config: &Path,
    executable: &Path,
    path: Option<&str>,
    conversation: Option<&str>,
    task: Option<&str>,
    request_id: Option<&str>,
) -> Result<Value> {
    let id = request_id.map(str::to_owned).unwrap_or_else(crate::id);
    require(
        !id.is_empty() && id.len() <= 128,
        "request_id must contain 1–128 bytes",
    )?;
    // The retry signature describes the request, including the requested PATH.
    let signature = crate::hash(serde_json::to_vec(&json!([
        executable,
        project,
        config,
        path,
        conversation,
        task
    ]))?);
    let _activation = core
        .runtime_activation_lock
        .lock()
        .expect("runtime activation mutex");
    if let Some(existing) = storage::load(&core.store.db(), Some(&id), false)? {
        require(
            existing.signature == signature,
            "activation request_id conflicts with different parameters",
        )?;
        return Ok(serde_json::to_value(existing)?);
    }
    require(
        active(core)?.is_none(),
        "another runtime activation is pending",
    )?;
    let task = activation_task(core, conversation, task)?;
    let generation = task
        .as_deref()
        .map(|id| {
            core.store
                .task_by_id(id)
                .map(|t| t.map(|t| t.input_generation))
        })
        .transpose()?
        .flatten();
    let mut resume = Vec::new();
    // Recording all durable conversation links avoids acquiring activity maps
    // under the activation mutex. Paused/cancelled links remain explicit.
    for item in core.store.conversations()? {
        resume.push(Resume {
            plan: core.store.accepted_plan(&item.id)?,
            conversation: item.id,
            cancelled: false,
        });
    }
    let log = core
        .state
        .join("activation-logs")
        .join(format!("{}-preflight.stderr", crate::hash(&id)))
        .to_string_lossy()
        .into_owned();
    let operation = Activation {
        id,
        signature,
        executable: executable.canonicalize()?,
        build: build_identity(executable)?,
        project: project.canonicalize()?,
        config: config.canonicalize()?,
        path: merged_path(std::env::var_os("PATH"), path)?,
        task,
        conversation: conversation.map(str::to_owned),
        resume,
        state: ActivationState::Validating,
        error: None,
        generation,
        config_revision: crate::hash(std::fs::read(config)?),
        log: Some(log),
        diagnostic_tail: None,
    };
    save(core, &operation)?;
    announce(core, &operation)?;
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    core.runtime_activation_cancels
        .lock()
        .expect("activation cancellations")
        .insert(operation.id.clone(), cancel.clone());
    let reply = serde_json::to_value(&operation)?;
    let worker = core.clone();
    std::thread::spawn(move || validation::run(worker, operation, cancel));
    Ok(reply)
}

pub(super) fn replacement_path(
    project: &Path,
    config: &Path,
    state: &Path,
    executable: &Path,
    daemon_path: Option<&str>,
) -> Result<OsString> {
    let path = merged_path(std::env::var_os("PATH"), daemon_path)?;
    let operation = Activation {
        id: crate::id(),
        executable: executable.canonicalize()?,
        build: build_identity(executable)?,
        project: project.canonicalize()?,
        config: config.canonicalize()?,
        path: path.clone(),
        task: None,
        conversation: None,
        resume: Vec::new(),
        state: ActivationState::Validating,
        signature: String::new(),
        generation: None,
        config_revision: crate::hash(std::fs::read(config)?),
        log: None,
        diagnostic_tail: None,
        error: None,
    };
    validation::check_target(
        &operation,
        state,
        &std::sync::atomic::AtomicBool::new(false),
        None,
    )?;
    Ok(path)
}

fn activation_task(
    core: &Core,
    conversation: Option<&str>,
    task: Option<&str>,
) -> Result<Option<String>> {
    if let Some(conversation) = conversation {
        core.store.conversation(conversation)?;
    }
    let Some(task) = task else { return Ok(None) };
    let conversation =
        conversation.ok_or_else(|| Error::Invalid("--task requires --conversation".into()))?;
    let plan = core
        .store
        .accepted_plan(conversation)?
        .ok_or_else(|| Error::Invalid("no accepted plan".into()))?;
    let task = core.task_unscoped(&plan, task)?;
    let spec: TaskContract = serde_json::from_value(task.spec)?;
    require(
        !spec.change_kind.has_delivery(),
        "runtime activation must belong to a read-only task",
    )?;
    require(
        matches!(
            task.status.as_str(),
            task_status::WORKING | task_status::REVIEWING
        ),
        "runtime activation task must be running",
    )?;
    Ok(Some(task.id))
}

fn announce(core: &Core, operation: &Activation) -> Result<()> {
    for item in &operation.resume {
        core.store.append_event(
            &item.conversation,
            None,
            item.plan.as_deref(),
            "runtime_activation",
            &json!({"id":operation.id,"state":operation.state,"build":operation.build,
                "task":operation.task,"error":operation.error,
                "text":format!("Runtime activation {}{}", operation.state.name(),
                    operation.error.as_ref().map(|e| format!(": {e}")).unwrap_or_default())}),
        )?;
    }
    Ok(())
}

/// Called at the ordinary task completion boundary, before dependents unblock.
pub(in crate::lean) fn gate_approval(
    core: &Core,
    task: &mut crate::lean::store::Task,
) -> Result<bool> {
    let Some(operation) = storage::for_task(core, task)? else {
        return Ok(false);
    };
    match operation.state {
        ActivationState::Applied => return Ok(false),
        ActivationState::Failed | ActivationState::Interrupted => {
            let conversation = core.store.plan_conversation(&task.plan_id)?;
            core.stop(
                &conversation,
                task,
                task_status::FAILED,
                operation
                    .error
                    .as_deref()
                    .or(Some("runtime activation did not confirm readiness")),
            )?;
            return Ok(true);
        }
        ActivationState::Cancelled => {
            let conversation = core.store.plan_conversation(&task.plan_id)?;
            core.stop(
                &conversation,
                task,
                task_status::CANCELLED,
                Some("runtime activation cancelled"),
            )?;
            return Ok(true);
        }
        _ => {}
    }
    task.feedback.get_or_insert_with(|| json!({}))["runtime_activation"] = json!(operation.id);
    let conversation = core.store.plan_conversation(&task.plan_id)?;
    core.stop(
        &conversation,
        task,
        task_status::WAITING,
        Some("runtime activation prepared; waiting for the successor to confirm readiness"),
    )?;
    Ok(true)
}

fn release_task(
    core: &Core,
    operation: &Activation,
    state: &str,
    failed: Option<&str>,
) -> Result<()> {
    let Some(id) = &operation.task else {
        return Ok(());
    };
    let Some(mut task) = core.store.task_by_id(id)? else {
        return Ok(());
    };
    if operation.generation != Some(task.input_generation) {
        return Ok(());
    }
    if task.status == task_status::CANCELLED {
        return Ok(());
    }
    if task
        .feedback
        .as_ref()
        .and_then(|f| f["runtime_activation"].as_str())
        != Some(&operation.id)
    {
        return Ok(());
    }
    let conversation = core.store.plan_conversation(&task.plan_id)?;
    if state == "cancelled" {
        core.stop(
            &conversation,
            &mut task,
            task_status::CANCELLED,
            Some("runtime activation cancelled"),
        )?;
    } else if let Some(error) = failed {
        core.stop(&conversation, &mut task, task_status::FAILED, Some(error))?;
    } else if task.status == task_status::WAITING {
        let feedback = task.feedback.get_or_insert_with(|| json!({}));
        feedback["resume_role"] = json!("reviewer");
        feedback["current_reason"] = Value::Null;
        core.set_status(&conversation, &mut task, task_status::QUEUED)?;
    }
    Ok(())
}

fn finish(
    core: &Core,
    mut operation: Activation,
    state: ActivationState,
    error: Option<String>,
) -> Result<Activation> {
    operation.state = state;
    operation.error = error;
    release_task(core, &operation, state.name(), operation.error.as_deref())?;
    save(core, &operation)?;
    announce(core, &operation)?;
    Ok(operation)
}

pub(in crate::lean) fn recover(core: &Arc<Core>) -> Result<()> {
    validation::recover_processes(core)?;
    for operation in storage::import(core)? {
        release_task(
            core,
            &operation,
            operation.state.name(),
            operation.error.as_deref(),
        )?;
        resume(core, operation)?;
    }
    if let Some(operation) = active(core)?
        && operation.state != ActivationState::Switching
    {
        finish(
            core,
            operation,
            ActivationState::Interrupted,
            Some("runtime restarted during activation validation; request activation again".into()),
        )?;
    }
    Ok(())
}

/// Startup confirmation happens before work is allowed to resume.
pub(super) fn confirm(core: &Arc<Core>, project: &Path, config: &Path) -> Result<()> {
    let Some(mut operation) = active(core)? else {
        return Ok(());
    };
    if operation.state != ActivationState::Switching {
        let operation = finish(
            core,
            operation,
            ActivationState::Interrupted,
            Some("runtime restarted before handoff; request activation again".into()),
        )?;
        return resume(core, operation);
    }
    let result = (|| {
        require(
            operation.build == build(),
            "successor build did not match the pending activation",
        )?;
        require(
            operation.project == project && operation.config == config,
            "pending runtime activation belongs to another project or configuration",
        )?;
        validation::check_inputs(core, &operation)?;
        crate::lean::doctor::preflight(&core.config_snapshot())
    })();
    let (state, error) = match result {
        Ok(()) => (ActivationState::Applied, None),
        Err(e) => (ActivationState::Failed, Some(e.to_string())),
    };
    // Requests admitted just before the switch can commit after the handoff
    // snapshot. Reconcile durable conversations before enabling successor work.
    for conversation in core.store.conversations()? {
        if !operation
            .resume
            .iter()
            .any(|item| item.conversation == conversation.id)
        {
            operation.resume.push(Resume {
                plan: core.store.accepted_plan(&conversation.id)?,
                conversation: conversation.id,
                cancelled: false,
            });
        }
    }
    let operation = finish(core, operation, state, error)?;
    core.replay_delivered_answers()?;
    resume(core, operation)
}

fn resume(core: &Arc<Core>, operation: Activation) -> Result<()> {
    for item in operation.resume {
        if item.cancelled || core.store.paused(&item.conversation)? {
            continue;
        }
        {
            if let Some(plan) = item.plan
                && core.store.accepted_plan(&item.conversation)?.as_deref() == Some(&plan)
                && core.has_runnable_dependents(&plan)?
                && let Err(error) = core.maybe_continue_plan(&item.conversation, &plan)
            {
                core.notice(
                    &item.conversation,
                    &format!("runtime activated; plan could not resume: {error}"),
                );
            }
        }
        core.wake(&item.conversation);
        if let Err(error) = core.resume_agent_reviews(&item.conversation) {
            core.notice(
                &item.conversation,
                &format!("runtime activation settled; pending reviews could not resume: {error}"),
            );
        }
    }
    Ok(())
}

/// Runs in the daemon's lifecycle thread; no helper service survives activation.
pub(super) fn apply_when_idle(core: &Arc<Core>) -> Result<()> {
    if core.busy() || core.state.join("stopped").exists() {
        return Ok(());
    }
    let graph = core.locks.graph();
    let integration = core.locks.integration();
    let session = core
        .locks
        .session_lifecycle
        .lock()
        .expect("session lifecycle mutex");
    let activation = core
        .runtime_activation_lock
        .lock()
        .expect("runtime activation mutex");
    let Some(mut operation) = active(core)? else {
        return Ok(());
    };
    if operation.state != ActivationState::WaitingForIdle
        || core.busy()
        || core.state.join("stopped").exists()
    {
        return Ok(());
    }
    if let Err(e) = validation::check_inputs(core, &operation) {
        finish(
            core,
            operation,
            ActivationState::Failed,
            Some(e.to_string()),
        )?;
        return Ok(());
    }
    for conversation in core.store.conversations()? {
        if !operation
            .resume
            .iter()
            .any(|item| item.conversation == conversation.id)
            && core.unanswered(&conversation.id)?.is_some()
        {
            operation.resume.push(Resume {
                plan: core.store.accepted_plan(&conversation.id)?,
                conversation: conversation.id,
                cancelled: false,
            });
        }
    }
    operation.state = ActivationState::Switching;
    save(core, &operation)?;
    drop(activation);
    drop(session);
    drop(integration);
    drop(graph);
    // All process operations, including shutdown, happen outside global locks.
    let result = (|| -> Result<()> {
        core.native.shutdown()?;
        use std::os::unix::process::CommandExt;
        let error = std::process::Command::new(&operation.executable)
            .arg("__core")
            .arg(&operation.project)
            .arg(&operation.config)
            .arg(&core.state)
            .env("PATH", &operation.path)
            .current_dir(&operation.project)
            .exec();
        Err(error.into())
    })();
    if let Err(error) = result {
        let operation = finish(
            core,
            operation,
            ActivationState::Failed,
            Some(error.to_string()),
        )?;
        resume(core, operation)?;
        return Err(error);
    }
    Ok(())
}

pub(in crate::lean) fn cancel_operation(core: &Core, id: &str) -> Result<Value> {
    // Signal before taking the operation mutex, even while a validator exits.
    if let Some(flag) = core
        .runtime_activation_cancels
        .lock()
        .expect("activation cancellations")
        .get(id)
    {
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    let _guard = core
        .runtime_activation_lock
        .lock()
        .expect("runtime activation mutex");
    let operation = storage::load(&core.store.db(), Some(id), false)?
        .ok_or_else(|| Error::Invalid("unknown activation request ID".into()))?;
    require(
        operation.state != ActivationState::Switching,
        "runtime handoff has started; inspect successor status",
    )?;
    let operation = if matches!(
        operation.state,
        ActivationState::Validating | ActivationState::WaitingForIdle
    ) {
        finish(core, operation, ActivationState::Cancelled, None)?
    } else {
        operation
    };
    Ok(serde_json::to_value(operation)?)
}

pub(in crate::lean) fn cancel_conversation(core: &Core, conversation: &str) -> Result<()> {
    let Some(operation) = active(core)? else {
        return Ok(());
    };
    if operation.conversation.as_deref() == Some(conversation) {
        cancel_operation(core, &operation.id)?;
    } else {
        core.store.db().execute("UPDATE runtime_activation_conversations SET cancelled=1 WHERE operation=?1 AND conversation=?2",rusqlite::params![operation.id,conversation])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activation_holds_approval_until_confirmation_and_preserves_failure_or_cancellation() {
        for (error, cancelled) in [
            (None, false),
            (Some("successor preflight failed"), false),
            (None, true),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let config =
                Config::from_toml(include_str!("../../../examples/antislop.toml")).unwrap();
            let core = Core::open(&dir.path().join("state"), config).unwrap();
            let conversation = core
                .store
                .create_conversation(dir.path().to_str().unwrap())
                .unwrap()
                .id;
            let tasks = core.store.add_plan(&conversation, &[
                json!({"id":"activate","title":"Activate","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Run successor","change_kind":"none","kind":"delivery"}),
                json!({"id":"next","title":"Continue","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Use successor","change_kind":"none","kind":"analysis","depends_on":["activate"]}),
            ]).unwrap();
            let mut task = tasks[0].clone();
            task.report = Some("prepared activation; retained report".into());
            task.verdict = Some(json!({"approved":true,"findings":[]}));
            let operation = Activation {
                id: crate::id(),
                executable: std::env::current_exe().unwrap(),
                build: build().into(),
                project: dir.path().into(),
                config: dir.path().join("config"),
                path: OsString::from("/usr/bin:/bin"),
                task: Some(task.id.clone()),
                conversation: Some(conversation.clone()),
                resume: vec![Resume {
                    conversation: conversation.clone(),
                    plan: Some(task.plan_id.clone()),
                    cancelled: false,
                }],
                state: ActivationState::WaitingForIdle,
                signature: String::new(),
                generation: Some(task.input_generation),
                config_revision: String::new(),
                log: None,
                diagnostic_tail: None,
                error: None,
            };
            save(&core, &operation).unwrap();
            core.set_status(&conversation, &mut task, task_status::APPROVED)
                .unwrap();
            assert_eq!(task.status, task_status::WAITING);
            assert!(
                task.verdict.as_ref().unwrap()["approved"]
                    .as_bool()
                    .unwrap()
            );
            assert!(!core.has_runnable_dependents(&task.plan_id).unwrap());
            if cancelled {
                core.cancel(&conversation);
                core.set_status(&conversation, &mut task, task_status::CANCELLED)
                    .unwrap();
                apply_when_idle(&core).unwrap();
            } else {
                finish(
                    &core,
                    operation,
                    if error.is_some() {
                        ActivationState::Failed
                    } else {
                        ActivationState::Applied
                    },
                    error.map(str::to_owned),
                )
                .unwrap();
            }
            let mut restored = core.task(&task.plan_id, &task.id).unwrap();
            assert_eq!(restored.report, task.report);
            assert!(active(&core).unwrap().is_none());
            assert_eq!(
                status(&core.state, None).unwrap()["state"],
                if cancelled {
                    "cancelled"
                } else if error.is_some() {
                    "failed"
                } else {
                    "applied"
                }
            );
            if error.is_some() || cancelled {
                let expected = restored.status.clone();
                core.set_status(&conversation, &mut restored, task_status::APPROVED)
                    .unwrap();
                assert_eq!(
                    restored.status, expected,
                    "terminal activation cannot earn approval"
                );
            }
            if cancelled {
                assert_eq!(restored.status, task_status::CANCELLED);
            } else if let Some(error) = error {
                assert_eq!(restored.status, task_status::FAILED);
                assert_eq!(restored.feedback.as_ref().unwrap()["reason"], error);
            } else {
                assert_eq!(restored.status, task_status::QUEUED);
                assert_eq!(
                    restored.feedback.as_ref().unwrap()["resume_role"],
                    "reviewer"
                );
                core.set_status(&conversation, &mut restored, task_status::APPROVED)
                    .unwrap();
                assert_eq!(restored.status, task_status::APPROVED);
                let mut next = tasks[1].clone();
                core.set_status(&conversation, &mut next, task_status::QUEUED)
                    .unwrap();
                assert!(core.has_runnable_dependents(&task.plan_id).unwrap());
            }
        }
    }
}

#[cfg(test)]
mod operation_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn fixture() -> (tempfile::TempDir, Arc<Core>, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("antislop.toml");
        std::fs::write(&config, include_str!("../../../examples/antislop.toml")).unwrap();
        let core = Core::open(&dir.path().join("state"), Config::load(&config).unwrap()).unwrap();
        let binary = dir.path().join("successor");
        std::fs::write(&binary,"#!/bin/sh\nif [ \"$1\" = --version ]; then cat version.json; exit; fi\nif [ \"$1\" = __runtime_preflight ]; then touch started; while [ -e hold ]; do sleep 0.02; done; cat preflight.json; exit; fi\necho UNEXPECTED >> unexpected\nexit 1\n").unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        let build = build_identity(&binary).unwrap();
        std::fs::write(dir.path().join("version.json"),json!({"name":"antislop","build":build,"protocol":PROTOCOL_VERSION,"capabilities":["runtime_preflight"]}).to_string()).unwrap();
        std::fs::write(dir.path().join("preflight.json"),json!({"build":build,"project":dir.path().canonicalize().unwrap(),"config":config.canonicalize().unwrap()}).to_string()).unwrap();
        (dir, core, config, binary)
    }

    fn wait_for(mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !condition() {
            assert!(
                Instant::now() < deadline,
                "activation worker did not progress"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn held_preflight_keeps_status_retries_and_manual_cancellation_responsive() {
        let (dir, core, config, binary) = fixture();
        std::fs::write(dir.path().join("hold"), []).unwrap();
        let send = |id, path| {
            request(
                &core,
                dir.path(),
                &config,
                &binary,
                path,
                None,
                None,
                Some(id),
            )
        };
        assert_eq!(send("held", None).unwrap()["state"], "validating");
        wait_for(|| dir.path().join("started").exists());
        assert_eq!(
            status(&core.state, Some("held")).unwrap()["state"],
            "validating"
        );
        assert!(!pending(&core));
        assert!(!draining(&core));
        assert_eq!(send("held", None).unwrap()["id"], "held");
        assert!(
            send("held", Some("/different"))
                .unwrap_err()
                .to_string()
                .contains("conflicts")
        );
        assert_eq!(
            cancel_operation(&core, "held").unwrap()["state"],
            "cancelled"
        );
        wait_for(|| core.runtime_activation_cancels.lock().unwrap().is_empty());
        assert_eq!(
            status(&core.state, Some("held")).unwrap()["state"],
            "cancelled"
        );
        assert!(!dir.path().join("unexpected").exists());
        // Read-only status is available without a daemon and creates no tables.
        let empty = dir.path().join("empty");
        std::fs::create_dir(&empty).unwrap();
        assert_eq!(status(&empty, None).unwrap()["state"], "none");
        assert!(!empty.join("core.db").exists());
    }

    #[test]
    fn restart_ends_the_recorded_validator_and_interrupts_its_operation() {
        let (dir, core, config, binary) = fixture();
        std::fs::write(dir.path().join("hold"), []).unwrap();
        request(
            &core,
            dir.path(),
            &config,
            &binary,
            None,
            None,
            None,
            Some("restart"),
        )
        .unwrap();
        wait_for(|| dir.path().join("started").exists());
        assert!(core.busy());
        let recorded: i64 = core
            .store
            .db()
            .query_row(
                "SELECT count(*) FROM runtime_validation_processes WHERE operation='restart'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(recorded, 1);
        validation::recover_processes(&core).unwrap();
        wait_for(|| core.runtime_activation_cancels.lock().unwrap().is_empty());
        // A real restart has no old worker thread racing the recovery write.
        let mut operation = storage::load(&core.store.db(), Some("restart"), false)
            .unwrap()
            .unwrap();
        operation.state = ActivationState::Validating;
        save(&core, &operation).unwrap();
        recover(&core).unwrap();
        assert_eq!(
            status(&core.state, Some("restart")).unwrap()["state"],
            "interrupted"
        );
        assert!(!core.busy());
        // Explicit shutdown cancellation also covers unscoped operations.
        std::fs::remove_file(dir.path().join("started")).unwrap();
        request(
            &core,
            dir.path(),
            &config,
            &binary,
            None,
            None,
            None,
            Some("shutdown"),
        )
        .unwrap();
        wait_for(|| dir.path().join("started").exists());
        cancel_all(&core).unwrap();
        wait_for(|| core.runtime_activation_cancels.lock().unwrap().is_empty());
        assert_eq!(
            status(&core.state, Some("shutdown")).unwrap()["state"],
            "cancelled"
        );
    }

    #[test]
    fn old_version_and_missing_capability_never_reach_an_internal_command() {
        for metadata in [
            "antislop 0.1.0",
            "{\"name\":\"antislop\",\"capabilities\":[]}",
        ] {
            let (dir, core, config, binary) = fixture();
            std::fs::write(dir.path().join("version.json"), metadata).unwrap();
            request(
                &core,
                dir.path(),
                &config,
                &binary,
                None,
                None,
                None,
                Some("old"),
            )
            .unwrap();
            wait_for(|| status(&core.state, Some("old")).unwrap()["state"] == "failed");
            assert!(!dir.path().join("started").exists());
            assert!(!dir.path().join("unexpected").exists());
            assert!(
                status(&core.state, Some("old")).unwrap()["error"]
                    .as_str()
                    .unwrap()
                    .contains("rebuild")
            );
        }
    }

    #[test]
    fn changed_validation_inputs_and_restart_cannot_earn_readiness() {
        let (dir, core, config, binary) = fixture();
        std::fs::write(dir.path().join("hold"), []).unwrap();
        request(
            &core,
            dir.path(),
            &config,
            &binary,
            None,
            None,
            None,
            Some("changed"),
        )
        .unwrap();
        wait_for(|| dir.path().join("started").exists());
        std::fs::write(
            &config,
            format!("{}\n# changed\n", std::fs::read_to_string(&config).unwrap()),
        )
        .unwrap();
        std::fs::remove_file(dir.path().join("hold")).unwrap();
        wait_for(|| status(&core.state, Some("changed")).unwrap()["state"] == "failed");
        assert!(
            status(&core.state, Some("changed")).unwrap()["error"]
                .as_str()
                .unwrap()
                .contains("configuration changed")
        );
        for state in [ActivationState::Validating, ActivationState::WaitingForIdle] {
            let mut operation = storage::load(&core.store.db(), Some("changed"), false)
                .unwrap()
                .unwrap();
            operation.state = state;
            save(&core, &operation).unwrap();
            recover(&core).unwrap();
            assert_eq!(
                status(&core.state, Some("changed")).unwrap()["state"],
                "interrupted"
            );
        }
        // Pending mail must survive startup without registering a review
        // worker before the successor has settled its readiness fence.
        let conversation = core
            .store
            .create_conversation(dir.path().to_str().unwrap())
            .unwrap()
            .id;
        core.store.db().execute(
            "INSERT INTO mail(conversation_id,sender,recipient,kind,text,created_ms) VALUES(?1,'orchestrator','worker:retained','guidance','Retained guidance',0)",
            [&conversation],
        ).unwrap();
        let mut operation = active(&core).unwrap().unwrap_or_else(|| {
            storage::load(&core.store.db(), Some("changed"), false)
                .unwrap()
                .unwrap()
        });
        operation.state = ActivationState::Switching;
        operation.resume.push(Resume {
            conversation: conversation.clone(),
            plan: None,
            cancelled: false,
        });
        save(&core, &operation).unwrap();
        wait_for(|| core.runtime_activation_cancels.lock().unwrap().is_empty());
        let state = core.state.clone();
        drop(core);
        let restored = Core::open(&state, Config::load(&config).unwrap()).unwrap();
        assert!(pending(&restored));
        assert!(!restored.busy());
        assert_eq!(
            restored
                .store
                .pending_mail(&conversation, "worker:retained")
                .unwrap()
                .len(),
            1
        );
        // The fixture executable differs from this running test executable:
        // confirmation must settle that failure before resuming mail work.
        confirm(&restored, dir.path(), &config).unwrap();
        assert!(!pending(&restored));
        assert_eq!(status(&state, Some("changed")).unwrap()["state"], "failed");
        wait_for(|| !restored.activity.agent_review_running(&conversation));
        assert_eq!(
            restored
                .store
                .pending_mail(&conversation, "worker:retained")
                .unwrap()
                .len(),
            1
        );
        drop(restored);
        std::fs::write(state.join("stopped"), []).unwrap();
        let stopped = Core::open(&state, Config::load(&config).unwrap()).unwrap();
        assert!(!stopped.busy());
        assert_eq!(
            stopped
                .store
                .pending_mail(&conversation, "worker:retained")
                .unwrap()
                .len(),
            1
        );
    }
}

pub(in crate::lean) fn cancel_all(core: &Core) -> Result<()> {
    for flag in core
        .runtime_activation_cancels
        .lock()
        .expect("activation cancellations")
        .values()
    {
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    if let Some(operation) = active(core)?
        && operation.state != ActivationState::Switching
    {
        cancel_operation(core, &operation.id)?;
    }
    Ok(())
}

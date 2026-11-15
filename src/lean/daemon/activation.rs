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
    state: String,
    error: Option<String>,
}

pub(in crate::lean) fn pending(core: &Core) -> bool {
    core.state.join(PENDING).exists()
}

fn load(path: &Path) -> Result<Option<Activation>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn save(state: &Path, name: &str, operation: &Activation) -> Result<()> {
    super::config_write::atomic_config_write(&state.join(name), &serde_json::to_string(operation)?)
}

pub(in crate::lean) fn status(state: &Path) -> Result<Value> {
    Ok(
        match load(&state.join(PENDING))?.or(load(&state.join(LAST))?) {
            Some(operation) => serde_json::to_value(operation)?,
            None => json!({"state":"none"}),
        },
    )
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

fn check_target(operation: &Activation, state: &Path) -> Result<()> {
    require(
        build_identity(&operation.executable)? == operation.build,
        "activation executable changed; request activation again",
    )?;
    let output = std::process::Command::new(&operation.executable)
        .arg("__runtime_preflight")
        .arg(&operation.project)
        .arg(&operation.config)
        .arg(state)
        .current_dir(&operation.project)
        .env("PATH", &operation.path)
        .stdin(std::process::Stdio::null())
        .output()?;
    require(
        output.status.success(),
        &format!(
            "runtime activation preflight failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    )?;
    let checked: Value = serde_json::from_slice(&output.stdout)?;
    require(
        checked["build"] == operation.build
            && checked["project"] == operation.project.to_string_lossy().as_ref()
            && checked["config"] == operation.config.to_string_lossy().as_ref(),
        "runtime activation preflight returned a different build or context",
    )
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
) -> Result<Value> {
    let _graph = core.locks.graph();
    let _integration = core.locks.integration();
    let _activation = core
        .runtime_activation_lock
        .lock()
        .expect("runtime activation mutex");
    let executable = executable.canonicalize()?;
    let expected = build_identity(&executable)?;
    let task = activation_task(core, conversation, task)?;
    if let Some(existing) = load(&core.state.join(PENDING))? {
        require(
            existing.build == expected && existing.task == task,
            "another runtime activation is pending",
        )?;
        return Ok(serde_json::to_value(existing)?);
    }
    let mut resume = Vec::new();
    for item in core.store.conversations()? {
        if !core.is_idle(&item.id) || conversation == Some(item.id.as_str()) {
            resume.push(Resume {
                plan: core.store.accepted_plan(&item.id)?,
                conversation: item.id,
                cancelled: false,
            });
        }
    }
    let mut operation = Activation {
        id: crate::id(),
        executable,
        build: expected,
        project: project.canonicalize()?,
        config: config.canonicalize()?,
        path: merged_path(std::env::var_os("PATH"), path)?,
        task,
        conversation: conversation.map(str::to_owned),
        resume,
        state: "pending".into(),
        error: None,
    };
    if let Err(error) = check_target(&operation, &core.state) {
        operation.state = "failed".into();
        operation.error = Some(error.to_string());
        save(&core.state, LAST, &operation)?;
        announce(core, &operation)?;
        return Err(error);
    }
    if operation.build == build() && crate::lean::doctor::preflight(&core.config_snapshot()).is_ok()
    {
        // A matching build is ready only when its actual launch environment works.
        operation.state = "applied".into();
        save(&core.state, LAST, &operation)?;
    } else {
        save(&core.state, PENDING, &operation)?;
    }
    announce(core, &operation)?;
    Ok(serde_json::to_value(operation)?)
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
        state: "pending".into(),
        error: None,
    };
    check_target(&operation, state)?;
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
    let task = core.task(&plan, task)?;
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
                "text":format!("Runtime activation {}{}", operation.state,
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
    let Some(operation) = load(&core.state.join(PENDING))? else {
        return Ok(false);
    };
    if operation.task.as_deref() != Some(&task.id) {
        return Ok(false);
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
        feedback["reason"] = Value::Null;
        core.set_status(&conversation, &mut task, task_status::QUEUED)?;
    }
    Ok(())
}

fn finish(
    core: &Core,
    mut operation: Activation,
    state: &str,
    error: Option<String>,
) -> Result<Activation> {
    operation.state = state.into();
    operation.error = error;
    release_task(core, &operation, state, operation.error.as_deref())?;
    save(&core.state, LAST, &operation)?;
    announce(core, &operation)?;
    std::fs::remove_file(core.state.join(PENDING))?;
    std::fs::File::open(&core.state)?.sync_all()?;
    Ok(operation)
}

/// Startup confirmation happens before work is allowed to resume.
pub(super) fn confirm(core: &Arc<Core>, project: &Path, config: &Path) -> Result<()> {
    let Some(operation) = load(&core.state.join(PENDING))? else {
        return Ok(());
    };
    if operation.state == "cancelled" {
        let operation = finish(core, operation, "cancelled", None)?;
        return resume(core, operation);
    }
    if operation.build != build() {
        return Ok(());
    }
    require(
        operation.project == project && operation.config == config,
        "pending runtime activation belongs to another project or configuration",
    )?;
    crate::lean::doctor::preflight(&core.config_snapshot())?;
    let operation = finish(core, operation, "applied", None)?;
    resume(core, operation)
}

fn resume(core: &Arc<Core>, operation: Activation) -> Result<()> {
    for item in operation.resume {
        if item.cancelled || core.store.paused(&item.conversation)? {
            continue;
        }
        {
            let _graph = core.locks.graph();
            let _integration = core.locks.integration();
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
    }
    Ok(())
}

/// Runs in the daemon's lifecycle thread; no helper service survives activation.
pub(super) fn apply_when_idle(core: &Arc<Core>) -> Result<()> {
    if core.busy() || core.state.join("stopped").exists() {
        return Ok(());
    }
    let _graph = core.locks.graph();
    let _integration = core.locks.integration();
    // A send accepted just before activation may still be recording its text.
    // Serialize its commit/wake with the final durable handoff snapshot.
    let _session = core
        .locks
        .session_lifecycle
        .lock()
        .expect("session lifecycle mutex");
    let _activation = core
        .runtime_activation_lock
        .lock()
        .expect("runtime activation mutex");
    if core.busy() || core.state.join("stopped").exists() {
        return Ok(());
    }
    let Some(mut operation) = load(&core.state.join(PENDING))? else {
        return Ok(());
    };
    if operation.state == "cancelled" {
        let operation = finish(core, operation, "cancelled", None)?;
        drop(_activation);
        drop(_session);
        drop(_integration);
        drop(_graph);
        return resume(core, operation);
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
    save(&core.state, PENDING, &operation)?;
    let result = (|| -> Result<()> {
        check_target(&operation, &core.state)?;
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
        let operation = finish(core, operation, "failed", Some(error.to_string()))?;
        drop(_activation);
        drop(_session);
        drop(_integration);
        drop(_graph);
        resume(core, operation)?;
        return Err(error);
    }
    Ok(())
}

pub(in crate::lean) fn cancel_conversation(core: &Core, conversation: &str) -> Result<()> {
    let _activation = core
        .runtime_activation_lock
        .lock()
        .expect("runtime activation mutex");
    let Some(mut operation) = load(&core.state.join(PENDING))? else {
        return Ok(());
    };
    for item in &mut operation.resume {
        if item.conversation == conversation {
            item.cancelled = true;
        }
    }
    if operation.conversation.as_deref() == Some(conversation) {
        operation.state = "cancelled".into();
    }
    save(&core.state, PENDING, &operation)
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
                json!({"id":"activate","title":"Activate","goal":"Run successor","change_kind":"none","kind":"delivery"}),
                json!({"id":"next","title":"Continue","goal":"Use successor","change_kind":"none","kind":"analysis","depends_on":["activate"]}),
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
                state: "pending".into(),
                error: None,
            };
            save(&core.state, PENDING, &operation).unwrap();
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
                    if error.is_some() { "failed" } else { "applied" },
                    error.map(str::to_owned),
                )
                .unwrap();
            }
            let mut restored = core.task(&task.plan_id, &task.id).unwrap();
            assert_eq!(restored.report, task.report);
            assert!(!pending(&core));
            assert_eq!(
                status(&core.state).unwrap()["state"],
                if cancelled {
                    "cancelled"
                } else if error.is_some() {
                    "failed"
                } else {
                    "applied"
                }
            );
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

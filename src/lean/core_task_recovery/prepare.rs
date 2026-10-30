use super::*;

pub(super) fn load(core: &Core, req: TaskRecoveryRequest, actor: &str) -> Result<RecoveryWork> {
    let config = core.config_snapshot();
    let conv = core.store.conversation(&req.conversation)?;
    let project = PathBuf::from(conv.project);
    let task = core.task(&req.plan, &req.task)?;
    let spec = serde_json::from_value(task.spec.clone())?;
    core.store.assert_task_inputs(&task)?;
    let token = core.task_freshness_token(&req.conversation, &req.plan, &task.id)?;
    validate_request(&req, &task, &token)?;
    Ok(RecoveryWork {
        req,
        actor: actor.into(),
        project,
        config,
        spec,
        task,
        token,
        integrated_files: Vec::new(),
        preserved_branch: None,
        notice: None,
    })
}

fn validate_request(req: &TaskRecoveryRequest, task: &Task, token: &str) -> Result<()> {
    require(req.freshness_token == token, "stale_token")?;
    require(
        allowed_status(&req.action, &task.status),
        "task is not waiting for recovery",
    )
}

fn allowed_status(action: &RecoveryAction, status: &str) -> bool {
    is_running_action(action)
        && matches!(status, status::QUEUED | status::WORKING | status::VERIFYING)
        || matches!(
            status,
            status::COORDINATING
                | status::NEEDS_USER
                | status::WAITING
                | status::FAILED
                | status::SKIPPED
                | status::CANCELLED
        )
}

fn is_running_action(action: &RecoveryAction) -> bool {
    matches!(
        action,
        RecoveryAction::RestartAgent | RecoveryAction::CancelTask
    )
}

pub(super) fn interrupt_if_active(
    core: &Core,
    work: &mut RecoveryWork,
) -> Option<TaskRecoveryResponse> {
    if !is_running_action(&work.req.action) {
        return None;
    }
    let active = core.task_cancels.lock().expect("task cancel mutex");
    let (_, cancel) = active.get(&work.task.id)?;
    queue_restart(core, work);
    cancel.store(true, Ordering::SeqCst);
    Some(response::interrupted(work))
}

fn queue_restart(core: &Core, work: &RecoveryWork) {
    core.task_restarts
        .lock()
        .expect("task restart mutex")
        .insert(work.task.id.clone(), work.req.clone());
}

use super::*;

pub(super) fn apply(core: &Arc<Core>, work: &mut RecoveryWork) -> Result<()> {
    match &work.req.action {
        RecoveryAction::IntegrateWithConsent => integrate::apply(core, work),
        RecoveryAction::VerifyManualSnapshot => manual::apply(core, work),
        RecoveryAction::KeepOnBranch => keep_branch(core, work),
        RecoveryAction::RetryWithGuidance => retry(core, work),
        RecoveryAction::RestartAgent => restart(core, work),
        RecoveryAction::CancelTask => cancel(core, work),
    }
}

fn keep_branch(core: &Core, work: &mut RecoveryWork) -> Result<()> {
    let branch = worktree_branch(&core.state, &work.task)?;
    work.preserved_branch = Some(branch.clone());
    let notice = format!("change preserved on branch {branch}; working tree untouched");
    work.task.status = status::NEEDS_USER.into();
    land(
        &mut work.task,
        Outcome::LeftOnBranch,
        "recovery",
        &notice,
        None,
        Some(&branch),
        &[],
    );
    let path = worktree_path(&core.state, &work.task)?;
    let _ = git::remove_worktree_keep_branch(&work.project, &path);
    note(&mut work.task, notice.clone());
    persist_simple(
        core,
        work,
        &notice,
        tasks::action::KEEP_ON_BRANCH,
        Some(&branch),
    )
}

fn retry(core: &Arc<Core>, work: &mut RecoveryWork) -> Result<()> {
    queue_guidance(core, work)?;
    work.task.status = status::QUEUED.into();
    work.task.verdict = None;
    let notice = "retrying task with guidance".to_string();
    work.task.feedback = None;
    core.store.update_task(&work.task)?;
    core.announce_task(&work.req.conversation, &work.task, None);
    retry_event(core, work)?;
    core.maybe_continue_plan(&work.req.conversation, &work.req.plan)?;
    work.notice = Some(notice);
    Ok(())
}

fn restart(core: &Arc<Core>, work: &mut RecoveryWork) -> Result<()> {
    core.store.forget_task_sessions(&work.task.id)?;
    queue_guidance(core, work)?;
    work.task.status = status::QUEUED.into();
    work.task.verdict = None;
    work.task.feedback = None;
    let notice = "restarting the task's agent in a fresh session".to_string();
    core.store.update_task(&work.task)?;
    core.announce_task(&work.req.conversation, &work.task, None);
    restart_event(core, work)?;
    core.maybe_continue_plan(&work.req.conversation, &work.req.plan)?;
    work.notice = Some(notice);
    Ok(())
}

fn cancel(core: &Core, work: &mut RecoveryWork) -> Result<()> {
    work.task.status = status::CANCELLED.into();
    work.task.verdict = None;
    let notice = "task cancelled; its outstanding requirements remain unsatisfied".to_string();
    note(&mut work.task, notice.clone());
    core.store.update_task(&work.task)?;
    core.announce_task(&work.req.conversation, &work.task, Some(&notice));
    work.notice = Some(notice);
    Ok(())
}

fn queue_guidance(core: &Core, work: &RecoveryWork) -> Result<()> {
    let Some(message) = &work.req.message else {
        return Ok(());
    };
    let role = format!("worker:{}", work.task.id);
    let mail = Mail {
        id: 0,
        created_ms: 0,
        delivered_turn: None,
        delivered_ms: None,
        sender: RUNTIME.into(),
        recipient: role.clone(),
        kind: "guidance".into(),
        text: message.clone(),
        state: "queued".into(),
    };
    core.store
        .add_mail(&work.req.conversation, &role, &mail)
        .map(|_| ())
}

fn persist_simple(
    core: &Core,
    work: &mut RecoveryWork,
    notice: &str,
    action: &str,
    branch: Option<&str>,
) -> Result<()> {
    core.store.update_task(&work.task)?;
    core.announce_task(&work.req.conversation, &work.task, None);
    let payload = recovery_payload(work, action, notice, branch);
    core.store.append_event(
        &work.req.conversation,
        None,
        Some(&work.req.plan),
        "task_recovery",
        &payload,
    )?;
    work.notice = Some(notice.into());
    Ok(())
}

fn recovery_payload(
    work: &RecoveryWork,
    action: &str,
    notice: &str,
    branch: Option<&str>,
) -> serde_json::Value {
    let mut payload = json!({ "actor": work.actor, "task": work.task.id,
        "action": action, "status": work.task.status,
        "detail": tasks::bound(notice, tasks::DETAIL_LIMIT) });
    if let Some(branch) = branch {
        payload["branch"] = json!(branch);
    }
    payload
}

fn retry_event(core: &Core, work: &RecoveryWork) -> Result<()> {
    let message = work
        .req
        .message
        .as_deref()
        .map(|m| tasks::bound(m, tasks::DETAIL_LIMIT));
    let payload = json!({ "actor": work.actor, "task": work.task.id,
        "action": tasks::action::RETRY_WITH_GUIDANCE, "status": work.task.status, "message": message });
    core.store.append_event(
        &work.req.conversation,
        None,
        Some(&work.req.plan),
        "task_recovery",
        &payload,
    )?;
    Ok(())
}

fn restart_event(core: &Core, work: &RecoveryWork) -> Result<()> {
    let payload = json!({ "actor": work.actor, "task": work.task.id,
        "action": tasks::action::RESTART_AGENT, "status": work.task.status });
    core.store.append_event(
        &work.req.conversation,
        None,
        Some(&work.req.plan),
        "task_recovery",
        &payload,
    )?;
    Ok(())
}

use super::*;

pub(super) fn interrupted(work: &RecoveryWork) -> TaskRecoveryResponse {
    TaskRecoveryResponse {
        task: work.task.id.clone(),
        status: work.task.status.clone(),
        round: work.task.round,
        integrated_files: Vec::new(),
        preserved_branch: None,
        new_freshness_token: work.token.clone(),
        notice: "stopping the current round before restarting the agent".into(),
    }
}

pub(super) fn build(core: &Core, work: RecoveryWork) -> Result<TaskRecoveryResponse> {
    let token = core.task_freshness_token(&work.req.conversation, &work.req.plan, &work.task.id)?;
    Ok(TaskRecoveryResponse {
        task: work.task.id,
        status: work.task.status,
        round: work.task.round,
        integrated_files: work.integrated_files,
        preserved_branch: work.preserved_branch,
        new_freshness_token: token,
        notice: work.notice.unwrap_or_default(),
    })
}

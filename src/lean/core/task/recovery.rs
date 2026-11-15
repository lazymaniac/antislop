use crate::lean::core::task::feedback_helpers::{event_files, land, note};
use crate::lean::core::worktree::paths::worktree_branch;
use crate::lean::*;

mod actions;
mod events;
mod integrate;
mod manual;
mod orchestrator_retry;
mod prepare;
mod response;
#[cfg(test)]
mod test_support;

pub(in crate::lean) struct RecoveryWork {
    pub(in crate::lean) req: TaskRecoveryRequest,
    pub(in crate::lean) actor: String,
    pub(in crate::lean) project: PathBuf,
    pub(in crate::lean) config: Arc<Config>,
    pub(in crate::lean) spec: TaskContract,
    pub(in crate::lean) task: Task,
    pub(in crate::lean) token: String,
    pub(in crate::lean) integrated_files: Vec<String>,
    pub(in crate::lean) preserved_branch: Option<String>,
    pub(in crate::lean) notice: Option<String>,
}

impl Core {
    pub(in crate::lean) fn recover_task_as(
        self: &Arc<Self>,
        req: TaskRecoveryRequest,
        actor: &str,
    ) -> Result<TaskRecoveryResponse> {
        let _graph = self.locks.graph();
        let _integration = self.locks.integration();
        let mut work = prepare::load(self, req, actor)?;
        if let Some(response) = prepare::interrupt_if_active(self, &mut work) {
            return Ok(response);
        }
        if let Err(error) = actions::apply(self, &mut work) {
            if !matches!(
                work.req.action,
                RecoveryAction::RestartAgent | RecoveryAction::RetryWithGuidance
            ) {
                return Err(error);
            }
            let reason = error.to_string();
            // Reload: an error may have happened after a partial state change.
            let mut task = self.task(&work.req.plan, &work.task.id)?;
            self.stop(
                &work.req.conversation,
                &mut task,
                status::FAILED,
                Some(&reason),
            )?;
            self.store.append_event(
                &work.req.conversation,
                None,
                Some(&work.req.plan),
                "task_recovery_failed",
                &json!({"task": task.id, "action": work.req.action, "actor": actor, "reason": reason}),
            )?;
            return Err(error);
        }
        response::build(self, work)
    }
}

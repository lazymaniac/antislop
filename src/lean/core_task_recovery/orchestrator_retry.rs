//! The orchestrator's `retry_task`: a task that failed for a reason outside
//! its contract (a turn limit, a crashed CLI) runs again in its own worktree,
//! without waiting for the user. A stalled task needs a new basis instead.
use super::*;

/// Retries the orchestrator may start for one task; then the user decides.
const MAX_ORCHESTRATOR_RETRIES: i64 = 2;

impl Core {
    pub(in crate::lean) fn retry_failed_task(
        self: &Arc<Self>,
        conversation: &str,
        name: &str,
        guidance: &str,
    ) -> Result<serde_json::Value> {
        let request = self.retry_request(conversation, name, guidance)?;
        self.recover_task_as(request, "orchestrator")?;
        Ok(json!({"retried": name}))
    }

    fn retry_request(
        &self,
        conversation: &str,
        name: &str,
        guidance: &str,
    ) -> Result<TaskRecoveryRequest> {
        let (plan, task) = self.failed_task(conversation, name)?;
        require(
            task.status == status::FAILED,
            "task: only a failed task can be retried; repair a stalled one with revise_plan",
        )?;
        require(
            self.orchestrator_retries(conversation, &task.id)? < MAX_ORCHESTRATOR_RETRIES,
            "task: retried twice already; ask the user with ask_user",
        )?;
        Ok(TaskRecoveryRequest {
            conversation: conversation.into(),
            plan: plan.clone(),
            freshness_token: self.task_freshness_token(conversation, &plan, &task.id)?,
            task: task.id,
            action: RecoveryAction::RetryWithGuidance,
            consent_files: None,
            message: Some(guidance.into()),
        })
    }

    fn failed_task(&self, conversation: &str, name: &str) -> Result<(String, Task)> {
        let plan = self
            .store
            .latest_plan(conversation)?
            .first()
            .map(|task| task.plan_id.clone())
            .ok_or_else(|| Error::Invalid("there is no running plan".into()))?;
        let task = self
            .store
            .task_in_plan_by_id(&plan, name)?
            .ok_or_else(|| Error::Invalid(format!("task: no task {name} in the running plan")))?;
        Ok((plan, task))
    }

    fn orchestrator_retries(&self, conversation: &str, task: &str) -> Result<i64> {
        Ok(self.store.db().query_row(
            "SELECT count(*) FROM events WHERE conversation_id=?1 AND kind='task_recovery'
               AND json_extract(payload,'$.actor')='orchestrator'
               AND json_extract(payload,'$.task')=?2",
            rusqlite::params![conversation, task],
            |row| row.get(0),
        )?)
    }
}

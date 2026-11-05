//! The orchestrator's `retry_task`: a task that failed for a reason outside
//! its contract (a turn limit, a crashed CLI) runs again in its own worktree,
//! without waiting for the user. `resume_task` continues a coordinating task
//! after its prerequisite is resolved, keeping the same inputs and permission.
use super::*;

impl Core {
    pub(in crate::lean) fn resume_coordinating_task(
        self: &Arc<Self>,
        conversation: &str,
        role: &str,
        turn: &str,
        name: &str,
        expected: (u32, i64),
        resolution: &str,
    ) -> Result<serde_json::Value> {
        let request = self.resume_request(conversation, role, turn, name, expected, resolution)?;
        self.recover_task_as(request, "orchestrator")?;
        Ok(json!({"resumed": name,"inputs_changed":false,"authorization_renewed":false}))
    }

    pub(in crate::lean) fn retry_failed_task(
        self: &Arc<Self>,
        conversation: &str,
        role: &str,
        turn: &str,
        name: &str,
        guidance: &str,
    ) -> Result<serde_json::Value> {
        let request = self.retry_request(conversation, role, turn, name, guidance)?;
        self.recover_task_as(request, "orchestrator")?;
        Ok(json!({"retried": name}))
    }

    fn retry_request(
        &self,
        conversation: &str,
        role: &str,
        turn: &str,
        name: &str,
        guidance: &str,
    ) -> Result<TaskRecoveryRequest> {
        let (plan, task) = self.recovery_task(conversation, role, turn, name)?;
        require(
            task.status == status::FAILED,
            &format!(
                "task {name} currently has status {} in plan {plan}; retry_task only accepts failed tasks. Use resume_task only for a resolved prerequisite under the unchanged contract; repair contracts with revise_plan",
                task.status
            ),
        )?;
        self.unchanged_recovery_request(conversation, plan, task, guidance)
    }

    fn resume_request(
        &self,
        conversation: &str,
        role: &str,
        turn: &str,
        name: &str,
        expected: (u32, i64),
        resolution: &str,
    ) -> Result<TaskRecoveryRequest> {
        let (plan, task) = self.recovery_task(conversation, role, turn, name)?;
        require(
            (task.round, task.input_generation) == expected,
            "stale task round/input_generation; use a fresh task_states snapshot",
        )?;
        require(
            task.status == status::COORDINATING && task.spec["deferred"] != true,
            "resume_task requires a non-deferred coordinating task; it cannot release future work, answer a question, or restart active work",
        )?;
        require(!resolution.trim().is_empty(), "resolution: required")?;
        require(
            !matches!(self.store.open_user_decision(conversation)?, Some(Some(_))),
            "an unanswered user decision is open; resume_task cannot bypass it",
        )?;
        self.unchanged_recovery_request(
            conversation,
            plan,
            task,
            &format!(
                "Continue retained work with full access and no execution budgets: {resolution}"
            ),
        )
    }

    fn unchanged_recovery_request(
        &self,
        conversation: &str,
        plan: String,
        task: Task,
        guidance: &str,
    ) -> Result<TaskRecoveryRequest> {
        self.store.assert_task_inputs(&task)?;
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

    pub(super) fn recovery_task(
        &self,
        conversation: &str,
        role: &str,
        turn: &str,
        name: &str,
    ) -> Result<(String, Task)> {
        let plan = self.store.dependency_plan(conversation, role, turn)?;
        require(
            self.store.accepted_plan(conversation)?.as_deref() == Some(&plan),
            "task recovery turn is not bound to the current accepted plan; use a fresh orchestration turn",
        )?;
        let tasks = self.store.plan(&plan)?;
        let task = tasks
            .iter()
            .find(|task| task.id == name || task.spec["id"].as_str() == Some(name))
            .cloned()
            .ok_or_else(|| {
                let current = tasks
                    .iter()
                    .map(|task| {
                        format!(
                            "{}={}",
                            task.spec["id"].as_str().unwrap_or("?"),
                            task.status
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                Error::Invalid(format!(
                    "task: no task {name} in accepted plan {plan}; current task states: {current}"
                ))
            })?;
        Ok((plan, task))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recoveries_continue_beyond_the_former_limit_without_a_permission_question() {
        let (_directory, core, conversation, mut task, turn) =
            super::super::test_support::recovery_fixture();
        for attempt in 0..5 {
            task.status = status::FAILED.into();
            core.store.update_task(&task).unwrap();
            let reply = core
                .retry_failed_task(
                    &conversation,
                    ORCHESTRATOR,
                    &turn.id,
                    "integration-gate",
                    &format!("Inspect retained files and repair failure {attempt}"),
                )
                .unwrap();
            assert_eq!(reply["retried"], "integration-gate");
            assert!(
                core.store
                    .open_user_decision(&conversation)
                    .unwrap()
                    .is_none()
            );
        }
        let count: i64 = core
            .store
            .db()
            .query_row(
                "SELECT count(*) FROM events WHERE conversation_id=?1 AND kind='task_recovery'",
                [&conversation],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 5);
    }
}

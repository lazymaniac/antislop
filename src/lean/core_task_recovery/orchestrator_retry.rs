//! The orchestrator's `retry_task`: a task that failed for a reason outside
//! its contract (a turn limit, a crashed CLI) runs again in its own worktree,
//! without waiting for the user. `resume_task` continues a coordinating task
//! after its prerequisite is resolved, keeping the same inputs and permission.
use super::*;

/// Retries the orchestrator may start for one task; then the user decides.
const MAX_ORCHESTRATOR_RETRIES: i64 = 2;
const RECOVERY_LIMIT: &str =
    "task: retried twice already; a bound one-time user recovery decision is required";

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
        let result = (|| {
            let request =
                self.resume_request(conversation, role, turn, name, expected, resolution)?;
            self.recover_task_as(request, "orchestrator")?;
            Ok(json!({"resumed": name,"inputs_changed":false,"authorization_renewed":false}))
        })();
        match result {
            Err(Error::Invalid(reason)) if reason == RECOVERY_LIMIT => self
                .request_recovery_approval(
                    conversation,
                    role,
                    turn,
                    name,
                    RecoveryContinuation {
                        expected: Some(expected),
                        guidance: resolution,
                        scope: RecoveryScope::EditsOnly,
                    },
                ),
            other => other,
        }
    }

    pub(in crate::lean) fn retry_failed_task(
        self: &Arc<Self>,
        conversation: &str,
        role: &str,
        turn: &str,
        name: &str,
        guidance: &str,
    ) -> Result<serde_json::Value> {
        let result = (|| {
            let request = self.retry_request(conversation, role, turn, name, guidance)?;
            self.recover_task_as(request, "orchestrator")?;
            Ok(json!({"retried": name}))
        })();
        match result {
            Err(Error::Invalid(reason)) if reason == RECOVERY_LIMIT => self
                .request_recovery_approval(
                    conversation,
                    role,
                    turn,
                    name,
                    RecoveryContinuation {
                        expected: None,
                        guidance,
                        scope: RecoveryScope::EditsOnly,
                    },
                ),
            other => other,
        }
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
            &format!("The orchestrator supplied a concrete continuation within the unchanged contract: {resolution}\nContinue retained work. Inputs, execution permission and cumulative budgets are unchanged. This is not permission for additional or repeated project invocations."),
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
        self.assert_orchestrator_recovery_budget(conversation, &task.id)?;
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

    fn orchestrator_retries(&self, conversation: &str, task: &str) -> Result<i64> {
        Ok(self.store.db().query_row(
            "SELECT count(*) FROM events WHERE conversation_id=?1 AND kind='task_recovery'
               AND json_extract(payload,'$.actor')='orchestrator'
               AND json_extract(payload,'$.task')=?2",
            rusqlite::params![conversation, task],
            |row| row.get(0),
        )?)
    }

    pub(super) fn assert_orchestrator_recovery_budget(
        &self,
        conversation: &str,
        task: &str,
    ) -> Result<()> {
        require(
            self.orchestrator_retries(conversation, task)? < MAX_ORCHESTRATOR_RETRIES,
            RECOVERY_LIMIT,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_uses_the_turns_accepted_plan_and_never_a_newer_draft() {
        let (_directory, core, conversation, mut accepted, turn) =
            super::super::test_support::recovery_fixture();
        let authorize = |plan: &str| {
            core.store.db().execute("INSERT INTO plan_authorizations(plan_id,conversation_id,scope,write_scope) VALUES(?1,?2,'[\"results\"]','[]')",rusqlite::params![plan,conversation]).unwrap();
        };
        // A replacement draft omits the failed gate entirely.
        let draft=core.store.add_plan(&conversation,&[json!({"id":"different","title":"Draft","goal":"Different work","kind":"analysis"})]).unwrap().remove(0);
        for name in ["integration-gate", accepted.id.as_str()] {
            let request = core
                .retry_request(
                    &conversation,
                    ORCHESTRATOR,
                    &turn.id,
                    name,
                    "Check retained work first",
                )
                .unwrap();
            assert_eq!(request.plan, accepted.plan_id);
            assert_eq!(request.task, accepted.id);
        }
        let unknown = core
            .retry_request(&conversation, ORCHESTRATOR, &turn.id, "different", "Retry")
            .unwrap_err()
            .to_string();
        assert!(
            unknown.contains("current task states: integration-gate=failed"),
            "{unknown}"
        );
        accepted.status = status::COORDINATING.into();
        core.store.update_task(&accepted).unwrap();
        let stalled = core
            .agent_tool(
                &conversation,
                ORCHESTRATOR,
                &turn.id,
                "retry_task",
                &json!({"task":"integration-gate","guidance":"Retry"}),
            )
            .unwrap_err()
            .to_string();
        assert!(
            stalled.contains("status coordinating") && stalled.contains("revise_plan"),
            "{stalled}"
        );
        assert_eq!(
            core.store.task_by_id(&accepted.id).unwrap().unwrap().status,
            status::COORDINATING
        );
        let before_resume = core.store.task_by_id(&accepted.id).unwrap().unwrap();
        let resumed = core
            .resume_request(
                &conversation,
                ORCHESTRATOR,
                &turn.id,
                "integration-gate",
                (accepted.round, accepted.input_generation),
                "The approved construction is available; implement the assigned output",
            )
            .unwrap();
        assert_eq!(resumed.task, accepted.id);
        for stale in [
            (accepted.round + 1, accepted.input_generation),
            (accepted.round, accepted.input_generation + 1),
        ] {
            assert!(
                core.resume_request(
                    &conversation,
                    ORCHESTRATOR,
                    &turn.id,
                    "integration-gate",
                    stale,
                    "Stale resolution"
                )
                .unwrap_err()
                .to_string()
                .contains("stale task")
            );
        }
        let decision = serde_json::from_value(json!({"id":"recovery-decision","context":"Synthetic execution decision","question":"Continue?","options":["Continue","Stop"]})).unwrap();
        core.store
            .ask_user_decision(&conversation, &decision)
            .unwrap();
        assert!(
            core.resume_request(
                &conversation,
                ORCHESTRATOR,
                &turn.id,
                "integration-gate",
                (accepted.round, accepted.input_generation),
                "Cannot bypass a pending decision"
            )
            .unwrap_err()
            .to_string()
            .contains("unanswered user decision")
        );
        core.store
            .add_message(
                &conversation,
                "user",
                "Interview answer (recovery-decision)\nQuestion: Continue?\nAnswer: Continue",
            )
            .unwrap();
        core.pause(&conversation, true).unwrap();
        // Model an already running, paused scheduler; no CLI or fresh planning cycle.
        core.plans
            .lock()
            .unwrap()
            .insert(conversation.clone(), Arc::new(AtomicBool::new(false)));
        core.recover_task_as(resumed, "orchestrator").unwrap();
        let after_resume = core.store.task_by_id(&accepted.id).unwrap().unwrap();
        assert_eq!(after_resume.status, status::QUEUED);
        assert_eq!(after_resume.spec, before_resume.spec);
        assert_eq!(
            after_resume.input_generation,
            before_resume.input_generation
        );
        assert_eq!(after_resume.round, before_resume.round);
        // Resuming cannot start active/queued work a second time.
        assert!(
            core.resume_request(
                &conversation,
                ORCHESTRATOR,
                &turn.id,
                "integration-gate",
                (accepted.round, accepted.input_generation),
                "Already resolved"
            )
            .is_err()
        );
        accepted.status = status::COORDINATING.into();
        core.store.update_task(&accepted).unwrap();
        core.store
            .db()
            .execute(
                "UPDATE tasks SET spec=json_set(spec,'$.deferred',json('true')) WHERE id=?1",
                [&accepted.id],
            )
            .unwrap();
        assert!(
            core.resume_request(
                &conversation,
                ORCHESTRATOR,
                &turn.id,
                "integration-gate",
                (accepted.round, accepted.input_generation),
                "Do not release future work"
            )
            .is_err()
        );
        core.store
            .db()
            .execute(
                "UPDATE tasks SET spec=json_remove(spec,'$.deferred') WHERE id=?1",
                [&accepted.id],
            )
            .unwrap();
        accepted.status = status::FAILED.into();
        core.store.update_task(&accepted).unwrap();
        let prepared_before_budget_changed = core
            .retry_request(
                &conversation,
                ORCHESTRATOR,
                &turn.id,
                "integration-gate",
                "Prepared before another recovery",
            )
            .unwrap();
        core.store
            .append_event(
                &conversation,
                None,
                Some(&accepted.plan_id),
                "task_recovery",
                &json!({"actor":"orchestrator","task":accepted.id}),
            )
            .unwrap();
        assert!(
            core.retry_request(
                &conversation,
                ORCHESTRATOR,
                &turn.id,
                "integration-gate",
                "Retry"
            )
            .unwrap_err()
            .to_string()
            .contains("retried twice")
        );
        assert!(
            core.recover_task_as(prepared_before_budget_changed, "orchestrator")
                .unwrap_err()
                .to_string()
                .contains("retried twice")
        );
        assert_eq!(
            core.store.task_by_id(&accepted.id).unwrap().unwrap().status,
            status::FAILED
        );
        authorize(&draft.plan_id);
        let replaced = core
            .retry_request(
                &conversation,
                ORCHESTRATOR,
                &turn.id,
                "integration-gate",
                "Retry",
            )
            .unwrap_err()
            .to_string();
        assert!(replaced.contains("current accepted plan"), "{replaced}");
    }
}

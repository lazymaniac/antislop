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
        let (plan, task) = self.failed_task(conversation, role, turn, name)?;
        require(
            task.status == status::FAILED,
            &format!(
                "task {name} currently has status {} in plan {plan}; retry_task only accepts failed tasks. Repair coordinating/stalled contracts with revise_plan, not retry_task",
                task.status
            ),
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

    fn failed_task(
        &self,
        conversation: &str,
        role: &str,
        turn: &str,
        name: &str,
    ) -> Result<(String, Task)> {
        let plan = self.store.dependency_plan(conversation, role, turn)?;
        require(
            self.store.accepted_plan(conversation)?.as_deref() == Some(&plan),
            "retry_task turn is not bound to the current accepted plan; use a fresh orchestration turn",
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_uses_the_turns_accepted_plan_and_never_a_newer_draft() {
        let directory = tempfile::tempdir().unwrap();
        let config = Config::from_toml(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/examples/antislop.toml"
        )))
        .unwrap();
        let core = Core::open(&directory.path().join("state"), config).unwrap();
        let conversation = core
            .store
            .create_conversation(directory.path().to_str().unwrap())
            .unwrap()
            .id;
        let spec = json!({"id":"integration-gate","title":"Integration","goal":"Validate","kind":"analysis","scope":["results"]});
        let mut accepted = core
            .store
            .add_plan(&conversation, &[spec])
            .unwrap()
            .remove(0);
        accepted.status = status::FAILED.into();
        core.store.update_task(&accepted).unwrap();
        let authorize = |plan: &str| {
            core.store.db().execute("INSERT INTO plan_authorizations(plan_id,conversation_id,scope,write_scope) VALUES(?1,?2,'[\"results\"]','[]')",rusqlite::params![plan,conversation]).unwrap();
        };
        authorize(&accepted.plan_id);
        let turn = core
            .store
            .start_turn(
                &conversation,
                ORCHESTRATOR,
                "fixture",
                0,
                Some(&accepted.plan_id),
            )
            .unwrap();
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
        accepted.status = status::FAILED.into();
        core.store.update_task(&accepted).unwrap();
        for _ in 0..2 {
            core.store
                .append_event(
                    &conversation,
                    None,
                    Some(&accepted.plan_id),
                    "task_recovery",
                    &json!({"actor":"orchestrator","task":accepted.id}),
                )
                .unwrap();
        }
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

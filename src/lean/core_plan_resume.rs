use super::*;

impl Core {
    /// Runs again every task of `plan_id` that ended without approval
    /// (interrupted, failed, waiting for a login, needing the user, skipped).
    /// Agents start a fresh round with their durable handoff; a change
    /// that was approved but could not be integrated is integrated again.
    pub fn resume_plan(self: &Arc<Self>, conversation: &str, plan_id: &str) -> Result<Vec<Task>> {
        require(
            self.store
                .plan_belongs_to_conversation(conversation, plan_id)?,
            "plan does not belong to conversation",
        )?;
        require(
            self.store.plan_run(plan_id)?.is_some(),
            "integrity error: plan has no requirements run",
        )?;
        if let Some(run) = self.store.plan_run(plan_id)?
            && self.store.extraction_needs_refresh(conversation, &run)?
        {
            self.store.add_message(conversation, RUNTIME,
                "Resume requested: refresh requirements source coverage before continuing the accepted plan. Preserve existing requirements and checkpoints; operational comments are not new acceptance criteria.")?;
            self.wake(conversation);
            return Err(Error::Invalid(
                "requirements source coverage refresh queued; resume after the check completes"
                    .into(),
            ));
        }
        self.start_plan(conversation, plan_id, |store| {
            store.db().execute("DELETE FROM provider_blocks WHERE (provider,profile) IN (SELECT t.provider,r.profile FROM run_contexts r JOIN turns t ON t.id=r.turn_id WHERE t.conversation_id=?1 AND t.plan_id=?2)", rusqlite::params![conversation,plan_id])?;
            if store.requeue_plan(conversation, plan_id)? > 0 {
                return Ok(());
            }
            let waiting: bool = store.db().query_row(
                "SELECT EXISTS(SELECT 1 FROM tasks t WHERE t.conversation_id=?1 AND t.plan_id=?2
                 AND (EXISTS(SELECT 1 FROM agent_questions q WHERE q.id=json_extract(t.feedback,'$.waiting_request') AND q.state IN ('pending','awaiting_user'))
                  OR EXISTS(SELECT 1 FROM agent_answer_waits w WHERE w.request=json_extract(t.feedback,'$.waiting_request') AND w.released=0)))",
                rusqlite::params![conversation,plan_id], |row| row.get(0))?;
            require(
                false,
                if waiting {
                    "this plan is waiting for agent answers or their revision/dependency; resolve its open questions before resuming"
                } else {
                    "this plan has nothing to resume"
                },
            )
        })
    }

    pub fn accept_plan_scoped(
        self: &Arc<Self>,
        conversation: &str,
        plan_id: &str,
        scope_hash: Option<&str>,
    ) -> Result<Vec<Task>> {
        if let Some(expected) = scope_hash {
            let current = self.plan_scope_hash(conversation, plan_id)?;
            require(expected == current, "stale_scope")?;
        }
        self.accept_plan(conversation, plan_id)
    }

    pub fn resume_plan_scoped(
        self: &Arc<Self>,
        conversation: &str,
        plan_id: &str,
        scope_hash: Option<&str>,
    ) -> Result<Vec<Task>> {
        if let Some(expected) = scope_hash {
            let current = self.plan_scope_hash(conversation, plan_id)?;
            require(expected == current, "stale_scope")?;
        }
        self.resume_plan(conversation, plan_id)
    }

    pub fn recover_task(
        self: &Arc<Self>,
        req: TaskRecoveryRequest,
    ) -> Result<TaskRecoveryResponse> {
        self.recover_task_as(req, "user")
    }
}

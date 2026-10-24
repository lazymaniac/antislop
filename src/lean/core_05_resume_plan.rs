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
        self.start_plan(conversation, plan_id, |store| {
            require(
                store.requeue_plan(conversation, plan_id)? > 0,
                "this plan has nothing to resume",
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

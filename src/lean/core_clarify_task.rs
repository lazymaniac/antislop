use super::*;

impl Core {
    pub(super) fn clarify_blocked_task(
        self: &Arc<Self>,
        conversation: &str,
        plan: &str,
        step: &tasks::Unblocking,
        cancel: &AtomicBool,
    ) -> Result<String> {
        let before = self.task_unscoped(plan, &step.task)?;
        self.check_clarifiable(&before)?;
        let spec = step
            .spec
            .as_ref()
            .ok_or_else(|| Error::Invalid("clarify_task needs spec".into()))?;
        let reason = step
            .message
            .as_deref()
            .filter(|text| !text.trim().is_empty())
            .ok_or_else(|| Error::Invalid("clarify_task needs an evidence-based message".into()))?;
        blocker::check_clarification(&serde_json::from_value(before.spec.clone())?, spec)?;
        let token = self.task_freshness_token(conversation, plan, &before.id)?;
        let project = PathBuf::from(self.store.conversation(conversation)?.project);
        let ledger = self
            .store
            .plan_run(plan)?
            .map(|run| self.ledger_context(conversation, &run))
            .transpose()?
            .unwrap_or_default();
        let review = self
            .review(
                conversation,
                PLAN_VERIFIER,
                Some(plan),
                &project,
                cancel,
                |_, request| clarification_prompt(request, &ledger, &before, spec, reason),
            )?
            .map_err(Error::Invalid)?;
        require(
            !cancel.load(Ordering::SeqCst),
            "clarification review was cancelled",
        )?;
        require(
            review.approved,
            &format!(
                "clarification rejected: {}",
                serde_json::to_string(&review.findings)?
            ),
        )?;
        {
            let _graph = self.graph_updates.lock().expect("graph update mutex");
            let _integration = self.integrating.lock().expect("integration mutex");
            let current = self.task_unscoped(plan, &before.id)?;
            self.check_clarifiable(&current)?;
            require(
                current.spec == before.spec
                    && token == self.task_freshness_token(conversation, plan, &before.id)?,
                "task changed during clarification review",
            )?;
            self.store
                .clarify_task(conversation, &before, spec, &review, reason)?;
        }
        self.maybe_continue_plan(conversation, plan)?;
        Ok(status::QUEUED.into())
    }

    fn check_clarifiable(&self, task: &Task) -> Result<()> {
        self.store.assert_task_inputs(task)?;
        require(
            task.status == status::NEEDS_USER,
            "only a blocked task can be clarified",
        )?;
        let block = task
            .report
            .as_deref()
            .and_then(blocker::parse)
            .ok_or_else(|| Error::Invalid("task has no structured blocker".into()))?;
        require(
            !block.for_user(),
            "only the user can lift a permission blocker",
        )?;
        require(
            !self
                .task_cancels
                .lock()
                .expect("task cancel mutex")
                .contains_key(&task.id),
            "the blocked task is still running",
        )?;
        require(
            self.store.clarification_count(&task.id)? < blocker::CLARIFICATIONS,
            "task was already clarified twice; the user decides",
        )?;
        Ok(())
    }
}

fn clarification_prompt(
    request: &str,
    ledger: &str,
    before: &Task,
    after: &TaskSpec,
    reason: &str,
) -> String {
    format!(
        "You independently review a task contract clarification, not a complete new plan. Work read-only. User request: {request}\n{}\nReview a proposed clarification of a blocked read-only task. Inspect the project's actual records and dependency reports/verdicts. Preserve every substantive accepted requirement and criterion. Reject scope expansion, weakened acceptance, fabricated history, force approval treated as independent verification, or a prerequisite waiting for work assigned to its downstream consumer. Missing historical source identities may be recorded as unavailable; a NEW source-backed assessment can replace stale claims only if all original substantive obligations still receive independent verification. Approving this clarification approves no task result. The worker and its verifier must run afterwards. Return the normal verdict format.\nAccepted ledger:\n{}\nBefore:\n{}\nProposed:\n{}\nRationale:\n{}\nCurrent blocker:\n{}",
        tasks::VERDICT_FORMAT,
        ledger,
        before.spec,
        serde_json::to_string(after).unwrap_or_default(),
        reason,
        before.report.as_deref().unwrap_or_default()
    )
}

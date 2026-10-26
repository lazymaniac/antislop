use super::*;
use serde_json::Value;

impl Core {
    pub(super) fn clarify_blocked_task(
        self: &Arc<Self>,
        conversation: &str,
        plan: &str,
        step: &tasks::Unblocking,
        cancel: &AtomicBool,
    ) -> Result<String> {
        require(
            !cancel.load(Ordering::SeqCst),
            "clarification review was cancelled",
        )?;
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
        let observation = self.recovery_observation(conversation, &before)?;
        let history = self.store.recovery_history(&before.id, "clarify_task")?;
        let attempt = {
            let _graph = self.graph_updates.lock().expect("graph update mutex");
            let _integration = self.integrating.lock().expect("integration mutex");
            self.check_clarification_fresh(conversation, plan, &before, &token, &observation)?;
            self.store
                .reserve_clarification(conversation, &before, spec, &observation, reason)?
        };
        let mut review = self
            .review(
                conversation,
                PLAN_VERIFIER,
                Some(plan),
                &project,
                cancel,
                |_, request| {
                    clarification_prompt(request, &ledger, &before, spec, reason, &history)
                },
            )?
            .map_err(Error::Invalid)?;
        review.approved &= !recovery_progress::needs_user_or_stalled(&review);
        require(
            !cancel.load(Ordering::SeqCst),
            "clarification review was cancelled",
        )?;
        {
            let _graph = self.graph_updates.lock().expect("graph update mutex");
            let _integration = self.integrating.lock().expect("integration mutex");
            self.check_clarification_fresh(conversation, plan, &before, &token, &observation)?;
            let reviewer = self
                .store
                .last_turn(conversation, PLAN_VERIFIER)?
                .map(|turn| turn.id);
            self.store.append_event(
                conversation,
                None,
                Some(plan),
                "clarification_reviewed",
                &json!({"task":before.id,"attempt":attempt,"review_turn":reviewer,"review":review}),
            )?;
            if !review.approved {
                self.report_clarification_rejection(conversation, &before, &review)?;
                return Err(Error::Invalid(format!(
                    "clarification rejected: {}",
                    serde_json::to_string(&review.findings)?
                )));
            }
            self.store.clarify_task(
                conversation,
                &before,
                spec,
                &review,
                reason,
                Some(&attempt),
            )?;
        }
        self.maybe_continue_plan(conversation, plan)?;
        Ok(status::QUEUED.into())
    }

    fn check_clarification_fresh(
        &self,
        conversation: &str,
        plan: &str,
        before: &Task,
        token: &str,
        observation: &str,
    ) -> Result<()> {
        let current = self.task_unscoped(plan, &before.id)?;
        self.check_clarifiable(&current)?;
        require(
            current.spec == before.spec
                && token == self.task_freshness_token(conversation, plan, &before.id)?
                && observation == self.recovery_observation(conversation, &current)?,
            "task changed during clarification review",
        )
    }

    fn report_clarification_rejection(
        &self,
        conversation: &str,
        task: &Task,
        review: &Verdict,
    ) -> Result<()> {
        if !recovery_progress::actionable(review) {
            return Ok(());
        }
        let next = "Address the specific reviewer findings in a materially corrected goal/focus or supply new evidence, then submit clarify_task for independent review. Preserve all accepted obligations. Repeating the same proposal with unchanged inputs is rejected; there is no attempt quota.";
        self.store.add_message(conversation, RUNTIME, &format!(
            "Clarification review rejected for {}. The task contract and result remain unchanged. Independent reviewer findings:\n{}\n{next}\nHistorical notes and architecture recommendations are evidence to assess, not additional accepted requirements. Make instrumentation or other implementation choices conditional on their necessity for an accepted criterion; propose any scope expansion separately.",
            task.spec["id"].as_str().unwrap_or(&task.id), serde_json::to_string(&review.findings)?
        ))?;
        Ok(())
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
        Ok(())
    }
}

fn clarification_prompt(
    request: &str,
    ledger: &str,
    before: &Task,
    after: &TaskSpec,
    reason: &str,
    history: &[Value],
) -> String {
    format!(
        "You independently review a task contract clarification, not a complete new plan. Work read-only. User request: {request}\n{}\nReview a proposed clarification of a blocked read-only task. Inspect the project's actual records and dependency reports/verdicts. Preserve every substantive accepted requirement and criterion. Treat historical notes and architecture recommendations as evidence, not as additional accepted requirements. Instrumentation or other implementation choices must be justified by an accepted criterion; added scope needs separate approval. Reject scope expansion, weakened acceptance, fabricated history, force approval treated as independent verification, or a prerequisite waiting for work assigned to its downstream consumer. Missing historical source identities may be recorded as unavailable; a NEW source-backed assessment can replace stale claims only if all original substantive obligations still receive independent verification. Approving this clarification approves no task result. The worker and its verifier must run afterwards. Compare this proposal with previous attempts and findings. A genuine correction or new evidence may continue without a numerical limit. Rewording the same rejected approach is not progress: include a material finding with code `recovery_no_progress`. If accepted requirements cannot be met without a user decision, use code `recovery_user_decision`. Those codes stop automatic continuation; ordinary actionable review findings return to the orchestrator for correction. Return the normal verdict format.\nPrevious recovery attempts and reviews:\n{}\nAccepted ledger:\n{}\nBefore:\n{}\nProposed:\n{}\nRationale:\n{}\nCurrent blocker:\n{}",
        tasks::VERDICT_FORMAT,
        serde_json::to_string(history).unwrap_or_default(),
        ledger,
        before.spec,
        serde_json::to_string(after).unwrap_or_default(),
        reason,
        before.report.as_deref().unwrap_or_default()
    )
}

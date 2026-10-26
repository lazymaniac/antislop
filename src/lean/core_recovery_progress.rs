use super::*;

impl Core {
    /// A round counter or a new CLI session is not new evidence.
    pub(super) fn recovery_observation(&self, conversation: &str, task: &Task) -> Result<String> {
        let spec: TaskSpec = serde_json::from_value(task.spec.clone())?;
        let project = PathBuf::from(self.store.conversation(conversation)?.project);
        let outcome = match task.report.as_deref().and_then(blocker::parse) {
            Some(block) => {
                json!({"kind":block.kind,"needs":recovery_progress::text(&block.needs),"why":recovery_progress::text(&block.why)})
            }
            None => json!(recovery_progress::outcome(
                task.report.as_deref().unwrap_or_default()
            )),
        };
        let mut dependencies = Vec::new();
        for id in &spec.depends_on {
            let dependency = self.task_unscoped(&task.plan_id, id)?;
            dependencies.push(json!({"id":id,"status":dependency.status,"generation":dependency.input_generation,
                "result":dependency.result,"report":recovery_progress::text(dependency.report.as_deref().unwrap_or_default()),"verdict":dependency.verdict}));
        }
        let state = json!({"spec":recovery_progress::normalized_spec(task.spec.clone()),"generation":task.input_generation,
            "outcome":outcome,"verdict":task.verdict,"result":task.result,"dependencies":dependencies,
            "sources":git::recovery_fingerprint(&project,&spec.scope)?,
            "user_message":self.store.recovery_user_message(conversation)?});
        Ok(crate::hash(serde_json::to_string(&state)?.as_bytes()))
    }

    pub(super) fn check_recovery_progress(
        self: &Arc<Self>,
        conversation: &str,
        task: &Task,
        step: &tasks::Unblocking,
        cancel: &AtomicBool,
    ) -> Result<()> {
        let observation = self.recovery_observation(conversation, task)?;
        let token = self.task_freshness_token(conversation, &task.plan_id, &task.id)?;
        let spec = step.spec.as_ref().map(serde_json::to_value).transpose()?;
        let parts = step
            .parts
            .iter()
            .map(serde_json::to_value)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let proposal = recovery_progress::proposal(
            &step.action,
            spec.as_ref(),
            step.message.as_deref(),
            &parts,
        );
        let history = self.store.recovery_history(&task.id, &step.action)?;
        let attempt = self.store.reserve_recovery_attempt(
            conversation,
            task,
            &step.action,
            &observation,
            &proposal,
        )?;
        if history.is_empty() && !(step.action == "add_task" && task.spec["resolves"].is_string()) {
            return Ok(());
        }
        let project = PathBuf::from(self.store.conversation(conversation)?.project);
        let ledger = self
            .store
            .plan_run(&task.plan_id)?
            .map(|run| self.ledger_context(conversation, &run))
            .transpose()?
            .unwrap_or_default();
        let mut review = self.review(conversation, PLAN_VERIFIER, Some(&task.plan_id), &project, cancel, |_, request| format!(
            "Assess progress of a proposed recovery, read-only. This is not approval of task results or a new plan. Compare the concrete previous approaches and findings with the proposed action. Approve only a changed approach addressing the failure, necessary new input, or newly available evidence. Closing previously open verified obligations is observable progress even if some findings remain. Rewording, renamed tasks, extra rounds and moving the same missing work to another prerequisite are not progress. Reject requirements that cannot be met without the user's decision. Preserve every accepted obligation. Assess new explicit user instructions or reported external fixes as evidence for another attempt, never as proof that a task passed.\nUser request: {request}\nAccepted ledger: {ledger}\nPrevious attempts and reviews: {}\nCurrent task: {}\nCurrent outcome: {}\nCurrent worker review and open obligation IDs: {}\nAction: {}\nProposal: {}\n{}",
            serde_json::to_string(&history).unwrap_or_default(),task.spec,task.report.as_deref().unwrap_or_default(),recovery_progress::review_evidence(task.verdict.as_ref()),step.action,proposal,tasks::VERDICT_FORMAT
        ))?.map_err(Error::Invalid)?;
        review.approved &= !recovery_progress::needs_user_or_stalled(&review);
        require(
            !cancel.load(Ordering::SeqCst),
            "recovery progress review was cancelled",
        )?;
        self.store.append_event(
            conversation,
            None,
            Some(&task.plan_id),
            "recovery_progress_reviewed",
            &json!({"task":task.id,"attempt":attempt,"review":review}),
        )?;
        require(
            review.approved,
            &format!(
                "recovery made no progress or needs a user decision: {}",
                serde_json::to_string(&review.findings)?
            ),
        )?;
        require(
            token == self.task_freshness_token(conversation, &task.plan_id, &task.id)?
                && observation
                    == self.recovery_observation(
                        conversation,
                        &self.task_unscoped(&task.plan_id, &task.id)?,
                    )?,
            "task inputs changed during recovery progress review",
        )
    }
}

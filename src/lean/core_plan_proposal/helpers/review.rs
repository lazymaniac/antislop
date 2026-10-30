use super::*;

impl Core {
    pub(in crate::lean::core_plan_proposal) fn review_plan(
        self: &Arc<Self>,
        conversation: &str,
        project: &Path,
        plan: &PreparedPlan,
        specs: &[TaskContract],
        config: &Config,
        cancel: &AtomicBool,
    ) -> Result<std::result::Result<Verdict, String>> {
        record_plan_review_start(&self.store, conversation, &plan.id, config, specs)?;
        let project_text = project.to_string_lossy().into_owned();
        let gaps = planning_gaps(specs);
        let dependency_note = dependency_note(specs);
        self.review(
            conversation,
            PLAN_VERIFIER,
            Some(&plan.id),
            project,
            cancel,
            |first, request| {
                let prompt = plan_review_prompt(&project_text, request, specs, first);
                append_planning_gaps(prompt, gaps.as_deref()) + &dependency_note
            },
        )
    }

    pub(in crate::lean::core_plan_proposal) fn cancel_proposed_plan(
        &self,
        plan: &str,
    ) -> Result<()> {
        self.store
            .set_plan_status(plan, status::PROPOSED, status::CANCELLED)?;
        Ok(())
    }

    pub(in crate::lean::core_plan_proposal) fn requeue_rejected_plan(
        &self,
        conversation: &str,
        plan: &str,
        review: &std::result::Result<Verdict, String>,
    ) -> Result<bool> {
        let Ok(verdict) = review else {
            return Ok(false);
        };
        if verdict.approved {
            return Ok(false);
        }
        if let Some(previous) = self.store.previous_plan_rejection(conversation)? {
            let before = super::super::progress::open_obligations(&previous);
            let now = super::super::progress::open_obligations(verdict);
            let mut progress = super::super::progress::Progress::default();
            progress.record(before.clone());
            if before.is_empty() || now.is_empty() || progress.record(now) {
                self.notice(conversation,"plan review has not confirmed closure of a previous finding; preserve stable finding ids and provide a new basis before retrying");
                return Ok(false);
            }
        }
        self.record_proposed_plan_rejection(conversation, plan, verdict)?;
        Ok(true)
    }

    fn record_proposed_plan_rejection(
        &self,
        conversation: &str,
        plan: &str,
        verdict: &Verdict,
    ) -> Result<()> {
        self.store
            .set_plan_status(plan, status::PROPOSED, status::REVISED)?;
        self.store.append_event(
            conversation,
            None,
            Some(plan),
            "plan_review",
            &json!({"plan": plan, "approved": false, "findings": verdict.findings}),
        )?;
        self.store.append_event(
            conversation,
            None,
            Some(plan),
            "planning_progress",
            &json!({"text": "Revising the plan after review"}),
        )?;
        self.store.add_message(
            conversation,
            PLAN_VERIFIER,
            &tasks::plan_rejected(&verdict.findings),
        )?;
        Ok(())
    }
}

fn dependency_note(specs: &[TaskContract]) -> String {
    feasibility::check(specs)
        .iter()
        .map(|finding| {
            format!(
                "\n- {} [{}]: {}",
                finding.task, finding.rule, finding.detail
            )
        })
        .collect()
}

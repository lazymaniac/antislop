use super::core_feedback_helpers::review_json;
use super::core_plan_helpers::{finalize_plan, planning_gaps};
use super::*;

impl Core {
    /// Stores the plan and has it checked. A plan the verifier sends back
    /// goes back to the orchestrator (a bounded number of times); otherwise it
    /// is shown to the user with the verdict, and in autonomous mode a checked
    /// plan starts at once.
    pub(in crate::lean) fn propose(
        self: &Arc<Self>,
        conversation: &str,
        project: &Path,
        text: &str,
        proposal: PlanProposal<'_>,
        addressed: Option<&[u64]>,
        cancel: &AtomicBool,
    ) -> Result<()> {
        let PlanProposal { config, specs } = proposal;
        // Every proposal, including format repairs and comment revisions, is
        // finalized before persistence, gap checks, review and graph rendering.
        let finalized = finalize_plan(specs.to_vec(), config.policy.required_gates);
        let specs = finalized.as_slice();
        if let Some(ids) = addressed {
            self.store.append_event(
                conversation, None, None, "plan_comments_addressed",
                &json!({"ids": ids, "upto_message": self.store.last_orchestration_answered(conversation)?}),
            )?;
        }
        let run = match self.ensure_requirements(conversation, cancel) {
            Ok(run) => run,
            Err(error) => {
                self.store.append_event(
                    conversation, None, None, "requirements_failed",
                    &json!({"error": error.to_string(), "retry": "Ask the orchestrator to regenerate the task plan; a bare approval does not rerun requirements preparation. For a proposed scope change, open /requirements and press p to review it first."}),
                )?;
                return Ok(());
            }
        };
        let values = specs
            .iter()
            .map(serde_json::to_value)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let tasks = self.store.add_plan(conversation, &values)?;
        let plan_id = tasks[0].plan_id.clone();
        self.store.link_plan_run(conversation, &plan_id, &run.id)?;
        if let Err(error) = self.store.bind_task_ledger(conversation, &run.id, &plan_id) {
            self.store
                .set_plan_status(&plan_id, status::PROPOSED, status::REVISED)?;
            self.notice(
                conversation,
                &format!("plan ledger mapping rejected: {error}"),
            );
            if self
                .store
                .count_since_user(conversation, "requirements-planner")?
                < tasks::MAX_ROUNDS as usize
            {
                self.store.add_message(
                    conversation,
                    "requirements-planner",
                    &format!(
                        "Regenerate the task plan from the verified ledger. {error}. {}",
                        self.ledger_context(conversation, &run.id)?
                    ),
                )?;
            }
            return Ok(());
        }
        // Capture the answered boundary before the verifier runs: comments
        // added during either turn have not been incorporated in this plan.
        let answered = self.store.last_orchestration_answered(conversation)?;
        let project_text = project.to_string_lossy().into_owned();
        let planning_gaps = planning_gaps(specs);
        if !config.policy.required_gates && !tasks::missing_quality_agents(specs).is_empty() {
            let _ = self.store.append_event(
                conversation,
                None,
                Some(&plan_id),
                "notice",
                &json!({"text": "policy.required_gates is off: this plan changes code but gets no \
                                 refactoring, static analysis, testing, security, performance \
                                 or integration check"}),
            );
        }
        self.store.append_event(
            conversation,
            None,
            Some(&plan_id),
            "planning_progress",
            &json!({"text": "Reviewing the plan"}),
        )?;
        let review = self.review(
            conversation,
            PLAN_VERIFIER,
            Some(&plan_id),
            project,
            cancel,
            |first, request| {
                let prompt = if first {
                    tasks::plan_review_prompt(&project_text, request, specs)
                } else {
                    tasks::plan_review_followup(request, specs)
                };
                match &planning_gaps {
                    Some(gaps) => format!("{prompt}\n\n{gaps}"),
                    None => prompt,
                }
            },
        )?;
        if cancel.load(Ordering::SeqCst) {
            self.store
                .set_plan_status(&plan_id, status::PROPOSED, status::CANCELLED)?;
            return Ok(());
        }
        if let Ok(verdict) = &review
            && !verdict.approved
            && self.store.count_since_user(conversation, PLAN_VERIFIER)?
                < tasks::MAX_ROUNDS as usize
        {
            self.store
                .set_plan_status(&plan_id, status::PROPOSED, status::REVISED)?;
            self.store.append_event(
                conversation,
                None,
                Some(&plan_id),
                "plan_review",
                &json!({"plan": plan_id, "approved": false, "findings": verdict.findings}),
            )?;
            self.store.append_event(
                conversation,
                None,
                Some(&plan_id),
                "planning_progress",
                &json!({"text": "Revising the plan after review"}),
            )?;
            // The orchestrator's loop answers this message next.
            self.store.add_message(
                conversation,
                PLAN_VERIFIER,
                &tasks::plan_rejected(&verdict.findings),
            )?;
            return Ok(());
        }
        let checked = review.as_ref().is_ok_and(|verdict| verdict.approved);
        if checked {
            // Options may have deferred this proposal and requested a fresh plan.
            // Keep the comment agent's selective claim until that plan is checked.
            let pending = self.store.pending_plan_comment_ids(conversation)?;
            self.store.resolve_listed_plan_comments(
                conversation,
                answered,
                addressed.or(pending.as_deref()),
            )?;
        }
        let autonomous =
            self.autonomous(conversation)? && self.store.workflow(conversation)?.is_none();
        let waiting = if !autonomous {
            None
        } else if !checked {
            Some("the plan verifier did not approve this plan")
        } else if self.store.count_since_user(conversation, RUNTIME)? > 0
            && self.repeats_a_plan_that_ran(conversation, &plan_id, specs)?
        {
            Some("the same plan already ran without you and did not close what was found")
        } else {
            None
        };
        let scope_hash = self.plan_scope_hash(conversation, &plan_id)?;
        let document = tasks::render_plan_document(text, specs);
        let parallel = tasks::parallel_peers(specs);
        self.store.append_event(
            conversation,
            None,
            Some(&plan_id),
            "plan_proposed",
            &json!({
                "plan": plan_id,
                "document": document,
                "parallel": parallel,
                "tasks": tasks.iter().map(|task| task.spec.clone()).collect::<Vec<_>>(),
                "review": review_json(&review),
                "automatic": autonomous && waiting.is_none(),
                "waiting": waiting,
                "scope_hash": scope_hash,
            }),
        )?;
        if autonomous && waiting.is_none() {
            self.accept_plan(conversation, &plan_id)?;
        }
        Ok(())
    }
}

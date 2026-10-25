use super::*;

#[path = "helpers/recording.rs"]
mod recording;
use recording::*;

impl Core {
    pub(super) fn prepare_plan(
        self: &Arc<Self>,
        conversation: &str,
        specs: &[TaskSpec],
        addressed: Option<&[u64]>,
        cancel: &AtomicBool,
    ) -> Result<Option<PreparedPlan>> {
        record_addressed_comments(&self.store, conversation, addressed)?;
        let run = match self.ensure_requirements(conversation, cancel) {
            Ok(run) => run,
            Err(error) => {
                record_requirements_failure(&self.store, conversation, &error)?;
                return Ok(None);
            }
        };
        let (tasks, id) = self.persist_proposed_tasks(conversation, specs)?;
        self.store.link_plan_run(conversation, &id, &run.id)?;
        if self.bind_proposed_plan(conversation, &id, &run.id)? {
            return Ok(None);
        }
        Ok(Some(PreparedPlan {
            id,
            tasks,
            answered: self.store.last_orchestration_answered(conversation)?,
        }))
    }

    fn persist_proposed_tasks(
        &self,
        conversation: &str,
        specs: &[TaskSpec],
    ) -> Result<(Vec<Task>, String)> {
        let values = specs
            .iter()
            .map(serde_json::to_value)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let tasks = self.store.add_plan(conversation, &values)?;
        let plan_id = tasks[0].plan_id.clone();
        Ok((tasks, plan_id))
    }

    fn bind_proposed_plan(&self, conversation: &str, plan: &str, run: &str) -> Result<bool> {
        let Err(error) = self.store.bind_task_ledger(conversation, run, plan) else {
            return Ok(false);
        };
        self.store
            .set_plan_status(plan, status::PROPOSED, status::REVISED)?;
        self.notice(
            conversation,
            &format!("plan ledger mapping rejected: {error}"),
        );
        self.retry_plan_from_ledger(conversation, run, &error)?;
        Ok(true)
    }

    fn retry_plan_from_ledger(
        &self,
        conversation: &str,
        run: &str,
        error: &dyn std::fmt::Display,
    ) -> Result<()> {
        if self
            .store
            .count_since_user(conversation, "requirements-planner")?
            >= tasks::MAX_ROUNDS as usize
        {
            return Ok(());
        }
        self.store.add_message(
            conversation,
            "requirements-planner",
            &format!(
                "Regenerate the task plan from the verified ledger. {error}. {}",
                self.ledger_context(conversation, run)?
            ),
        )?;
        Ok(())
    }

    pub(super) fn review_plan(
        self: &Arc<Self>,
        conversation: &str,
        project: &Path,
        plan: &PreparedPlan,
        specs: &[TaskSpec],
        config: &Config,
        cancel: &AtomicBool,
    ) -> Result<std::result::Result<Verdict, String>> {
        record_plan_review_start(&self.store, conversation, &plan.id, config, specs)?;
        let project_text = project.to_string_lossy().into_owned();
        let gaps = planning_gaps(specs);
        let dependency_findings = feasibility::check(specs);
        let dependency_note = dependency_findings
            .iter()
            .map(|finding| {
                format!(
                    "\n- {} [{}]: {}",
                    finding.task, finding.rule, finding.detail
                )
            })
            .collect::<String>();
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

    pub(super) fn cancel_proposed_plan(&self, plan: &str) -> Result<()> {
        self.store
            .set_plan_status(plan, status::PROPOSED, status::CANCELLED)?;
        Ok(())
    }

    pub(super) fn requeue_rejected_plan(
        &self,
        conversation: &str,
        plan: &str,
        review: &std::result::Result<Verdict, String>,
    ) -> Result<bool> {
        let Ok(verdict) = review else {
            return Ok(false);
        };
        if verdict.approved
            || self.store.count_since_user(conversation, PLAN_VERIFIER)?
                >= tasks::MAX_ROUNDS as usize
        {
            return Ok(false);
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

    pub(super) fn publish_plan(
        self: &Arc<Self>,
        conversation: &str,
        text: &str,
        addressed: Option<&[u64]>,
        plan: &PreparedPlan,
        specs: &[TaskSpec],
        review: &std::result::Result<Verdict, String>,
    ) -> Result<()> {
        let checked = review.as_ref().is_ok_and(|verdict| verdict.approved);
        self.resolve_plan_comments(conversation, addressed, plan, checked)?;
        let autonomous =
            self.autonomous(conversation)? && self.store.workflow(conversation)?.is_none();
        let waiting = self.plan_waiting_reason(conversation, plan, specs, autonomous, checked)?;
        let scope_hash = self.plan_scope_hash(conversation, &plan.id)?;
        self.store.append_event(
            conversation,
            None,
            Some(&plan.id),
            "plan_proposed",
            &plan_event_payload(text, plan, specs, review, autonomous, waiting, scope_hash)?,
        )?;
        if autonomous && waiting.is_none() {
            self.accept_plan(conversation, &plan.id)?;
        }
        Ok(())
    }

    fn resolve_plan_comments(
        &self,
        conversation: &str,
        addressed: Option<&[u64]>,
        plan: &PreparedPlan,
        checked: bool,
    ) -> Result<()> {
        if !checked {
            return Ok(());
        }
        let pending = self.store.pending_plan_comment_ids(conversation)?;
        self.store.resolve_listed_plan_comments(
            conversation,
            plan.answered,
            addressed.or(pending.as_deref()),
        )?;
        Ok(())
    }

    fn plan_waiting_reason(
        &self,
        conversation: &str,
        plan: &PreparedPlan,
        specs: &[TaskSpec],
        autonomous: bool,
        checked: bool,
    ) -> Result<Option<&'static str>> {
        if !autonomous {
            return Ok(None);
        }
        if !checked {
            return Ok(Some("the plan verifier did not approve this plan"));
        }
        if self.store.count_since_user(conversation, RUNTIME)? > 0
            && self.repeats_a_plan_that_ran(conversation, &plan.id, specs)?
        {
            return Ok(Some(
                "the same plan already ran without you and did not close what was found",
            ));
        }
        Ok(None)
    }
}
pub(super) struct PreparedPlan {
    pub(super) id: String,
    tasks: Vec<Task>,
    answered: u64,
}

fn record_addressed_comments(
    store: &Store,
    conversation: &str,
    addressed: Option<&[u64]>,
) -> Result<()> {
    if let Some(ids) = addressed {
        store.append_event(
            conversation,
            None,
            None,
            "plan_comments_addressed",
            &json!({"ids": ids, "upto_message": store.last_orchestration_answered(conversation)?}),
        )?;
    }
    Ok(())
}

fn record_requirements_failure(
    store: &Store,
    conversation: &str,
    error: &dyn std::fmt::Display,
) -> Result<()> {
    store.append_event(
        conversation,
        None,
        None,
        "requirements_failed",
        &json!({"error": error.to_string(), "retry": "Ask the orchestrator to regenerate the task plan; a bare approval does not rerun requirements preparation. For a proposed scope change, open /requirements and press p to review it first."}),
    )?;
    Ok(())
}

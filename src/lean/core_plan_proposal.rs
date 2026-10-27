use super::core_feedback_helpers::review_json;

#[path = "core_plan_proposal/helpers.rs"]
mod helpers;
use super::core_plan_helpers::{finalize_plan, planning_gaps};
use super::*;

impl Core {
    /// Stores the plan and has it checked. A plan the verifier sends back
    /// goes back to the orchestrator while review confirms progress; otherwise it
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
        if specs.len() > config.policy.max_tasks {
            let message = format!(
                "The plan has {} tasks; at most {} are allowed. Group the work by module or responsibility into fewer, larger tasks (the runtime adds options and quality gates itself) and return the complete plan again.",
                specs.len(),
                config.policy.max_tasks
            );
            if self.store.count_since_user(conversation, PLAN_LINT)?
                < tasks::FORMAT_REPAIR_ATTEMPTS as usize
            {
                self.store.add_message(conversation, PLAN_LINT, &message)?;
            } else {
                self.notice(conversation, &message);
            }
            return Ok(());
        }
        let specs = finalize_plan(
            specs.to_vec(),
            config.policy.required_gates,
            config.policy.local_options,
        );
        let findings = feasibility::check(&specs);
        let blocking: Vec<_> = findings
            .iter()
            .filter(|finding| finding.severity == feasibility::Severity::Block)
            .collect();
        if !blocking.is_empty() {
            let message = feasibility::message(&blocking);
            if self.store.count_since_user(conversation, PLAN_LINT)?
                < tasks::FORMAT_REPAIR_ATTEMPTS as usize
            {
                self.store.add_message(conversation, PLAN_LINT, &message)?;
            } else {
                self.notice(conversation, &message);
            }
            return Ok(());
        }
        let Some(plan) = self.prepare_plan(conversation, &specs, addressed, cancel)? else {
            return Ok(());
        };
        let review = self.review_plan(conversation, project, &plan, &specs, config, cancel)?;
        if cancel.load(Ordering::SeqCst) {
            self.cancel_proposed_plan(&plan.id)?;
            return Ok(());
        }
        if self.requeue_rejected_plan(conversation, &plan.id, &review)? {
            return Ok(());
        }
        self.publish_plan(conversation, text, addressed, &plan, &specs, &review)?;
        Ok(())
    }
}

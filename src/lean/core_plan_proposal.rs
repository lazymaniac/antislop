use super::core_feedback_helpers::review_json;

#[path = "core_plan_proposal/helpers.rs"]
mod helpers;
use super::core_plan_helpers::{finalize_plan, planning_gaps};
use super::*;

/// The plan the runtime runs: `specs` with its options and quality gates, if
/// it fits the policy and every task can be carried out as written. The
/// error is what the planner must change; `submit_plan` returns it at once.
pub(in crate::lean) fn checked_plan(
    specs: &[TaskContract],
    config: &Config,
) -> std::result::Result<Vec<TaskContract>, String> {
    if specs.len() > config.policy.max_tasks {
        return Err(format!(
            "The plan has {} tasks; at most {} are allowed. Group the work by module or responsibility into fewer, larger tasks (the runtime adds options and quality gates itself) and submit the complete plan again.",
            specs.len(),
            config.policy.max_tasks
        ));
    }
    let specs = finalize_plan(
        specs.to_vec(),
        config.policy.required_gates,
        config.policy.local_options,
    );
    let specs = tasks::order_graph(specs).map_err(|error| error.to_string())?;
    let findings = feasibility::check(&specs);
    let blocking: Vec<_> = findings
        .iter()
        .filter(|finding| finding.severity == feasibility::Severity::Block)
        .collect();
    if blocking.is_empty() {
        Ok(specs)
    } else {
        Err(feasibility::message(&blocking))
    }
}

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
        let Some(run) = self.planning_requirements(conversation, addressed, cancel)? else {
            return Ok(());
        };
        // Options and quality gates copy the links of the tasks they serve,
        // so `covers` is resolved before they are added.
        let Some(specs) = self.covered_plan(conversation, &run, specs)? else {
            return Ok(());
        };
        // `submit_plan` already refused a plan that fails these checks; a
        // stored answer from before a configuration change can still fail.
        let specs = match checked_plan(&specs, config) {
            Ok(specs) => specs,
            Err(message) => {
                if self.store.count_since_user(conversation, PLAN_LINT)?
                    < tasks::FORMAT_REPAIR_ATTEMPTS as usize
                {
                    self.store.add_message(conversation, PLAN_LINT, &message)?;
                } else {
                    self.notice(conversation, &message);
                }
                return Ok(());
            }
        };
        let Some(plan) = self.prepare_plan(conversation, &specs, &run)? else {
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

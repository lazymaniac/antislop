use crate::lean::core::task::feedback_helpers::review_json;
use crate::lean::ledger::Run;
use helpers::PreparedPlan;

#[path = "proposal/helpers.rs"]
mod helpers;
use crate::lean::core::plan::helpers::{finalize_team_plan, planning_gaps};
use crate::lean::*;

struct ProposalPublication<'a> {
    conversation: &'a str,
    project: &'a Path,
    text: &'a str,
    addressed: Option<&'a [u64]>,
    plan: &'a PreparedPlan,
    specs: &'a [TaskContract],
    config: &'a Config,
    cancel: &'a AtomicBool,
}

#[cfg(test)]
pub(in crate::lean) fn checked_plan(
    specs: &[TaskContract],
    config: &Config,
) -> std::result::Result<Vec<TaskContract>, String> {
    checked_team_plan(specs, config, tasks::Team::Feature)
}

/// The plan the runtime runs: `specs` with its options and quality gates, if
/// it fits the policy and `team`, and every task can be carried out as written. The
/// error is what the planner must change; `submit_plan` returns it at once.
pub(in crate::lean) fn checked_team_plan(
    specs: &[TaskContract],
    config: &Config,
    team: tasks::Team,
) -> std::result::Result<Vec<TaskContract>, String> {
    if let Some(misfit) = team.misfit(specs) {
        return Err(misfit);
    }
    let specs = finalize_team_plan(
        specs.to_vec(),
        config.policy.required_gates,
        config.policy.local_options,
        team,
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
    /// Stores the plan and has it checked. A plan the reviewer sends back
    /// goes back to the planner until review approves it. A checked plan
    /// waits for explicit user acceptance after review; failures and missing inputs remain visible.
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
        let Some((run, specs)) =
            self.prepare_proposal(conversation, addressed, cancel, specs, config)?
        else {
            return Ok(());
        };
        let Some(plan) = self.prepare_plan(conversation, &specs, &run)? else {
            return Ok(());
        };
        self.complete_proposal(ProposalPublication {
            conversation,
            project,
            text,
            addressed,
            plan: &plan,
            specs: &specs,
            config,
            cancel,
        })
    }

    fn prepare_proposal(
        self: &Arc<Self>,
        conversation: &str,
        addressed: Option<&[u64]>,
        cancel: &AtomicBool,
        specs: &[TaskContract],
        config: &Config,
    ) -> Result<Option<(Run, Vec<TaskContract>)>> {
        let Some(run) = self.planning_requirements(conversation, addressed, cancel)? else {
            return Ok(None);
        };
        let Some(specs) = self.covered_plan(conversation, &run, specs)? else {
            return Ok(None);
        };
        match checked_team_plan(&specs, config, self.store.team(conversation)?) {
            Ok(specs) => Ok(Some((run, specs))),
            Err(message) => {
                self.record_plan_lint(conversation, &message)?;
                Ok(None)
            }
        }
    }

    fn record_plan_lint(&self, conversation: &str, message: &str) -> Result<()> {
        self.store.add_message(conversation, PLAN_LINT, message)?;
        Ok(())
    }

    fn complete_proposal(self: &Arc<Self>, publication: ProposalPublication<'_>) -> Result<()> {
        let ProposalPublication {
            conversation,
            project,
            text,
            addressed,
            plan,
            specs,
            config,
            cancel,
        } = publication;
        let review = self.review_plan(conversation, project, plan, specs, config, cancel)?;
        if cancel.load(Ordering::SeqCst) {
            self.cancel_proposed_plan(&plan.id)?;
            return Ok(());
        }
        if self.requeue_rejected_plan(conversation, &plan.id, &review)? {
            return Ok(());
        }
        self.publish_plan(conversation, text, addressed, plan, specs, &review)?;
        // A cancel that arrived after the review but before the activated plan
        // registered its own flag would otherwise be lost.
        if cancel.load(Ordering::SeqCst) {
            self.cancel(conversation);
        }
        Ok(())
    }
}

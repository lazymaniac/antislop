use crate::lean::core::task::feedback_helpers::review_json;
use helpers::PreparedPlan;

#[path = "proposal/helpers.rs"]
mod helpers;
use crate::lean::core::plan::helpers::finalize_team_plan;
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
    let mut authored = specs.to_vec();
    if team == tasks::Team::CodeReview {
        validate_code_review_plan(&authored)?;
    } else {
        for task in &mut authored {
            if team != tasks::Team::NoQualityGates {
                task.integration_batch = false;
            }
            if tasks::is_report_only_review_task(task) {
                task.reviewer.summary = None;
            }
        }
    }
    let specs = authored.as_slice();
    if let Some(misfit) = team.misfit(specs) {
        return Err(misfit);
    }
    let mut specs = finalize_team_plan(
        specs.to_vec(),
        config.policy.required_gates,
        config.policy.local_options,
        team,
    );
    if team == tasks::Team::CodeReview {
        for task in &mut specs {
            task.reviewer.summary = Some(tasks::CODE_REVIEW_REVIEWER_SUMMARY.into());
        }
        if let Some(task) = specs.iter().find(|task| !is_review_only(task)) {
            return Err(format!(
                "code-review generated task `{}` that is not report-only; remove code delivery, options, quality gates and implementation work",
                task.id
            ));
        }
    }
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

fn validate_code_review_plan(
    specs: &[TaskContract],
) -> std::result::Result<&[TaskContract], String> {
    for task in specs {
        let reason = if task.change_kind() != tasks::ChangeKind::None {
            Some("must use change_kind=none")
        } else if task.quality_gate {
            Some("cannot be a quality gate")
        } else if task.options_for.is_some() {
            Some("cannot be an options task")
        } else if !is_review_kind(task.kind) {
            Some("must be a review or validation task, not implementation or delivery work")
        } else {
            None
        };
        if let Some(reason) = reason {
            return Err(format!(
                "code-review task `{}` {reason}; plan review must be report-only",
                task.id
            ));
        }
    }
    Ok(specs)
}

fn is_review_only(task: &TaskContract) -> bool {
    task.change_kind() == tasks::ChangeKind::None
        && !task.quality_gate
        && task.options_for.is_none()
        && is_review_kind(task.kind)
}

fn is_review_kind(kind: WorkKind) -> bool {
    matches!(
        kind,
        WorkKind::Analysis
            | WorkKind::Research
            | WorkKind::Architecture
            | WorkKind::Dependencies
            | WorkKind::StaticAnalysis
            | WorkKind::Testing
            | WorkKind::Security
            | WorkKind::Performance
            | WorkKind::Integration
    )
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
        let PlanProposal {
            config,
            specs,
            scope,
        } = proposal;
        let Some(specs) = self.prepare_proposal(conversation, addressed, cancel, specs, config)?
        else {
            return Ok(());
        };
        let plan = self.prepare_plan(conversation, &specs, scope)?;
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
        _cancel: &AtomicBool,
        specs: &[TaskContract],
        config: &Config,
    ) -> Result<Option<Vec<TaskContract>>> {
        helpers::record_addressed_comments(&self.store, conversation, addressed)?;
        match checked_team_plan(specs, config, self.store.team(conversation)?) {
            Ok(specs) => Ok(Some(specs)),
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
        let _graph = self.locks.graph();
        if self.requeue_rejected_plan(conversation, &plan.id, &review)? {
            return Ok(());
        }
        // A recommendation may have been replaced while plan review was
        // running. Never publish or accept its obsolete gate pipeline.
        if self
            .store
            .plan(&plan.id)?
            .iter()
            .any(|task| task.status != status::PROPOSED)
        {
            return Ok(());
        }
        let current_specs = checked_team_plan(specs, config, self.store.team(conversation)?)
            .map_err(Error::Invalid)?;
        if current_specs != specs {
            self.cancel_proposed_plan(&plan.id)?;
            self.record_plan_lint(conversation, "The user changed rigor during planning. Redraft with the selected rigor team and its gates.")?;
            return Ok(());
        }
        self.publish_plan(conversation, text, addressed, plan, specs, &review)?;
        drop(_graph);
        // A cancel that arrived after the review but before the activated plan
        // registered its own flag would otherwise be lost.
        if cancel.load(Ordering::SeqCst) {
            self.cancel(conversation);
        }
        Ok(())
    }
}

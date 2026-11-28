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

/// The plan the runtime runs: `specs` with its options and quality gates, if
/// it fits the policy and `team`, and every task can be carried out as written. The
/// error is what the planner must change; `submit_plan` returns it at once.
pub(in crate::lean) fn checked_team_plan(
    specs: &[TaskContract],
    config: &Config,
    team: tasks::Team,
) -> std::result::Result<Vec<TaskContract>, String> {
    checked_team_plan_preserving(specs, config, team, &Default::default())
}

fn checked_team_plan_preserving(
    specs: &[TaskContract],
    _config: &Config,
    team: tasks::Team,
    preserved: &std::collections::BTreeSet<String>,
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
    let mut specs = finalize_team_plan(specs.to_vec(), team);
    // A kept contract is immutable. Newly generated Options serve remaining writers.
    for task in &mut specs {
        if preserved.contains(&task.id)
            && let Some(original) = authored.iter().find(|old| old.id == task.id)
        {
            *task = original.clone();
        }
    }
    let used: std::collections::BTreeSet<_> = specs
        .iter()
        .flat_map(|t| t.depends_on.iter().cloned())
        .collect();
    specs.retain(|t| {
        t.options_for
            .as_ref()
            .is_none_or(|owner| !preserved.contains(owner) || used.contains(&t.id))
    });

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
    if team == tasks::Team::Feature {
        tasks::validate_feature_pipeline(&specs, preserved).map_err(|e| e.to_string())?;
    }
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
    /// Call under the graph lock, which also serializes plan starts.
    pub(in crate::lean) fn ensure_replacement_plan_allowed(
        &self,
        conversation: &str,
    ) -> Result<()> {
        require(
            !self.activity.plan_running(conversation),
            "a plan is already running; use revise_plan on the accepted plan with changed/added contracts, explicit retire entries and replacements for unfinished work. Include the complete scope when requirements change; the reviewed scope revision holds affected tasks until user acceptance. Do not assign proposed replacement contracts through worker answers",
        )
    }

    pub(in crate::lean) fn checked_candidate_plan(
        &self,
        conversation: &str,
        specs: &[TaskContract],
        team: tasks::Team,
        redo: &[String],
    ) -> Result<Vec<TaskContract>> {
        let history = self.store.approved_task_history(conversation)?;
        let generated: std::collections::BTreeSet<_> = history
            .iter()
            .filter(|t| t.spec["quality_gate"] == true || t.spec["options_for"].is_string())
            .filter_map(|t| t.spec["id"].as_str().map(str::to_owned))
            .chain(
                specs
                    .iter()
                    .filter(|t| t.quality_gate || t.options_for.is_some())
                    .map(|t| t.id.clone()),
            )
            .collect();
        let normalize = |mut spec: TaskContract| {
            spec.depends_on.retain(|id| !generated.contains(id));
            spec
        };
        let preserved = specs
            .iter()
            .filter(|spec| {
                !redo.contains(&spec.id)
                    && history.iter().any(|task| {
                        task.status == status::APPROVED
                            && serde_json::from_value::<TaskContract>(task.spec.clone())
                                .is_ok_and(|old| normalize(old) == normalize((*spec).clone()))
                    })
            })
            .map(|t| t.id.clone())
            .collect();
        let specs = checked_team_plan_preserving(specs, &self.config_snapshot(), team, &preserved)
            .map_err(Error::Invalid)?;
        for task in specs.iter().filter(|task| !preserved.contains(&task.id)) {
            self.check_planning_task_inputs(conversation, PLANNER, "", task)?;
        }
        Ok(specs)
    }

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
            keep,
            config,
            specs,
            scope,
        } = proposal;
        let (specs, plan) = {
            // Selection either precedes finalization or updates these same rows.
            let _graph = self.locks.graph();
            let Some(specs) =
                self.prepare_proposal(conversation, addressed, cancel, specs, config)?
            else {
                return Ok(());
            };
            let plan = self.prepare_plan(conversation, &specs, scope, keep)?;
            (specs, plan)
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
        _cancel: &AtomicBool,
        specs: &[TaskContract],
        _config: &Config,
    ) -> Result<Option<Vec<TaskContract>>> {
        self.ensure_replacement_plan_allowed(conversation)?;
        helpers::record_addressed_comments(&self.store, conversation, addressed)?;
        match self.checked_candidate_plan(conversation, specs, self.store.team(conversation)?, &[])
        {
            Ok(specs) => Ok(Some(specs)),
            Err(message) => {
                self.record_plan_lint(conversation, &message.to_string())?;
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
        self.publish_plan(conversation, text, addressed, plan, &review)?;
        drop(_graph);
        // A cancel that arrived after the review but before the activated plan
        // registered its own flag would otherwise be lost.
        if cancel.load(Ordering::SeqCst) {
            self.cancel(conversation);
        }
        Ok(())
    }
}

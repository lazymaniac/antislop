//! Team templates: which quality gates and option tasks a plan of a kind of
//! work gets. The planner names one when it submits a plan, says why, and
//! the runtime builds the team from it; a plan that names none is a feature.

use super::{GATE_KINDS, TaskContract};
use crate::config::WorkKind;
use serde::{Deserialize, Serialize};

/// Bumped when a template's gates or options change, so a recorded choice
/// says which definition it meant.
pub const TEAM_VERSION: u32 = 3;

/// Runtime-owned marker used to specialize workers and reviewers for the
/// report-only code-review workflow. Planner-authored summaries cannot enable
/// this mode; plan validation installs it from the selected team.
pub(crate) const CODE_REVIEW_REVIEWER_SUMMARY: &str =
    "Code review: independently validate findings and report scope, verdict, evidence and limits.";

pub(crate) const CODE_REVIEW_PLANNING_RULES: &str = "\nSelected rigor: `code-review`. Plan a read-only review of the exact target derived from the user's request. Use an explicitly named commit/range, files, symbols or component when provided; use the current working-tree diff only when the request clearly asks to review current/uncommitted changes; use repository-wide scope only when explicitly requested. If no target or diff can be inferred, do not guess or submit a plan: finish_turn with the missing scope and ask the orchestrator to clarify it with the user. Plan only report-only review and validation tasks with change_kind=none. Every task must inspect the same declared target, must not modify/create/delete project files, and must not repair findings. Do not add implementation, refactoring, options, artifact/report-writing, quality-gate, or code-changing tasks. The final report task, if any, must depend on all review and validation tasks. Require results to state reviewed scope, overall verdict, prioritized confirmed findings (severity, path and line, trigger, impact, evidence, remediation), unresolved questions, commands/checks with actual outcomes, limitations and coverage gaps. No confirmed findings is a valid outcome.";

pub(crate) const NO_QUALITY_GATES_PLANNING_RULES: &str = "\nSelected rigor: `no-quality-gates`. Plan the requested code and artifact work as ordinary tasks with meaningful acceptance criteria and real dependency edges. Do not add quality-gate tasks or options tasks; the runtime ignores global required-gates and local-options policy for this mode. The runtime adds one final integration task after all code-producing tasks and integrates their accepted results on the branch. Do not add another integration task or a final quality audit. Task-local acceptance and any explicitly requested validation remain ordinary planned work.";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Team {
    /// A contained fix or tweak: options where policy allows them, static
    /// analysis and tests.
    SmallChange,
    /// New behaviour: every gate, options where policy allows them.
    #[default]
    Feature,
    /// Behaviour kept, structure changed.
    Refactoring,
    /// A vulnerability closed.
    SecurityFix,
    /// Code changes reviewed without generated quality gates or option tasks.
    CodeReview,
    /// Requested work with ordinary task acceptance and one final integration.
    NoQualityGates,
    /// Findings, no code change.
    Research,
    /// A document or other artifact, no code change.
    Document,
}

impl Team {
    pub const ALL: [Team; 8] = [
        Team::SmallChange,
        Team::Feature,
        Team::Refactoring,
        Team::SecurityFix,
        Team::CodeReview,
        Team::NoQualityGates,
        Team::Research,
        Team::Document,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Team::SmallChange => "small-change",
            Team::Feature => "feature",
            Team::Refactoring => "refactoring",
            Team::SecurityFix => "security-fix",
            Team::CodeReview => "code-review",
            Team::NoQualityGates => "no-quality-gates",
            Team::Research => "research",
            Team::Document => "document",
        }
    }

    pub fn parse(name: &str) -> crate::Result<Self> {
        Team::ALL
            .into_iter()
            .find(|team| team.name() == name)
            .ok_or_else(|| {
                crate::Error::Invalid(format!(
                    "unknown team `{name}`; choose one of: {}",
                    Team::ALL.map(Team::name).join(", ")
                ))
            })
    }

    /// The gates the runtime adds after a code change, in pipeline order.
    pub fn gates(self) -> &'static [WorkKind] {
        use WorkKind::*;
        match self {
            Team::SmallChange => &[StaticAnalysis, Testing],
            Team::Feature => &GATE_KINDS,
            Team::Refactoring => &[Refactoring, StaticAnalysis, Testing, Integration],
            Team::SecurityFix => &[StaticAnalysis, Testing, Security, Integration],
            Team::CodeReview => &[],
            Team::NoQualityGates => &[],
            Team::Research | Team::Document => &[],
        }
    }

    /// Whether writers get option tasks (alternatives compared before the
    /// change), when policy allows them at all.
    pub fn options(self) -> bool {
        matches!(self, Team::SmallChange | Team::Feature | Team::Refactoring)
    }

    /// What the planner reads about the template.
    pub fn summary(self) -> String {
        let gates = if self.gates().is_empty() {
            "no gates".to_owned()
        } else {
            self.gates()
                .iter()
                .map(|kind| super::work_kind_name(*kind))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let use_for = match self {
            Team::SmallChange => "a contained fix or tweak in a few files",
            Team::Feature => "new behaviour",
            Team::Refactoring => "structure changed, behaviour kept",
            Team::SecurityFix => "closing a vulnerability",
            Team::CodeReview => "reviewing code changes without generated gates",
            Team::NoQualityGates => {
                "requested work with ordinary acceptance and final integration only"
            }
            Team::Research => "findings without code changes",
            Team::Document => "a document or other artifact, no code",
        };
        format!(
            "`{}`: {use_for}; gates: {gates}; options: {}",
            self.name(),
            if self.options() {
                "when local options are enabled"
            } else {
                "no"
            }
        )
    }

    /// Why `tasks` cannot be run by this team, if they cannot.
    pub fn misfit(self, tasks: &[TaskContract]) -> Option<String> {
        let code = tasks.iter().find(|task| task.changes_code())?;
        matches!(self, Team::Research | Team::Document).then(|| {
            format!(
                "team `{}` changes no code, but task `{}` does; choose `small-change`, `feature`, `refactoring`, `security-fix`, `code-review` or `no-quality-gates`",
                self.name(),
                code.id
            )
        })
    }
}

pub(in crate::lean) fn is_report_only_review_task(task: &TaskContract) -> bool {
    task.reviewer.summary.as_deref() == Some(CODE_REVIEW_REVIEWER_SUMMARY)
}

/// Preserve requested work while adding only the runtime-owned final landing
/// task. Global gate policy and local-options policy do not apply to this team.
pub(crate) fn finalize_no_quality_gates(mut specs: Vec<TaskContract>) -> Vec<TaskContract> {
    specs = super::without_options(specs);
    for spec in &mut specs {
        spec.quality_gate = false;
    }
    let producers: Vec<_> = specs
        .iter()
        .filter(|spec| spec.change_kind() == super::ChangeKind::Code && !spec.integration_batch)
        .map(|spec| spec.id.clone())
        .collect();
    let deferred = specs.iter().any(|spec| {
        spec.change_kind() == super::ChangeKind::Code && spec.deferred && !spec.integration_batch
    });
    if producers.is_empty() {
        specs.retain(|spec| !spec.integration_batch);
        return specs;
    }
    if let Some(finalizer) = specs.iter_mut().find(|spec| spec.integration_batch) {
        finalizer.depends_on = producers;
        finalizer.deferred = deferred;
        return super::order_graph(specs.clone()).unwrap_or(specs);
    }
    let mut id = "final-integration".to_owned();
    let mut suffix = 2;
    while specs.iter().any(|spec| spec.id == id) {
        id = format!("final-integration-{suffix}");
        suffix += 1;
    }
    specs.push(TaskContract {
        quality_gate: false,
        integration_batch: true,
        options_for: None,
        id,
        checkout_name: Some(super::checkout_slug("Integrate delivered changes")),
        acceptance_criteria: vec![super::AcceptanceCriterion {
            id: "C1".into(),
            text: "All accepted code changes from this plan are integrated on the branch".into(),
        }],
        title: "Integrate delivered changes".into(),
        goal: "Integrate every accepted code change from this plan on the branch, resolving actual conflicts through the existing repair flow and reporting failures.".into(),
        summary: Some("Integrate accepted code changes after all producers finish.".into()),
        section: None,
        deferred,
        change_kind: super::ChangeKind::Code,
        outputs: Vec::new(),
        delivery: None,
        depends_on: producers,
        kind: crate::config::WorkKind::Integration,
        reviewer: super::ReviewerSpec::default(),
    });
    super::order_graph(specs.clone()).unwrap_or(specs)
}

/// The teams, one line each, for planner instructions.
pub fn team_catalog() -> String {
    Team::ALL
        .map(|team| format!("- {}", team.summary()))
        .join("\n")
}

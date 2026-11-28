//! Team templates: which quality gates and option tasks a plan of a kind of
//! work gets. The planner names one when it submits a plan, says why, and
//! the runtime builds the team from it; a plan that names none is a feature.

use super::{GATE_KINDS, TaskContract};
use crate::config::WorkKind;
use crate::lean::prompts;
use serde::{Deserialize, Serialize};

/// Bumped when a template's gates or options change, so a recorded choice
/// says which definition it meant.
pub const TEAM_VERSION: u32 = 4;

/// Runtime-owned marker used to specialize workers and reviewers for the
/// report-only code-review workflow. Planner-authored summaries cannot enable
/// this mode; plan validation installs it from the selected team.
pub(crate) const CODE_REVIEW_REVIEWER_SUMMARY: &str = prompts::CODE_REVIEW_REVIEWER_SUMMARY;

pub(crate) const CODE_REVIEW_PLANNING_RULES: &str = prompts::CODE_REVIEW_PLANNING_RULES;

pub(crate) const NO_QUALITY_GATES_PLANNING_RULES: &str = prompts::NO_QUALITY_GATES_PLANNING_RULES;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Team {
    /// A contained fix or tweak: options, static
    /// analysis and tests.
    SmallChange,
    /// New behaviour: every gate, options.
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
    /// change), for the selected rigor.
    pub fn options(self) -> bool {
        matches!(self, Team::SmallChange | Team::Feature | Team::Refactoring)
    }

    /// What the planner reads about the template.
    pub fn summary(self) -> String {
        let gates = if self.gates().is_empty() {
            prompts::TEAM_NO_GATES.to_owned()
        } else {
            self.gates()
                .iter()
                .map(|kind| super::work_kind_name(*kind))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let use_for = match self {
            Team::SmallChange => prompts::SMALL_CHANGE_USE_CASE,
            Team::Feature => prompts::FEATURE_USE_CASE,
            Team::Refactoring => prompts::REFACTORING_USE_CASE,
            Team::SecurityFix => prompts::SECURITY_FIX_USE_CASE,
            Team::CodeReview => prompts::CODE_REVIEW_USE_CASE,
            Team::NoQualityGates => prompts::NO_QUALITY_GATES_USE_CASE,
            Team::Research => prompts::RESEARCH_USE_CASE,
            Team::Document => prompts::DOCUMENT_USE_CASE,
        };
        format!(
            prompts::template!(TEAM_SUMMARY),
            self.name(),
            if self.options() {
                prompts::TEAM_OPTIONS_DESCRIPTION
            } else {
                "no"
            },
            use_for = use_for,
            gates = gates
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
/// task. This rigor has no Options or final quality gates.
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
        open_decisions: None,
        planning_inputs: Vec::new(),
        id,
        checkout_name: Some(super::checkout_slug(prompts::INTEGRATION_TASK_TITLE)),
        acceptance_criteria: vec![super::AcceptanceCriterion {
            id: "C1".into(),
            text: prompts::INTEGRATION_ACCEPTANCE_CRITERION.into(),
        }],
        title: prompts::INTEGRATION_TASK_TITLE.into(),
        goal: prompts::INTEGRATION_TASK_GOAL.into(),
        summary: Some(prompts::INTEGRATION_TASK_SUMMARY.into()),
        section: None,
        deferred,
        change_kind: super::ChangeKind::Code,
        outputs: Vec::new(),
        delivery: None,
        result_inputs: Vec::new(),
        depends_on: producers,
        kind: crate::config::WorkKind::Integration,
        reviewer: super::ReviewerSpec::default(),
    });
    super::order_graph(specs.clone()).unwrap_or(specs)
}

/// The teams, one line each, for planner instructions.
pub fn team_catalog() -> String {
    Team::ALL
        .map(|team| {
            format!(
                crate::lean::prompts::template!(TEAM_CATALOG_ENTRY),
                team.summary()
            )
        })
        .join("\n")
}

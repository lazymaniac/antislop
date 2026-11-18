//! Team templates: which quality gates and option tasks a plan of a kind of
//! work gets. The planner names one when it submits a plan, says why, and
//! the runtime builds the team from it; a plan that names none is a feature.

use super::{GATE_KINDS, TaskContract};
use crate::config::WorkKind;
use serde::{Deserialize, Serialize};

/// Bumped when a template's gates or options change, so a recorded choice
/// says which definition it meant.
pub const TEAM_VERSION: u32 = 2;

/// Runtime-owned marker used to specialize workers and reviewers for the
/// report-only code-review workflow. Planner-authored summaries cannot enable
/// this mode; plan validation installs it from the selected team.
pub(crate) const CODE_REVIEW_REVIEWER_SUMMARY: &str =
    "Code review: independently validate findings and report scope, verdict, evidence and limits.";

pub(crate) const CODE_REVIEW_PLANNING_RULES: &str = "\nSelected rigor: `code-review`. Plan a read-only review of the exact target derived from the user's request. Use an explicitly named commit/range, files, symbols or component when provided; use the current working-tree diff only when the request clearly asks to review current/uncommitted changes; use repository-wide scope only when explicitly requested. If no target or diff can be inferred, do not guess or submit a plan: finish_turn with the missing scope and ask the orchestrator to clarify it with the user. Plan only report-only review and validation tasks with change_kind=none. Every task must inspect the same declared target, must not modify/create/delete project files, and must not repair findings. Do not add implementation, refactoring, options, artifact/report-writing, quality-gate, or code-changing tasks. The final report task, if any, must depend on all review and validation tasks. Require results to state reviewed scope, overall verdict, prioritized confirmed findings (severity, path and line, trigger, impact, evidence, remediation), unresolved questions, commands/checks with actual outcomes, limitations and coverage gaps. No confirmed findings is a valid outcome.";

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
    /// Findings, no code change.
    Research,
    /// A document or other artifact, no code change.
    Document,
}

impl Team {
    pub const ALL: [Team; 7] = [
        Team::SmallChange,
        Team::Feature,
        Team::Refactoring,
        Team::SecurityFix,
        Team::CodeReview,
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
                "team `{}` changes no code, but task `{}` does; choose `small-change`, `feature`, `refactoring`, `security-fix` or `code-review`",
                self.name(),
                code.id
            )
        })
    }
}

pub(in crate::lean) fn is_report_only_review_task(task: &TaskContract) -> bool {
    task.reviewer.summary.as_deref() == Some(CODE_REVIEW_REVIEWER_SUMMARY)
}

/// The teams, one line each, for planner instructions.
pub fn team_catalog() -> String {
    Team::ALL
        .map(|team| format!("- {}", team.summary()))
        .join("\n")
}

//! Team templates: which quality gates and option tasks a plan of a kind of
//! work gets. The planner names one when it submits a plan, says why, and
//! the runtime builds the team from it; a plan that names none is a feature.

use super::{GATE_KINDS, TaskContract};
use crate::config::WorkKind;
use serde::{Deserialize, Serialize};

/// Bumped when a template's gates or options change, so a recorded choice
/// says which definition it meant.
pub const TEAM_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Team {
    /// A contained fix or tweak: static analysis and tests, no options, no
    /// context scout.
    SmallChange,
    /// New behaviour: every gate, options where policy allows them.
    #[default]
    Feature,
    /// Behaviour kept, structure changed.
    Refactoring,
    /// A vulnerability closed.
    SecurityFix,
    /// Findings, no code change.
    Research,
    /// A document or other artifact, no code change.
    Document,
}

impl Team {
    pub const ALL: [Team; 6] = [
        Team::SmallChange,
        Team::Feature,
        Team::Refactoring,
        Team::SecurityFix,
        Team::Research,
        Team::Document,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Team::SmallChange => "small-change",
            Team::Feature => "feature",
            Team::Refactoring => "refactoring",
            Team::SecurityFix => "security-fix",
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
            Team::Research | Team::Document => &[],
        }
    }

    /// Whether writers get option tasks (alternatives compared before the
    /// change), when policy allows them at all.
    pub fn options(self) -> bool {
        matches!(self, Team::Feature | Team::Refactoring)
    }

    /// Whether a code change must be preceded by a context scout.
    pub fn needs_context_scout(self) -> bool {
        self != Team::SmallChange
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
            Team::Research => "findings without code changes",
            Team::Document => "a document or other artifact, no code",
        };
        format!(
            "`{}`: {use_for}; gates: {gates}; options: {}{}",
            self.name(),
            if self.options() { "yes" } else { "no" },
            if self.needs_context_scout() {
                ""
            } else {
                "; no context scout required"
            }
        )
    }

    /// Why `tasks` cannot be run by this team, if they cannot.
    pub fn misfit(self, tasks: &[TaskContract]) -> Option<String> {
        let code = tasks.iter().find(|task| task.changes_code())?;
        self.gates().is_empty().then(|| {
            format!(
                "team `{}` changes no code, but task `{}` does; choose `small-change`, `feature`, `refactoring` or `security-fix`",
                self.name(),
                code.id
            )
        })
    }
}

/// The teams, one line each, for planner instructions.
pub fn team_catalog() -> String {
    Team::ALL
        .map(|team| format!("- {}", team.summary()))
        .join("\n")
}

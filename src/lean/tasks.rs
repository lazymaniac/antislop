//! Plans and worker/reviewer pairs. Workers solve a task; each reviewer judges
//! its worker's result, knowing only the risks of that worker's domain.

use crate::{Error, Result, config::WorkKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use unicode_segmentation::UnicodeSegmentation;

#[path = "tasks/agents.rs"]
mod agents;
#[path = "tasks/bounds.rs"]
mod bounds;
#[path = "tasks/effect.rs"]
mod effect;
#[path = "tasks/feedback.rs"]
mod feedback;
#[path = "tasks/graph.rs"]
mod graph;
#[path = "tasks/prompts/instructions.rs"]
mod instructions;
#[path = "tasks/messages.rs"]
mod messages;
#[path = "tasks/plan.rs"]
mod plan;
#[path = "tasks/prompts/plan_review.rs"]
mod plan_review;
#[path = "tasks/prompts/requirements.rs"]
mod requirements;
#[path = "tasks/prompts/review.rs"]
mod review;
#[path = "tasks/validation.rs"]
mod validation;
#[path = "tasks/prompts/worker.rs"]
mod worker;
pub use agents::*;
pub use bounds::*;
pub use effect::*;
#[cfg(test)]
use feedback::OUTCOMES;
pub(crate) use feedback::counted_status;
pub use feedback::*;
pub use graph::*;
pub use instructions::*;
pub use messages::*;
pub use plan::*;
pub use plan_review::*;
pub use requirements::*;
use requirements::{COVERAGE_FORMAT, OUTSIDE, PROJECT_RULES, REVIEW_RULES};
pub use review::*;
use validation::obligations_text;
pub use validation::*;
#[cfg(test)]
use worker::dependency_metadata;
pub use worker::*;

/// The delivered change, independent of filesystem access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    None,
    Artifact,
    Code,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMode {
    Untracked,
}

/// A task as the runtime stores it. Agents never write this form: plans arrive
/// as [`PlannedTask`]s and the runtime fills in the rest. Fields of earlier
/// contracts (declared inputs, reviewer focus and risks) are ignored, so
/// stored plans keep loading. Legacy write access infers the change kind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskContract {
    #[serde(default)]
    pub quality_gate: bool,
    #[serde(default)]
    pub options_for: Option<String>,
    pub id: String,
    #[serde(default)]
    pub requirement_ids: Vec<String>,
    #[serde(default)]
    pub criterion_ids: Vec<String>,
    pub title: String,
    pub goal: String,
    /// One concise sentence describing the worker's task on the graph.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Optional section heading in the plan document that this task implements.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    #[serde(default)]
    pub writes: bool,
    /// Missing in legacy contracts: infer none/code from writes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_kind: Option<ChangeKind>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<DeliveryMode>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    pub kind: WorkKind,
    /// Requirement or criterion ids the planner says this task serves; the
    /// runtime resolves them into `requirement_ids` and `criterion_ids`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub covers: Vec<String>,
    #[serde(default)]
    pub reviewer: ReviewerSpec,
}

impl TaskContract {
    pub fn change_kind(&self) -> ChangeKind {
        self.change_kind.unwrap_or(if self.writes {
            ChangeKind::Code
        } else {
            ChangeKind::None
        })
    }
    pub fn changes_code(&self) -> bool {
        self.change_kind() == ChangeKind::Code
    }

    pub(crate) fn validate_change_paths(&self, paths: &[String]) -> Result<()> {
        if self.change_kind() == ChangeKind::Artifact {
            for path in paths {
                crate::require(
                    self.outputs.contains(path),
                    &format!("task {}: artifact change outside outputs: {path}", self.id),
                )?;
            }
        } else if self.change_kind() == ChangeKind::None {
            crate::require(
                paths.is_empty(),
                &format!(
                    "task {}: change_kind none cannot deliver file changes",
                    self.id
                ),
            )?;
        }
        Ok(())
    }
}

/// What the graph shows about a task's reviewer. Its obligations follow from
/// the task itself: the goal, the assigned criteria and its kind's risk catalog.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReviewerSpec {
    /// One concise sentence describing what the reviewer checks on the graph.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    pub approved: bool,
    #[serde(default)]
    pub findings: Vec<Value>,
    /// A task reviewer's account of each of its obligations, by id (`R1`,
    /// `V1`, `C2`): `{"status": "ok|issue|blocked|n/a", "note": "..."}`.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub coverage: serde_json::Map<String, Value>,
    /// Ids of the previous round's findings that the reviewer confirms the
    /// updated work fixes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resolved: Vec<String>,
}

pub const FORMAT_REPAIR_ATTEMPTS: u32 = 3;

pub mod status {
    pub const STALE: &str = "stale";
    pub const PROPOSED: &str = "proposed";
    pub const QUEUED: &str = "queued";
    pub const WORKING: &str = "working";
    pub const REVIEWING: &str = "reviewing";
    pub const APPROVED: &str = "approved";
    pub const COORDINATING: &str = "coordinating";
    pub const NEEDS_USER: &str = "needs_user";
    pub const FAILED: &str = "failed";
    pub const SKIPPED: &str = "skipped";
    pub const CANCELLED: &str = "cancelled";
    /// A plan the plan reviewer sent back; the planner corrects it.
    pub const REVISED: &str = "revised";
    /// The agent's CLI needs the user: a login, or a usage limit to reset.
    pub const WAITING: &str = "waiting";

    pub fn finished(status: &str) -> bool {
        matches!(
            status,
            STALE
                | APPROVED
                | COORDINATING
                | NEEDS_USER
                | FAILED
                | SKIPPED
                | CANCELLED
                | REVISED
                | WAITING
        )
    }
}

const OPTIONS_BINDING: &str = "Implement the selected_approach input for this task. It is a reviewed design decision, never permission to expand scope. Ask the orchestrator if a premise fails or a different approach is necessary; a changed selection requires fresh options and review. Never silently switch variants.";

/// An options task inherits its writer's criteria; what the writer builds
/// and proves is checked on the writer, not on the selection.
const SELECTION_REVIEW: &str = "This is an options task: its deliverable is the selection of an approach, not an implementation. Judge each acceptance criterion by whether the selected variant can satisfy it and the selection does not rule it out. Criteria about the tests, validation commands, measurements or other evidence the dependent writer must deliver are checked when the writer's work is reviewed: mark them n/a here unless the selected variant makes them impossible. Do not ask the report to plan tests or list validation commands.";
const REPORT_REVIEW: &str = "This task delivers an analysis report, optionally in declared document files. Failing checks in the inspected project are evidence to describe, not an obligation to fix the project. Judge whether each reported problem is real and backed by the cited code, and whether the report inspected the paths its goal names. A report that states which paths it did not inspect is not incomplete for that alone. Raise a finding only for a concrete problem in this task's scope that the report missed or misstated, with its path and line, or for a path the goal names that the report did not inspect. List every such finding in this round and hold none back.";
/// A report reviewer that finds one more old gap every round never lets a
/// report task finish, so later rounds block only on what the round changed.
const REPORT_RECHECK: &str = "In this later round, a blocking finding (medium or above) is only an earlier finding above that the updated report did not fix, or a problem in text the worker added or changed since the previous report. A gap in the code or report that was already there in the previous round and that you did not name then is not blocking now: record it in your report or as a low finding.";
const OPTIONS_REVIEW: &str = "Independently compare the implementation with selected_approach and this task's local requirements. Review actual files and recorded checks. A changed approach requires reviewed options; the worker's declaration is not proof. Ask the orchestrator about missing or inconsistent input. Do not reassess other tasks or the whole session.";

#[cfg(test)]
#[path = "tasks/tests.rs"]
mod tests;

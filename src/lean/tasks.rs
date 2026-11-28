//! Plans and worker/reviewer pairs. Workers solve a task; each reviewer judges
//! its worker's result, knowing only the risks of that worker's domain.

use crate::{Error, Result, config::WorkKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use unicode_segmentation::UnicodeSegmentation;

#[path = "tasks/agents.rs"]
mod agents;
#[path = "tasks/bounds.rs"]
mod bounds;
#[path = "tasks/effect.rs"]
mod effect;
#[path = "tasks/feature.rs"]
mod feature;
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
#[path = "tasks/templates.rs"]
mod templates;
#[path = "tasks/validation.rs"]
mod validation;
#[path = "tasks/prompts/worker.rs"]
mod worker;
pub use agents::*;
pub use bounds::*;
pub use effect::*;
pub use feature::*;
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
pub use templates::*;
use validation::obligations_text;
pub use validation::*;
#[cfg(test)]
use worker::dependency_metadata;
pub use worker::*;

/// The delivered change, independent of filesystem access.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    #[default]
    None,
    Artifact,
    Code,
}

impl ChangeKind {
    /// Whether the result includes changed files, independent of CLI access.
    pub fn has_delivery(self) -> bool {
        self != Self::None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMode {
    Untracked,
}

/// One mandatory, task-local acceptance check. Conditions belong in its text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptanceCriterion {
    pub id: String,
    pub text: String,
}

/// An exact historical report used as context, without approval or scheduling edges.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResultInput {
    pub alias: String,
    pub result_ref: String,
}

/// A task as the runtime stores it, with an explicit delivery purpose.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskContract {
    #[serde(default)]
    pub quality_gate: bool,
    /// Runtime-generated final assembler; planner-authored values are cleared.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub integration_batch: bool,
    #[serde(default)]
    pub options_for: Option<String>,
    /// Absent in saved legacy graphs, which retain their existing Options policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_decisions: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub planning_inputs: Vec<String>,
    pub id: String,
    /// Stable lowercase ASCII label (letters, digits and hyphens, 1–48 bytes)
    /// for a writer checkout. New plans freeze a slug of the task title when
    /// omitted; retained older contracts keep their ID-based paths.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkout_name: Option<String>,
    #[serde(default)]
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
    pub title: String,
    pub goal: String,
    /// One concise sentence describing the worker's task on the graph.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Optional section heading in the plan document that this task implements.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    /// Future work kept in the graph but never scheduled until a reviewed revision releases it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deferred: bool,
    pub change_kind: ChangeKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<DeliveryMode>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub result_inputs: Vec<ResultInput>,
    pub kind: WorkKind,
    #[serde(default)]
    pub reviewer: ReviewerSpec,
}

pub(crate) fn valid_checkout_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 48
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !name.starts_with('-')
        && !name.ends_with('-')
}

impl TaskContract {
    pub(crate) fn require_not_deferred(&self) -> Result<()> {
        crate::require(
            !self.deferred,
            &format!(
                "task {} execution is deferred; release it through a reviewed revision to make it current",
                self.id
            ),
        )
    }

    pub fn change_kind(&self) -> ChangeKind {
        self.change_kind
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
#[serde(deny_unknown_fields)]
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
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub resolution_evidence: std::collections::BTreeMap<String, ScenarioEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioEvidence {
    pub scenario: String,
    pub validation: String,
}

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

const OPTIONS_BINDING: &str = crate::lean::prompts::OPTIONS_BINDING;

/// An options task inherits its writer's criteria; what the writer builds
/// and proves is checked on the writer, not on the selection.
const SELECTION_REVIEW: &str = crate::lean::prompts::SELECTION_REVIEW;
const REPORT_REVIEW: &str = crate::lean::prompts::REPORT_REVIEW;
const OPTIONS_REVIEW: &str = crate::lean::prompts::OPTIONS_REVIEW;
const GATE_REVIEW: &str = crate::lean::prompts::GATE_REVIEW;

#[cfg(test)]
#[path = "tasks/tests.rs"]
mod tests;

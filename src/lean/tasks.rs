//! Plans and worker/verifier pairs. Workers solve a task; each verifier judges
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
#[path = "tasks/prompts/checklist.rs"]
mod checklist;
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
pub use checklist::*;
pub use effect::*;
#[cfg(test)]
use feedback::OUTCOMES;
pub(crate) use feedback::counted_status;
pub use feedback::*;
pub use graph::*;
pub use instructions::*;
pub use messages::*;
pub use plan::*;
pub use requirements::*;
use requirements::{COVERAGE_FORMAT, OUTSIDE, PROJECT_RULES, REVIEW_RULES, scope_text};
pub use review::*;
use validation::obligations_text;
pub use validation::*;
#[cfg(test)]
use worker::dependency_metadata;
pub use worker::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskInput {
    pub name: String,
    pub task: String,
    pub output: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskContract {
    #[serde(default)]
    pub quality_gate: bool,
    #[serde(default)]
    pub inputs: Vec<TaskInput>,
    #[serde(default)]
    pub outputs: Vec<String>,
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
    /// Directories or files that matter: the project's own, or related ones
    /// such as a service it talks to. Agents read them themselves.
    #[serde(default)]
    pub scope: Vec<String>,
    #[serde(default)]
    pub writes: bool,
    #[serde(default)]
    pub depends_on: Vec<String>,
    pub kind: WorkKind,
    pub verifier: VerifierSpec,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifierSpec {
    #[serde(default)]
    pub risk_ids: Vec<String>,
    pub focus: String,
    /// One concise sentence describing what the verifier checks on the graph.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default)]
    pub risks: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    pub approved: bool,
    #[serde(default)]
    pub findings: Vec<Value>,
    /// A task verifier's account of each of its obligations, by id (`R1`,
    /// `V1`, `C2`): `{"status": "ok|issue|blocked|n/a", "note": "..."}`.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub coverage: serde_json::Map<String, Value>,
}

pub const FORMAT_REPAIR_ATTEMPTS: u32 = 3;

pub mod status {
    pub const STALE: &str = "stale";
    pub const PROPOSED: &str = "proposed";
    pub const QUEUED: &str = "queued";
    pub const WORKING: &str = "working";
    pub const VERIFYING: &str = "verifying";
    pub const APPROVED: &str = "approved";
    pub const COORDINATING: &str = "coordinating";
    pub const NEEDS_USER: &str = "needs_user";
    pub const FAILED: &str = "failed";
    pub const SKIPPED: &str = "skipped";
    pub const CANCELLED: &str = "cancelled";
    /// A plan the plan verifier sent back; the orchestrator proposes another.
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

/// The last ```<tag> fenced block: its body, and the text with it removed.
const OPTIONS_BINDING: &str = "Implement the selected_approach input for this task. It is a reviewed design decision, never permission to expand scope. Ask the orchestrator if a premise fails or a different approach is necessary; a changed selection requires fresh options and review. Never silently switch variants.";

const OPTIONS_VERIFICATION: &str = "Independently compare the implementation with selected_approach and this task's local requirements. Verify actual files and recorded checks. A changed approach requires reviewed options; the worker's declaration is not proof. Ask the orchestrator about missing or inconsistent input. Do not reassess other tasks or the whole session.";

/// A verifier's verdict. Without a readable verdict block the whole answer
/// counts as one finding, so the worker sees what the verifier said.
/// An approval that still lists high or medium findings contradicts itself
/// and counts as a rejection.
#[cfg(test)]
#[path = "tasks/tests.rs"]
mod tests;

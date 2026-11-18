// Typed task results. Prose is evidence, never a control protocol.
use crate::lean::{
    interview,
    tasks::{PlannedTask, Verdict},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum TurnResult {
    Report {
        text: String,
        /// The open comments the orchestrator says this answer settles.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        addressed: Option<Vec<u64>>,
    },
    /// A task graph from `submit_plan`, already checked in the turn.
    Plan {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scope: Option<String>,
        document: String,
        tasks: Vec<PlannedTask>,
        /// Approved tasks of the plan that ran, kept with their results.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        keep: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        addressed: Option<Vec<u64>>,
    },
    /// A question for the user from `ask_user`, its id given by the runtime.
    Question {
        question: interview::Question,
    },
    /// The interview's requirements summary from `submit_requirements`.
    Requirements {
        markdown: String,
        #[serde(default)]
        title: String,
    },
    Completed {
        summary: String,
        report: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        commit_message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        selection: Option<Selection>,
    },
    Review {
        report: String,
        verdict: Verdict,
    },
    Blocked {
        report: String,
        resume_condition: String,
    },
    NeedsInput {
        request: String,
        checkpoint: String,
    },
}

/// The approach an options task chose and what it was weighed against.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub alternatives: Vec<Alternative>,
    pub selected: String,
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Alternative {
    pub id: String,
    pub approach: String,
    pub tradeoffs: String,
}

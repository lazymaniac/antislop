// Typed task results. Prose is evidence, never a control protocol.
use crate::lean::{
    interview, ledger,
    tasks::{PlannedTask, Verdict},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
// Not `deny_unknown_fields`: stored results from before a field was dropped
// (a completed turn's `outputs`) must still load. Agents never write this
// type directly; each MCP tool has its own strict schema.
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TurnResult {
    Report {
        text: String,
        /// The open comments the orchestrator says this answer settles.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        addressed: Option<Vec<u64>>,
    },
    /// A task graph from `submit_plan`, already checked in the turn.
    Plan {
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
    },
    Completed {
        summary: String,
        report: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        selection: Option<Selection>,
    },
    Review {
        report: String,
        verdict: Verdict,
    },
    NeedsInput {
        request: String,
        checkpoint: String,
    },
    /// The requirements `submit_ledger` extracted, ranges and run filled in
    /// by the runtime.
    Ledger {
        extraction: ledger::Extraction,
    },
    /// The audit of `submit_audit`; it approves unless a finding blocks.
    Audit {
        findings: Vec<serde_json::Value>,
        evidence: Vec<ledger::AuditEvidence>,
    },
    /// The assessments of `submit_audit_review`, by the evidence ids the
    /// runtime gave the audit's proposals.
    AuditReview {
        note: String,
        assessments: Vec<ledger::AuditAssessment>,
    },
}

/// The approach an options task chose and what it was weighed against.
/// Stored selections may carry fields of the earlier, longer form; they are
/// ignored. The `select` tool's schema is strict.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Selection {
    pub alternatives: Vec<Alternative>,
    pub selected: String,
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Alternative {
    pub id: String,
    pub approach: String,
    pub tradeoffs: String,
}

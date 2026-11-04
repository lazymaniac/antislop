//! Versioned obligations. Sources and history are immutable; completion is derived
//! from independent evidence about the integrated result, never task approval.
mod constraints;
mod model;
pub(crate) use constraints::{validate_plan_runtime_constraints, validate_runtime_constraints};
mod runtime;
mod storage;
mod task_links;

pub use model::*;
pub(crate) use task_links::{resolve_covers, validate_criterion_coverage, validate_task_links};

pub use runtime::{AuditAssessment, AuditEvidence};

use crate::{Result, require};
use rusqlite::Connection;

/// The failure of a requirements analysis that ended without its ledger;
/// the workflow repairs it once per failure.
pub const MISSING_LEDGER: &str = "the requirements analyst ended its turn without submit_ledger";

/// Approvals bind semantic edits, immutable version identities and their impact;
/// source documents are never rehashed as part of an approval token.
pub fn diff_hash(diff: &LedgerDiff) -> Result<String> {
    diff_hash_with_after(diff, &serde_json::to_value(&diff.after)?)
}

pub(crate) fn matches_proposal_hash(
    diff: &LedgerDiff,
    request: &str,
    expected: &str,
) -> Result<bool> {
    if diff_hash(diff)? == expected {
        return Ok(true);
    }
    let raw: serde_json::Value = serde_json::from_str(request)?;
    Ok(diff_hash_with_after(diff, &raw["entries"])? == expected)
}

fn diff_hash_with_after(diff: &LedgerDiff, after: &serde_json::Value) -> Result<String> {
    Ok(crate::hash(serde_json::to_vec(
        &serde_json::json!({"run_id":diff.run_id,"from_revision":diff.from_revision,"to_revision":diff.to_revision,"before":diff.before.iter().map(|e|serde_json::json!({"id":e.spec.id,"version":e.version,"spec_revision":e.spec_revision})).collect::<Vec<_>>(),"after":after,"affected_tasks":diff.affected_tasks}),
    )?))
}

//! Versioned obligations. Sources and history are immutable; completion is derived
//! from independent evidence about the integrated result, never task approval.
mod model;
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
    Ok(crate::hash(serde_json::to_vec(
        &serde_json::json!({"run_id":diff.run_id,"from_revision":diff.from_revision,"to_revision":diff.to_revision,"before":diff.before.iter().map(|e|serde_json::json!({"id":e.spec.id,"version":e.version,"spec_revision":e.spec_revision})).collect::<Vec<_>>(),"after":diff.after,"affected_tasks":diff.affected_tasks}),
    )?))
}

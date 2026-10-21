//! Versioned obligations. Sources and history are immutable; completion is derived
//! from independent evidence about the integrated result, never task approval.
mod runtime;
mod storage;

use crate::{Result, require};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

pub(super) fn migrate(db: &Connection) -> Result<()> {
    let tx = db.unchecked_transaction()?;
    tx.execute_batch(include_str!("schema.sql"))?;
    let version: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version < 7 {
        tx.pragma_update(None, "user_version", 7)?;
    }
    tx.commit()?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    pub id: String,
    pub conversation_id: String,
    pub goal: String,
    pub spec_revision: i64,
    pub state: String,
    pub start_message: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerKind {
    Requirement,
    PlanItem,
}
impl LedgerKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Requirement => "requirement",
            Self::PlanItem => "plan_item",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    #[default]
    Active,
    SupersededByUser,
    ReplacedByUser,
    WithdrawnByUser,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceSpan {
    pub source_id: String,
    pub start: usize,
    pub end: usize,
}
impl SourceSpan {
    pub fn text<'a>(&self, source: &'a str) -> Result<&'a str> {
        let boundary =
            |at| at == source.len() || source.grapheme_indices(true).any(|(i, _)| i == at);
        require(
            self.start < self.end
                && self.end <= source.len()
                && boundary(self.start)
                && boundary(self.end),
            "source range must use grapheme boundaries",
        )?;
        Ok(&source[self.start..self.end])
    }
    pub fn lines(&self, source: &str) -> Result<(usize, usize)> {
        self.text(source)?;
        Ok((
            source[..self.start].bytes().filter(|b| *b == b'\n').count() + 1,
            source.as_bytes()[..self.end - 1]
                .iter()
                .filter(|b| **b == b'\n')
                .count()
                + 1,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub run_id: String,
    pub message: Option<u64>,
    pub locator: Option<String>,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Criterion {
    pub id: String,
    pub text: String,
    #[serde(default = "yes")]
    pub mandatory: bool,
    #[serde(default)]
    pub integration: bool,
}
fn yes() -> bool {
    true
}

/// One small record; collections are stored as normalized rows, not run snapshots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntrySpec {
    pub id: String,
    pub kind: LedgerKind,
    pub source: SourceSpan,
    pub interpretation: String,
    #[serde(default)]
    pub qualifiers: Vec<String>,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default = "yes")]
    pub mandatory: bool,
    #[serde(default)]
    pub implementation_owners: Vec<String>,
    #[serde(default)]
    pub verification_owners: Vec<String>,
    #[serde(default)]
    pub criteria: Vec<Criterion>,
    #[serde(default)]
    pub requirement_ids: Vec<String>,
    #[serde(default)]
    pub task_ids: Vec<String>,
    #[serde(default)]
    pub position: u32,
    #[serde(default)]
    pub technical_goal: Option<String>,
    #[serde(default)]
    pub check: Option<String>,
    #[serde(default)]
    pub disposition: Disposition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub run_id: String,
    pub version: i64,
    pub spec_revision: i64,
    pub original: String,
    pub spec: EntrySpec,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LedgerChange {
    pub conversation: String,
    pub run_id: String,
    pub expected_spec_revision: i64,
    pub operation_id: String,
    pub reason: String,
    pub entries: Vec<EntrySpec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerDiff {
    pub run_id: String,
    pub from_revision: i64,
    pub to_revision: i64,
    pub before: Vec<LedgerEntry>,
    pub after: Vec<EntrySpec>,
    pub affected_tasks: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeResult {
    pub run_id: String,
    pub spec_revision: i64,
    pub affected_tasks: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LedgerQuery {
    pub conversation: String,
    pub run_id: String,
    #[serde(default)]
    pub revision: Option<i64>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub task: Option<String>,
    #[serde(default)]
    pub missing_coverage: bool,
    #[serde(default)]
    pub offset: usize,
    #[serde(default)]
    pub limit: Option<usize>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerPage {
    pub run: Run,
    pub entries: Vec<LedgerEntry>,
    pub total: usize,
    pub offset: usize,
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceProposal {
    pub entry_id: String,
    pub entry_version: i64,
    pub criterion_id: String,
    pub task_id: String,
    pub generation: i64,
    pub attempt: u32,
    pub producer_turn: String,
    pub result: String,
    pub artifact: Option<String>,
    pub integrated_fingerprint: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub id: String,
    pub run_id: String,
    pub proposal: EvidenceProposal,
    pub verifier_turn: String,
    pub assessment: String,
    pub approved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assessment {
    pub evidence: EvidenceProposal,
    pub approved: bool,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Extraction {
    pub run_id: String,
    pub expected_spec_revision: i64,
    pub entries: Vec<EntrySpec>,
}

/// Read-only agent output cannot name its own author/session.
pub fn parse_block<T: serde::de::DeserializeOwned>(answer: &str, tag: &str) -> Result<T> {
    let open = format!("```{tag}\n");
    let body = answer
        .rsplit_once(&open)
        .and_then(|(_, tail)| tail.split_once("\n```").map(|(body, _)| body));
    Ok(serde_json::from_str(body.ok_or_else(|| {
        crate::Error::Invalid(format!("missing {tag} JSON block"))
    })?)?)
}

/// Approvals bind semantic edits, immutable version identities and their impact;
/// source documents are never rehashed as part of an approval token.
pub fn diff_hash(diff: &LedgerDiff) -> Result<String> {
    Ok(crate::hash(serde_json::to_vec(
        &serde_json::json!({"run_id":diff.run_id,"from_revision":diff.from_revision,"to_revision":diff.to_revision,"before":diff.before.iter().map(|e|serde_json::json!({"id":e.spec.id,"version":e.version,"spec_revision":e.spec_revision})).collect::<Vec<_>>(),"after":diff.after,"affected_tasks":diff.affected_tasks}),
    )?))
}

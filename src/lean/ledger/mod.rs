//! Versioned obligations. Sources and history are immutable; completion is derived
//! from independent evidence about the integrated result, never task approval.
mod runtime;
mod storage;

pub use runtime::{AuditAssessment, AuditEvidence};

use crate::{Result, require};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    pub id: String,
    pub conversation_id: String,
    pub goal: String,
    pub spec_revision: i64,
    pub state: String,
    pub start_message: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    #[default]
    Active,
    SupersededByUser,
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
    pub task_ids: Vec<String>,
    #[serde(default)]
    pub position: u32,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Extraction {
    pub run_id: String,
    pub expected_spec_revision: i64,
    pub entries: Vec<EntrySpec>,
}

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

/// Fills `requirement_ids` and `criterion_ids` from what the planner named in
/// `covers`: a criterion brings its requirement along, a requirement stands
/// for itself. Links already present (a stored plan) are kept when `covers`
/// is empty.
pub(crate) fn resolve_covers(
    spec: &mut crate::lean::tasks::TaskContract,
    entries: &[LedgerEntry],
) -> Result<()> {
    if spec.covers.is_empty() {
        if spec.requirement_ids.is_empty() {
            return Err(crate::Error::Invalid(format!(
                "task {}: `covers` is empty; name the acceptance criteria (or requirements) it delivers",
                spec.id
            )));
        }
        return Ok(());
    }
    let active: Vec<_> = entries
        .iter()
        .filter(|e| e.spec.disposition == Disposition::Active)
        .collect();
    let (mut requirements, mut criteria) = (Vec::new(), Vec::new());
    for id in &spec.covers {
        if let Some(entry) = active.iter().find(|e| e.spec.id == *id) {
            push_unique(&mut requirements, &entry.spec.id);
        } else if let Some(entry) = active
            .iter()
            .find(|e| e.spec.criteria.iter().any(|c| c.id == *id))
        {
            push_unique(&mut requirements, &entry.spec.id);
            push_unique(&mut criteria, id);
        } else {
            return Err(crate::Error::Invalid(format!(
                "task {}: covers names {id}, which is neither an active requirement nor one of its criteria",
                spec.id
            )));
        }
    }
    spec.requirement_ids = requirements;
    spec.criterion_ids = criteria;
    Ok(())
}

fn push_unique(ids: &mut Vec<String>, id: &str) {
    if !ids.iter().any(|known| known == id) {
        ids.push(id.to_owned());
    }
}

/// Shared by initial plans and edits of the running task graph.
pub(crate) fn validate_task_links(
    spec: &crate::lean::tasks::TaskContract,
    entries: &[LedgerEntry],
) -> Result<()> {
    require(
        !spec.requirement_ids.is_empty(),
        "task needs at least one active requirement",
    )?;
    for id in &spec.requirement_ids {
        require(
            entries
                .iter()
                .any(|e| e.spec.id == *id && e.spec.disposition == Disposition::Active),
            "unknown or inactive task requirement",
        )?;
    }
    for id in &spec.criterion_ids {
        require(
            entries.iter().any(|e| {
                spec.requirement_ids.contains(&e.spec.id)
                    && e.spec.criteria.iter().any(|c| c.id == *id)
            }),
            "task criterion is not owned by a linked requirement",
        )?;
    }
    Ok(())
}

pub(crate) fn validate_criterion_coverage(
    entries: &[LedgerEntry],
    covered: &std::collections::HashSet<String>,
) -> Result<()> {
    for e in entries
        .iter()
        .filter(|e| e.spec.disposition == Disposition::Active && e.spec.mandatory)
    {
        require(
            e.spec.criteria.iter().any(|c| c.mandatory),
            &format!(
                "mandatory requirement {} needs clarification and acceptance criteria",
                e.spec.id
            ),
        )?;
        for c in e.spec.criteria.iter().filter(|c| c.mandatory) {
            require(
                covered.contains(&c.id),
                &format!("mandatory criterion {} has no task", c.id),
            )?;
        }
    }
    Ok(())
}

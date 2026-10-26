//! Whether a worker is getting closer to approval. A verifier words its
//! findings differently every round, so their text says little; the
//! obligations it leaves open (`V5`, `R2`…) keep their ids. A worker
//! that leaves mostly the same ones open round after round, without ever
//! getting below its best so far, is not getting closer.
use std::collections::BTreeSet;

use serde_json::Value;

use crate::lean::tasks::{Verdict, counted_status};

/// The obligations a verdict leaves open: judged an issue, blocked, or
/// answered without a note.
pub fn open_obligations(verdict: &Verdict) -> BTreeSet<String> {
    verdict
        .coverage
        .iter()
        .filter(|(_, entry)| is_open(entry))
        .map(|(id, _)| id.clone())
        .chain(
            verdict
                .findings
                .iter()
                .filter(|f| matches!(f["severity"].as_str(), Some("critical" | "high" | "medium")))
                .filter_map(|f| f["id"].as_str().map(|id| format!("finding:{id}"))),
        )
        .collect()
}

fn is_open(entry: &Value) -> bool {
    matches!(counted_status(entry), "issue" | "blocked" | "missing")
}

#[derive(Debug, Default)]
pub struct Progress {
    previous: Option<BTreeSet<String>>,
}
impl Progress {
    /// The first rejection establishes a baseline; subsequent rejection must close an earlier obligation.
    pub fn record(&mut self, open: BTreeSet<String>) -> bool {
        let stalled = self
            .previous
            .as_ref()
            .is_some_and(|before| !before.is_empty() && before.difference(&open).next().is_none());
        self.previous = Some(open);
        stalled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn open(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    fn rounds(sets: &[&[&str]]) -> Vec<bool> {
        let mut progress = Progress::default();
        sets.iter().map(|ids| progress.record(open(ids))).collect()
    }

    #[test]
    fn the_same_obligations_round_after_round_stall_the_worker() {
        let same = ["V5", "R2", "C3", "V1"];
        let stalled = rounds(&[&same, &same, &same, &same, &same]);
        assert_eq!(stalled, [false, true, true, true, true]);
    }

    #[test]
    fn reworded_findings_do_not_matter_only_the_ids_do() {
        let stalled = rounds(&[
            &["V5", "R2", "C3", "V1", "C7"],
            &["V5", "R2", "C3", "V1", "R238"],
            &["V5", "R2", "C3", "V1", "C13"],
            &["V5", "R2", "C3", "V1", "V4"],
        ]);
        assert_eq!(stalled, [false, false, false, false]);
    }

    #[test]
    fn a_new_best_is_progress_even_when_the_rest_is_unchanged() {
        let stalled = rounds(&[
            &["a", "b", "c", "d"],
            &["a", "b", "c", "d"],
            &["a", "b", "c"],
            &["a", "b", "c"],
            &["a", "b"],
            &["a", "b"],
            &["a", "b"],
        ]);
        assert_eq!(stalled, [false, true, false, true, false, true, true]);
    }

    #[test]
    fn different_problems_each_round_are_not_a_stall() {
        let stalled = rounds(&[
            &["a", "b"],
            &["c", "d"],
            &["e", "f"],
            &["g", "h"],
            &["i", "j"],
        ]);
        assert!(stalled.iter().all(|stalled| !stalled));
    }

    #[test]
    fn a_verdict_without_coverage_says_nothing() {
        let mut progress = Progress::default();
        assert!(!(0..10).any(|_| progress.record(BTreeSet::new())));
    }

    #[test]
    fn open_means_issue_blocked_or_unexplained() {
        let verdict = Verdict {
            approved: false,
            findings: Vec::new(),
            coverage: json!({
                "R1": {"status": "ok", "note": "fine"},
                "R2": {"status": "issue", "note": "wrong"},
                "R3": {"status": "blocked", "note": "no data"},
                "R4": {"status": "n/a", "note": "not here"},
                "R5": {"status": "ok"}
            })
            .as_object()
            .unwrap()
            .clone(),
        };
        assert_eq!(open_obligations(&verdict), open(&["R2", "R3", "R5"]));
    }
}

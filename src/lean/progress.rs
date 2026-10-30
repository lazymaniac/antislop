//! Whether a worker is getting closer to approval. A verifier words its
//! findings differently every round, so their text says little; the
//! obligations it leaves open (`V5`, `R2`…) keep their ids. A worker
//! that leaves mostly the same ones open round after round, without ever
//! getting below its best so far, is not getting closer. Broad checks such as
//! `goal` stay open while the verifier finds one new defect after another, so
//! the verifier also confirms which of its earlier findings the round fixed;
//! a fixed finding is progress even when the same checks remain open.
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Digest;

use crate::lean::tasks::{Verdict, counted_status};

/// The obligations a verdict leaves open: judged an issue, blocked, or
/// answered without a note. A finding against a check already open counts
/// as that check, since its own id follows its wording and changes every
/// round; any other finding counts by its id.
pub fn open_obligations(verdict: &Verdict) -> BTreeSet<String> {
    let checks: BTreeSet<String> = verdict
        .coverage
        .iter()
        .filter(|(_, entry)| is_open(entry))
        .map(|(id, _)| id.clone())
        .collect();
    let findings: Vec<String> = verdict
        .findings
        .iter()
        .filter(|f| blocks(f))
        .filter(|f| {
            f["check"]
                .as_str()
                .is_none_or(|check| !checks.contains(check))
        })
        .filter_map(|f| f["id"].as_str().map(|id| format!("finding:{id}")))
        .collect();
    checks.into_iter().chain(findings).collect()
}

fn is_open(entry: &Value) -> bool {
    matches!(counted_status(entry), "issue" | "blocked" | "missing")
}

/// Whether a finding sends the task back.
pub fn blocks(finding: &Value) -> bool {
    matches!(
        finding["severity"].as_str(),
        Some("critical" | "high" | "medium")
    )
}

/// Whether the verdict confirms that a blocking finding of the previous round
/// is fixed. Ids it names that the previous round never raised do not count.
pub fn fixed_earlier(earlier: &[Value], verdict: &Verdict) -> bool {
    earlier
        .iter()
        .filter(|f| blocks(f))
        .filter_map(|f| f["id"].as_str())
        .any(|id| verdict.resolved.iter().any(|fixed| fixed == id))
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Progress {
    previous: Option<BTreeSet<String>>,
}

/// A task's rounds since the orchestrator last gave it a basis, kept across a
/// restart that cuts them off. A new contract or new inputs start a new run.
#[derive(Debug, Serialize, Deserialize)]
pub struct Run {
    basis: String,
    /// The round the run began with.
    pub since: u32,
    pub progress: Progress,
    /// The previous round's findings.
    pub findings: Vec<Value>,
}

impl Run {
    /// The run a stored record continues, or a new one from `round`.
    pub fn resume(record: Option<Value>, spec: &Value, generation: i64, round: u32) -> Self {
        let basis = basis(spec, generation);
        record
            .and_then(|record| serde_json::from_value::<Run>(record).ok())
            .filter(|run| run.basis == basis)
            .unwrap_or(Run {
                basis,
                since: round,
                progress: Progress::default(),
                findings: Vec::new(),
            })
    }
}

fn basis(spec: &Value, generation: i64) -> String {
    let digest = sha2::Sha256::digest(format!("{generation}:{spec}"));
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
impl Progress {
    /// The first rejection establishes a baseline; subsequent rejection must close an earlier obligation.
    /// A rejection that names nothing open, twice in a row, is a stall too: the
    /// worker has nothing to correct.
    pub fn record(&mut self, open: BTreeSet<String>) -> bool {
        let stalled = self.previous.as_ref().is_some_and(|before| {
            before.difference(&open).next().is_none() && (!before.is_empty() || open.is_empty())
        });
        self.previous = Some(open);
        stalled
    }

    /// A task round: stalled only when its open obligations show no progress
    /// and the verifier confirms none of its earlier findings fixed.
    pub fn record_round(&mut self, open: BTreeSet<String>, fixed_earlier: bool) -> bool {
        self.record(open) && !fixed_earlier
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
    fn rejections_naming_nothing_open_stall_after_the_first() {
        let stalled = rounds(&[&[], &[], &[]]);
        assert_eq!(stalled, [false, true, true]);
    }

    #[test]
    fn a_first_named_obligation_after_an_empty_rejection_is_new_information() {
        assert_eq!(rounds(&[&[], &["a"], &["a"]]), [false, false, true]);
    }

    #[test]
    fn reworded_findings_against_the_same_checks_stall_the_worker() {
        let verdict = |suffix: &str| Verdict {
            resolved: Vec::new(),
            approved: false,
            findings: vec![
                json!({"check": "R5.1", "id": format!("R5.1:{suffix}"), "severity": "medium"}),
                json!({"check": "goal", "id": format!("goal:{suffix}"), "severity": "medium"}),
            ],
            coverage: json!({
                "R5.1": {"status": "issue", "note": "records absent"},
                "goal": {"status": "issue", "note": "evidence missing"}
            })
            .as_object()
            .unwrap()
            .clone(),
        };
        let mut progress = Progress::default();
        assert!(!progress.record(open_obligations(&verdict("f0cab199"))));
        assert!(progress.record(open_obligations(&verdict("a2c82963"))));
    }

    #[test]
    fn open_means_issue_blocked_or_unexplained() {
        let verdict = Verdict {
            resolved: Vec::new(),
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

    fn rejected(findings: &[&str], resolved: &[&str]) -> Verdict {
        Verdict {
            approved: false,
            findings: findings
                .iter()
                .map(|id| json!({"check": "goal", "id": id, "severity": "medium"}))
                .collect(),
            coverage: json!({"goal": {"status": "issue", "note": "incomplete"}})
                .as_object()
                .unwrap()
                .clone(),
            resolved: resolved.iter().map(|id| (*id).to_owned()).collect(),
        }
    }

    #[test]
    fn a_fixed_finding_is_progress_while_a_new_one_keeps_the_same_check_open() {
        let mut progress = Progress::default();
        let first = rejected(&["goal:aaaa"], &[]);
        assert!(!progress.record_round(open_obligations(&first), false));
        let second = rejected(&["goal:bbbb"], &["goal:aaaa"]);
        let fixed = fixed_earlier(&first.findings, &second);
        assert!(fixed);
        assert!(!progress.record_round(open_obligations(&second), fixed));
        let third = rejected(&["goal:bbbb"], &[]);
        let fixed = fixed_earlier(&second.findings, &third);
        assert!(progress.record_round(open_obligations(&third), fixed));
    }

    #[test]
    fn only_blocking_findings_of_the_previous_round_count_as_fixed() {
        let mut earlier = rejected(&["goal:aaaa"], &[]).findings;
        earlier.push(json!({"check": "goal", "id": "goal:low", "severity": "low"}));
        assert!(!fixed_earlier(
            &earlier,
            &rejected(&[], &["goal:elsewhere"])
        ));
        assert!(!fixed_earlier(&earlier, &rejected(&[], &["goal:low"])));
        assert!(fixed_earlier(&earlier, &rejected(&[], &["goal:aaaa"])));
    }

    #[test]
    fn a_run_continues_on_its_basis_and_starts_over_on_a_new_one() {
        let spec = json!({"id": "a", "goal": "g"});
        let mut run = Run::resume(None, &spec, 1, 3);
        assert_eq!(run.since, 3);
        assert!(!run.progress.record(open(&["goal"])));
        run.findings = vec![json!({"id": "goal:aaaa", "severity": "medium"})];
        let record = serde_json::to_value(&run).unwrap();

        let mut resumed = Run::resume(Some(record.clone()), &spec, 1, 5);
        assert_eq!(resumed.since, 3);
        assert_eq!(resumed.findings.len(), 1);
        assert!(
            resumed.progress.record(open(&["goal"])),
            "the baseline survived"
        );

        let revised = json!({"id": "a", "goal": "narrower"});
        assert_eq!(Run::resume(Some(record.clone()), &revised, 1, 5).since, 5);
        assert_eq!(Run::resume(Some(record), &spec, 2, 5).since, 5);
        assert_eq!(Run::resume(Some(json!("garbage")), &spec, 1, 5).since, 5);
    }
}

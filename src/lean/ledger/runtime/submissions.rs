//! Validated ledger submission adapters.
use super::*;

#[path = "submissions/audit.rs"]
mod audit;
#[path = "submissions/extraction.rs"]
mod extraction;

#[cfg(test)]
pub(crate) use audit::plain_findings;
pub use audit::{AuditAssessment, AuditEvidence};
pub(crate) use audit::{audit_approves, audited_tasks, evidence_id, resolve_evidence};
#[cfg(test)]
use extraction::quoted_span;
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quote_names_one_place_in_the_sources() {
        let sources = vec![
            ("s1".to_owned(), "Add a 👩‍💻 badge. Keep the API.".to_owned()),
            ("s2".to_owned(), "Keep the API stable.".to_owned()),
        ];
        let span = quoted_span(&sources, "r1", " Add a 👩‍💻 badge.", None).unwrap();
        assert_eq!((span.source_id.as_str(), span.start), ("s1", 0));
        assert_eq!(span.text(&sources[0].1).unwrap(), "Add a 👩‍💻 badge.");
        // A later source repeating the words does not make them ambiguous.
        let first = quoted_span(&sources, "r2", "Keep the API", None).unwrap();
        assert_eq!((first.source_id.as_str(), first.start), ("s1", 25));
        let named = quoted_span(&sources, "r2", "Keep the API", Some("s2")).unwrap();
        assert_eq!((named.source_id.as_str(), named.start), ("s2", 0));
        let twice = quoted_span(&sources, "r3", "e", None).unwrap_err();
        assert!(twice.to_string().contains("times in source s1"), "{twice}");
        let missing = quoted_span(&sources, "r4", "Remove the API", None).unwrap_err();
        assert!(missing.to_string().contains("does not occur"), "{missing}");
        assert!(quoted_span(&sources, "r5", "Keep", Some("s9")).is_err());
        assert!(quoted_span(&sources, "r6", " ", None).is_err());
    }

    #[test]
    fn overlapping_quotes_are_ambiguous() {
        for (text, quote) in [("aaaa", "aaa"), ("ążążąż", "ążąż")] {
            let sources = vec![("s1".to_owned(), text.to_owned())];
            let error = quoted_span(&sources, "r1", quote, None).unwrap_err();
            assert!(
                error.to_string().contains("2 times in source s1"),
                "{error}"
            );
        }
    }

    #[test]
    fn an_audit_approves_unless_a_finding_blocks() {
        let low = plain_findings(&serde_json::json!([
            {"severity": "low", "issue": "naming", "where": "README"}
        ]))
        .unwrap();
        assert!(audit_approves(&low));
        let high = plain_findings(&serde_json::json!([
            {"severity": "high", "issue": "missing", "where": "R1"}
        ]))
        .unwrap();
        assert!(!audit_approves(&high));
        assert!(plain_findings(&serde_json::json!([{"issue": "x"}])).is_err());
    }

    #[test]
    fn evidence_names_the_task_of_the_plan_that_ran() {
        let entry: LedgerEntry = serde_json::from_value(serde_json::json!({
            "run_id": "run", "version": 1, "spec_revision": 1, "original": "Add sub",
            "status": "active",
            "spec": {"id": "R1", "interpretation": "Add sub", "mandatory": true,
                "position": 0, "disposition": "active", "depends_on": [], "qualifiers": [],
                "parent": null, "task_ids": [], "implementation_owners": [],
                "verification_owners": [],
                "source": {"source_id": "s1", "start": 0, "end": 7},
                "criteria": [{"id": "R1.1", "text": "sub subtracts", "mandatory": true,
                    "integration": false}]}
        }))
        .unwrap();
        let task = |id: &str, status: &str, round: u32| -> crate::lean::store::Task {
            serde_json::from_value(serde_json::json!({
                "id": id, "plan_id": "p", "spec": {"id": "build"}, "status": status,
                "round": round, "report": null, "verdict": null, "base": null, "result": null
            }))
            .unwrap()
        };
        // The plan verifier sent the first plan back; its task never ran.
        let tasks = [task("old", "revised", 0), task("new", "approved", 1)];
        let item = AuditEvidence {
            criterion_id: "R1.1".into(),
            task_id: "build".into(),
            result: "ran the tests".into(),
            artifact: None,
        };
        let proposal = resolve_evidence(&item, &[entry], &tasks).unwrap();
        assert_eq!((proposal.task_id.as_str(), proposal.attempt), ("new", 1));
        assert_eq!(audited_tasks(&tasks).count(), 1);
    }
}

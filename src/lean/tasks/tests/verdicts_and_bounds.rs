use super::*;

#[test]
fn a_verdict_counts_only_with_every_obligation_accounted_for() {
    let task: TaskContract = serde_json::from_value(serde_json::json!({
        "id": "t1", "title": "Revenue", "goal": "g", "kind": "analysis",
        "verifier": {"focus": "f", "risks": ["units"]}
    }))
    .unwrap();
    let obligations = obligations(&task);
    assert_eq!(obligations[0], ("R1".to_owned(), "units".to_owned()));
    assert_eq!(obligations[1].0, "V1");
    assert_eq!(
        obligations.len(),
        1 + COMMON_CHECKS.split("; ").count()
            + checklist(task.kind).split("; ").count()
            + task.verifier.risk_ids.len()
    );

    let mut skipped = parse_verdict("```verdict\n{\"approved\": true}\n```");
    assert_eq!(unaccounted(&skipped, &obligations).len(), obligations.len());
    settle(&mut skipped, &obligations, false);
    assert!(!skipped.approved, "a skipped obligation never approves");

    let mut coverage = serde_json::Map::new();
    for (id, _) in &obligations {
        coverage.insert(
            id.clone(),
            serde_json::json!({"status": "ok", "note": "checked"}),
        );
    }
    coverage.insert(
        "C1".into(),
        serde_json::json!({"status": "n/a", "note": " "}),
    );
    let mut verdict = Verdict {
        approved: true,
        findings: vec![],
        coverage,
    };
    let missing: Vec<&str> = unaccounted(&verdict, &obligations)
        .iter()
        .map(|(id, _)| id.as_str())
        .collect();
    assert_eq!(missing, ["C1"], "n/a needs a reason");

    verdict.coverage["C1"] = serde_json::json!({"status": "N/A", "note": "no data"});
    verdict.coverage["R1"] = serde_json::json!({"status": "issue", "note": "EUR + PLN"});
    verdict.coverage["V2"] = serde_json::json!({"status": "blocked", "note": "no data to rerun"});
    settle(&mut verdict, &obligations, false);
    assert!(!verdict.approved);
    assert_eq!(
        findings_text(&verdict.findings),
        format!(
            "- [medium] units: EUR + PLN (R1)\n- [medium] could not be settled: {}: no data to rerun (V2)",
            obligations[2].1
        )
    );
}

#[test]
fn a_read_only_report_is_not_rejected_for_defects_it_rightly_reports() {
    let task: TaskContract = serde_json::from_value(serde_json::json!({
        "id": "t1", "title": "Audit", "goal": "g", "kind": "analysis",
        "verifier": {"focus": "f", "risks": ["stale consent"]}
    }))
    .unwrap();
    let obligations = obligations(&task);
    let coverage = |c: &str| {
        let mut map = serde_json::Map::new();
        for (id, _) in &obligations {
            map.insert(
                id.clone(),
                serde_json::json!({"status": "ok", "note": "checked"}),
            );
        }
        map.insert(
            "R1".into(),
            serde_json::json!({"status": c, "note": "open in code"}),
        );
        map.insert(
            "V1".into(),
            serde_json::json!({"status": c, "note": "report gap"}),
        );
        map
    };
    let mut verdict = Verdict {
        approved: true,
        findings: vec![],
        coverage: coverage("issue"),
    };
    settle(&mut verdict, &obligations, true);
    assert!(
        !verdict.approved,
        "an issue on a common check still rejects"
    );
    assert_eq!(verdict.findings.len(), 2);
    assert_eq!(verdict.findings[0]["where"], "R1");

    let mut only_risk = Verdict {
        approved: true,
        findings: vec![],
        coverage: coverage("ok"),
    };
    only_risk.coverage["R1"] = serde_json::json!({"status": "issue", "note": "open in code"});
    settle(&mut only_risk, &obligations, true);
    assert!(!only_risk.approved && !only_risk.findings.is_empty());
    let mut writer = Verdict {
        approved: true,
        findings: vec![],
        coverage: only_risk.coverage.clone(),
    };
    settle(&mut writer, &obligations, false);
    assert!(
        !writer.approved,
        "a writing task still needs its risks settled"
    );
}

#[test]
fn repeated_findings_are_recognised_despite_reworded_notes() {
    let round = |note: &str| {
        vec![serde_json::json!({
            "severity": "high",
            "issue": format!("Consent subset is approved: {note}"),
            "where": "mod.rs:1110"
        })]
    };
    assert_eq!(
        findings_key(&round("seen in v8")),
        findings_key(&round("still open in v9, checked again"))
    );
    assert_ne!(
        findings_key(&round("x")),
        findings_key(&[serde_json::json!({"severity": "low", "issue": "Other", "where": "a"})])
    );
}

#[test]
fn verdicts_parse_or_fall_back_to_the_whole_answer() {
    let verdict =
        parse_verdict("Looks fine.\n```verdict\n{\"approved\": true, \"findings\": []}\n```");
    assert!(verdict.approved);
    let contradictory = parse_verdict(
        "```verdict\n{\"approved\": true, \"findings\": [{\"severity\": \"High\", \"issue\": \"x\"}]}\n```",
    );
    assert!(!contradictory.approved);
    let fallback = parse_verdict("I could not decide.");
    assert!(!fallback.approved);
    assert_eq!(findings_text(&fallback.findings), "- I could not decide.");
}

#[test]
fn a_verdict_fenced_as_json_still_counts_but_other_json_does_not() {
    let fenced = parse_verdict("Approved.\n```json\n{\"approved\": true, \"findings\": []}\n```");
    assert!(fenced.approved);
    let unrelated = parse_verdict("Sample:\n```json\n{\"name\": \"x\"}\n```");
    assert!(!unrelated.approved);
    // Bare JSON, as the whole answer or its last line, is a verdict too.
    let whole = parse_verdict("{\"approved\":true,\"findings\":[],\"review\":\"fine\"}");
    assert!(whole.approved);
    let last =
        parse_verdict("Approved after reading the files.\n\n{\"approved\":true,\"findings\":[]}\n");
    assert!(last.approved);
    assert!(!parse_verdict("Looks {\"approved\":true} to me, but unsure.").approved);
}

#[test]
fn bound_cuts_on_graphemes_within_a_byte_budget() {
    assert_eq!(bound("short", 10), "short");
    assert_eq!(bound("abcd", 4), "abcd");
    // The budget is bytes, counted before the mark is appended: at 3
    // there is no room for a whole cluster beside the mark, so the
    // first whole cluster stands for the cut instead of splitting one.
    assert_eq!(bound("abcd", 3), "a");
    // A cut never splits a multi-byte character.
    let text = "abąćęł cńódź efżó";
    let cut = bound(text, 5);
    assert_eq!(cut, "ab…");
    assert!(cut.len() <= 5 && cut.is_char_boundary(cut.len()));
}

#[test]
fn bound_answers_zero_one_and_oversized_clusters_whole() {
    // limit 0 is empty: never a mark paid over the budget.
    assert_eq!(bound("abcd", 0), "");
    // limit 1 keeps the first whole cluster instead of all content lost.
    assert_eq!(bound("abcd", 1), "a");
    assert_eq!(bound("abcd", 2), "a");
    // A combining sequence is one cluster: kept whole, never split.
    let composed = "e\u{301}xtra";
    assert_eq!(bound(composed, 3), "e\u{301}");
    assert_eq!(bound(composed, 2), "");
    assert_eq!(bound(composed, 6), "e\u{301}…");
    // A ZWJ family is one oversized cluster: where it fits no more than
    // the limit holds, the mark alone says the cut happened.
    let family = "👨‍👩‍👧‍👦🚀rest";
    assert!(
        family.graphemes(true).next().unwrap().len() > 24,
        "the family is one oversized cluster"
    );
    assert_eq!(bound(family, 3), CUT_MARK.to_string());
    assert_eq!(bound(family, 2), "");
}

#[test]
fn bound_never_splits_a_cluster_and_always_fits_the_budget() {
    let text = "aé–—👨‍👩‍👧‍👦🚀ż\u{301}end";
    let clusters: Vec<&str> = text.graphemes(true).collect();
    for limit in 0..=text.len() + 2 {
        let (cut, was_cut) = bound_cut(text, limit);
        assert_eq!(was_cut, text.len() > limit, "limit {limit}: {cut:?}");
        assert!(cut.len() <= limit, "limit {limit}: {cut:?}");
        let body = cut.strip_suffix(CUT_MARK).unwrap_or(&cut);
        let shown: Vec<&str> = body.graphemes(true).collect();
        assert!(
            clusters.starts_with(&shown),
            "limit {limit}: {cut:?} is not whole leading clusters of the text"
        );
    }
}

#[test]
fn verdict_json_label_renders_findings_as_text() {
    let verdict = parse_verdict(
        "```verdict JSON\n{\"approved\":false,\"findings\":[{\"issue\":\"Preserve Ctrl+O.\"}]}\n```",
    );
    assert!(!verdict.approved);
    assert_eq!(findings_text(&verdict.findings), "- Preserve Ctrl+O.");
}

#[test]
fn only_structured_medium_and_low_findings_are_minor() {
    let rejected = |findings: serde_json::Value| {
        parse_verdict(&format!(
            "```verdict\n{}\n```",
            json!({"approved": false, "findings": findings})
        ))
    };
    assert!(only_minor_findings(&rejected(
        json!([{"severity": "medium", "issue": "x"}, {"severity": "low", "issue": "y"}])
    )));
    assert!(!only_minor_findings(&rejected(
        json!([{"severity": "medium", "issue": "x"}, {"severity": "high", "issue": "y"}])
    )));
    assert!(!only_minor_findings(&rejected(json!([]))));
    assert!(!only_minor_findings(&parse_verdict(
        "REJECT without a block"
    )));
}

#[test]
fn bare_review_verdicts_preserve_findings_and_reject_blockers() {
    for severity in ["low", "medium", "high", "critical"] {
        let finding = serde_json::json!({"severity": severity, "issue": "Check source"});
        let raw =
            serde_json::json!({"approved": true, "findings": [finding], "review": "Reviewed"});
        for answer in [
            raw.to_string(),
            format!(
                "Review complete.\n{}",
                serde_json::to_string_pretty(&raw).unwrap()
            ),
        ] {
            let verdict = parse_verdict(&answer);
            assert_eq!(verdict.approved, severity == "low", "{answer}");
            assert_eq!(verdict.findings, vec![finding.clone()]);
        }
    }
    assert!(parse_verdict("Reviewed.\n{\n  \"approved\": true,\n  \"findings\": []\n}\n").approved);
    for answer in [
        "Reviewed.\n{\"name\":\"example\"}",
        "{\"approved\":true} trailing text",
        "{\"approved\":true}\n{\"approved\":false,\"findings\":[]}",
        "{\"approved\":\"true\"}",
    ] {
        assert!(!parse_verdict(answer).approved, "{answer}");
    }
}

#[test]
fn prose_alone_is_not_a_structured_verdict() {
    assert!(structured_verdict("Plan review: approved. Everything is consistent.").is_none());
    assert!(structured_verdict("```verdict\n{\"approved\":true,\"findings\":[]}\n```").is_some());
    assert!(!parse_verdict("Plan review: approved.").approved);
}

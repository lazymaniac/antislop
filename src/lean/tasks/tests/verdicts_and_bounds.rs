use super::*;

#[test]
fn a_verifier_owes_the_goal_the_assigned_criteria_and_the_kind_catalog_only() {
    let mut task: TaskContract = serde_json::from_value(serde_json::json!({
        "id": "t1", "title": "Revenue", "goal": "g", "kind": "analysis",
        "verifier": {"focus": "f", "risks": ["units"], "risk_ids": ["R001"]}
    }))
    .unwrap();
    task.criterion_ids = vec!["REQ-1.1".into()];
    let ids: Vec<String> = obligations(&task).into_iter().map(|(id, _)| id).collect();
    let catalog: Vec<String> = crate::lean::risks::for_kind(task.kind)
        .map(|(id, _)| id)
        .collect();
    assert!(!catalog.is_empty());
    let mut expected = vec![GOAL_CHECK.to_owned(), "REQ-1.1".to_owned()];
    expected.extend(catalog);
    assert_eq!(
        ids, expected,
        "no planner risks, common checks or checklist"
    );
}

#[test]
fn a_verdict_counts_only_with_every_obligation_accounted_for() {
    let task: TaskContract = serde_json::from_value(serde_json::json!({
        "id": "t1", "title": "Revenue", "goal": "g", "kind": "analysis"
    }))
    .unwrap();
    let obligations = obligations(&task);
    assert_eq!(obligations[0].0, GOAL_CHECK);
    let (first, second) = (&obligations[1], &obligations[2]);

    let mut skipped = verdict(true, json!([]));
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
        GOAL_CHECK.into(),
        serde_json::json!({"status": "n/a", "note": " "}),
    );
    let mut verdict = Verdict {
        resolved: Vec::new(),
        approved: true,
        findings: vec![],
        coverage,
    };
    let missing: Vec<&str> = unaccounted(&verdict, &obligations)
        .iter()
        .map(|(id, _)| id.as_str())
        .collect();
    assert_eq!(missing, [GOAL_CHECK], "n/a needs a reason");

    verdict.coverage[GOAL_CHECK] = serde_json::json!({"status": "N/A", "note": "no data"});
    verdict.coverage[&first.0] = serde_json::json!({"status": "issue", "note": "EUR + PLN"});
    verdict.coverage[&second.0] =
        serde_json::json!({"status": "blocked", "note": "no data to rerun"});
    settle(&mut verdict, &obligations, false);
    assert!(!verdict.approved);
    assert_eq!(
        findings_text(&verdict.findings),
        format!(
            "- [medium] {}: EUR + PLN ({})\n- [medium] could not be settled: {}: no data to rerun ({})",
            first.1, first.0, second.1, second.0
        )
    );
}

#[test]
fn an_issue_on_any_obligation_rejects_a_read_only_report_too() {
    let task: TaskContract = serde_json::from_value(serde_json::json!({
        "id": "t1", "title": "Audit", "goal": "g", "kind": "analysis"
    }))
    .unwrap();
    let obligations = obligations(&task);
    let risk = obligations[1].0.clone();
    let coverage = |status: &str| {
        let mut map = serde_json::Map::new();
        for (id, _) in &obligations {
            map.insert(
                id.clone(),
                serde_json::json!({"status": "ok", "note": "checked"}),
            );
        }
        map.insert(
            risk.clone(),
            serde_json::json!({"status": status, "note": "open in code"}),
        );
        map
    };
    let mut verdict = Verdict {
        resolved: Vec::new(),
        approved: true,
        findings: vec![],
        coverage: coverage("issue"),
    };
    settle(&mut verdict, &obligations, true);
    assert!(!verdict.approved);
    assert_eq!(verdict.findings[0]["where"], risk.as_str());
    let mut clean = Verdict {
        resolved: Vec::new(),
        approved: true,
        findings: vec![],
        coverage: coverage("ok"),
    };
    settle(&mut clean, &obligations, true);
    assert!(clean.approved && clean.findings.is_empty());
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
fn findings_render_as_text() {
    let findings = vec![json!({"issue": "Preserve Ctrl+O."})];
    assert_eq!(findings_text(&findings), "- Preserve Ctrl+O.");
}

#[test]
fn only_medium_and_low_findings_are_minor() {
    let rejected = |findings: serde_json::Value| verdict(false, findings);
    assert!(only_minor_findings(&rejected(
        json!([{"severity": "medium", "issue": "x"}, {"severity": "low", "issue": "y"}])
    )));
    assert!(!only_minor_findings(&rejected(
        json!([{"severity": "medium", "issue": "x"}, {"severity": "high", "issue": "y"}])
    )));
    assert!(!only_minor_findings(&rejected(json!([]))));
}

fn verdict(approved: bool, findings: serde_json::Value) -> Verdict {
    Verdict {
        resolved: Vec::new(),
        approved,
        findings: serde_json::from_value(findings).unwrap(),
        coverage: serde_json::Map::new(),
    }
}

#[test]
fn a_verifier_is_shown_its_earlier_blocking_findings_by_runtime_id() {
    let task: TaskContract = serde_json::from_value(serde_json::json!({
        "id": "w", "title": "Writer", "goal": "Do it", "kind": "implementation",
        "verifier": {"focus": "f"}
    }))
    .unwrap();
    let earlier = vec![
        serde_json::json!({"id": "goal:aaaa1111", "check": "goal", "severity": "medium", "issue": "map omits a file", "where": "x"}),
        serde_json::json!({"id": "goal:low00000", "check": "goal", "severity": "low", "issue": "typo", "where": "y"}),
    ];
    let prompt = verifier_turn_prompt("/p", &task, "report", None, false, "", &earlier);
    assert!(prompt.contains("- goal:aaaa1111 [goal, medium] map omits a file"));
    assert!(!prompt.contains("goal:low00000"));
    assert!(prompt.contains("List in resolved"));
    assert!(prompt.contains("When it fixed part of one, list that id too"));
    assert!(prompt.contains("new finding for only what remains"));
    assert!(prompt.contains(
        "list every violation, each with its location and size, in one finding; never give examples"
    ));
    let first = verifier_turn_prompt("/p", &task, "report", None, true, "", &[]);
    assert!(!first.contains("List in resolved"));
}

#[test]
fn an_options_verifier_leaves_the_writers_tests_and_validation_to_the_writer() {
    let options: TaskContract = serde_json::from_value(serde_json::json!({
        "id": "w-options", "title": "Options for W", "goal": "g", "kind": "options",
        "options_for": "w", "verifier": {"focus": "f"}
    }))
    .unwrap();
    let writer: TaskContract = serde_json::from_value(serde_json::json!({
        "id": "w", "title": "W", "goal": "g", "kind": "implementation", "writes": true,
        "verifier": {"focus": "f"}
    }))
    .unwrap();
    for prompt in [
        verifier_prompt("/p", &options, "report", None),
        verifier_followup(&options, "report", None),
    ] {
        assert!(prompt.contains(SELECTION_VERIFICATION), "{prompt}");
    }
    assert!(!verifier_prompt("/p", &writer, "report", None).contains(SELECTION_VERIFICATION));
}

#[test]
fn a_report_verifier_names_every_concrete_gap_at_once_and_accepts_stated_residual_paths() {
    let audit: TaskContract = serde_json::from_value(serde_json::json!({
        "id": "audit", "title": "Audit", "goal": "g", "kind": "performance",
        "verifier": {"focus": "f"}
    }))
    .unwrap();
    for prompt in [
        verifier_prompt("/p", &audit, "report", None),
        verifier_followup(&audit, "report", None),
    ] {
        assert!(prompt.contains(REPORT_VERIFICATION), "{prompt}");
        assert!(!prompt.contains(SELECTION_VERIFICATION), "{prompt}");
    }
}

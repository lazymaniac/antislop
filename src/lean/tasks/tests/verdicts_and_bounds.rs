use super::*;

#[test]
fn a_reviewer_owes_the_goal_the_assigned_criteria_and_the_kind_catalog_only() {
    let mut task: TaskContract = serde_json::from_value(serde_json::json!({
        "id": "t1", "title": "Revenue", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal": "g", "change_kind": "none", "kind": "analysis",
        "reviewer": {"summary": "f"}}))
    .unwrap();
    task.acceptance_criteria = vec![AcceptanceCriterion {
        id: "C2".into(),
        text: "Assigned acceptance text".into(),
    }];
    let ids: Vec<String> = obligations(&task).into_iter().map(|(id, _)| id).collect();
    let catalog: Vec<String> = crate::lean::risks::for_kind(task.kind)
        .map(|(id, _)| id)
        .collect();
    assert!(!catalog.is_empty());
    let mut expected = vec![GOAL_CHECK.to_owned(), "C2".to_owned()];
    expected.extend(catalog);
    assert_eq!(
        ids, expected,
        "no planner risks, common checks or checklist"
    );
}

#[test]
fn a_verdict_counts_only_with_every_obligation_accounted_for() {
    let task: TaskContract = serde_json::from_value(serde_json::json!({
        "id": "t1", "title": "Revenue", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal": "g", "change_kind": "none", "kind": "analysis"
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
            "- check:{} [medium] {}: EUR + PLN ({})\n- check:{} [medium] could not be settled: {}: no data to rerun ({})",
            first.0, first.1, first.0, second.0, second.1, second.0
        )
    );
}

#[test]
fn an_issue_on_any_obligation_rejects_a_read_only_report_too() {
    let task: TaskContract = serde_json::from_value(serde_json::json!({
        "id": "t1", "title": "Audit", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal": "g", "change_kind": "none", "kind": "analysis"
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

fn verdict(approved: bool, findings: serde_json::Value) -> Verdict {
    Verdict {
        resolved: Vec::new(),
        approved,
        findings: serde_json::from_value(findings).unwrap(),
        coverage: serde_json::Map::new(),
    }
}

#[test]
fn a_reviewer_is_shown_its_earlier_blocking_findings_by_runtime_id() {
    let task: TaskContract = serde_json::from_value(serde_json::json!({
        "id": "w", "title": "Writer", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal": "Do it", "change_kind": "code", "kind": "implementation",
        "reviewer": {"summary": "f"}}))
    .unwrap();
    let earlier = vec![
        serde_json::json!({"id": "goal:aaaa1111", "check": "goal", "severity": "medium", "issue": "map omits a file", "where": "x"}),
        serde_json::json!({"id": "goal:low00000", "check": "goal", "severity": "low", "issue": "typo", "where": "y"}),
    ];
    let prompt = reviewer_turn_prompt("/p", &task, "report", None, false, "", &earlier);
    assert!(prompt.contains("- goal:aaaa1111 [goal, medium] map omits a file"));
    assert!(!prompt.contains("goal:low00000"));
    assert!(prompt.contains("List in resolved"));
    assert!(prompt.contains("When it fixed part of one, list that id too"));
    assert!(prompt.contains("new finding for only what remains"));
    assert!(prompt.contains("List all measured violations of an assigned bound"));
    let first = reviewer_turn_prompt("/p", &task, "report", None, true, "", &[]);
    assert!(!first.contains("List in resolved"));
}

#[test]
fn an_options_reviewer_leaves_the_writers_tests_and_validation_to_the_writer() {
    let options: TaskContract = serde_json::from_value(serde_json::json!({
        "id": "w-options", "title": "Options for W", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal": "g", "change_kind": "none", "kind": "options",
        "options_for": "w", "reviewer": {"summary": "f"}}))
    .unwrap();
    let writer: TaskContract = serde_json::from_value(serde_json::json!({
        "id": "w", "title": "W", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal": "g", "change_kind": "code", "kind": "implementation",
        "reviewer": {"summary": "f"}}))
    .unwrap();
    for prompt in [
        reviewer_prompt("/p", &options, "report", None),
        reviewer_followup(&options, "report", None),
    ] {
        assert!(prompt.contains(SELECTION_REVIEW), "{prompt}");
    }
    assert!(!reviewer_prompt("/p", &writer, "report", None).contains(SELECTION_REVIEW));
}

#[test]
fn code_review_workers_and_reviewers_are_read_only_and_validate_findings_independently() {
    let review: TaskContract = serde_json::from_value(serde_json::json!({
        "id":"review", "title":"Review scope", "goal":"Review the named diff and report findings.",
        "change_kind":"none", "kind":"analysis"
    }))
    .unwrap();
    let mut review = review;
    review.reviewer.summary = Some(CODE_REVIEW_REVIEWER_SUMMARY.into());
    let worker = worker_prompt("/p", &review);
    assert!(worker.contains("read-only code-review task"));
    assert!(worker.contains("do not modify, create, delete or commit project files"));
    let fresh = reviewer_prompt("/p", &review, "Scope and candidate findings", None);
    let resumed = reviewer_followup(&review, "Updated report", None);
    for prompt in [fresh, resumed] {
        assert!(
            prompt.contains("Independently validate every candidate finding"),
            "{prompt}"
        );
        assert!(
            prompt.contains("Separate confirmed findings from unresolved questions"),
            "{prompt}"
        );
        assert!(
            prompt.contains("An empty confirmed-findings list is valid"),
            "{prompt}"
        );
        assert!(prompt.contains("read-only"), "{prompt}");
    }
}

#[test]
fn on_demand_review_keeps_task_local_criteria_and_preserves_the_reviewed_source() {
    let task: TaskContract = serde_json::from_value(json!({
        "id":"send", "title":"Send", "goal":"Reject incompatible images", "kind":"implementation", "change_kind":"code",
        "acceptance_criteria":[{"id":"C4","text":"After a provider error the draft text and usable attachments remain recoverable"}]
    })).unwrap();
    let prompt = code_review_prompt("/p", &task, "base", Some("result"), &[]);
    assert!(prompt.contains(
        "C4: After a provider error the draft text and usable attachments remain recoverable"
    ));
    assert!(prompt.contains(READ_ONLY_REVIEW));
    assert!(prompt.contains("Trace real call paths to observable success"));
}

#[test]
fn code_review_final_audit_has_valid_empty_and_finding_report_requirements() {
    let prompt = final_scope_audit_prompt("/p", "Review src/lib.rs", "No findings were proposed.");
    for required in [
        "exact reviewed scope",
        "overall verdict",
        "triggering condition",
        "supporting evidence",
        "unresolved questions",
        "checks and outcomes",
        "limitations and coverage gaps",
        "A valid review may have no confirmed findings",
        "do not modify files",
    ] {
        assert!(prompt.contains(required), "missing {required}: {prompt}");
    }
}

#[test]
fn a_report_reviewer_checks_semantic_scope_and_disclosed_in_scope_gaps() {
    let audit: TaskContract = serde_json::from_value(serde_json::json!({
        "id": "audit", "title": "Audit", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal": "g", "change_kind": "none", "kind": "performance",
        "reviewer": {"summary": "f"}}))
    .unwrap();
    for prompt in [
        reviewer_prompt("/p", &audit, "report", None),
        reviewer_followup(&audit, "report", None),
    ] {
        assert!(prompt.contains(REPORT_REVIEW), "{prompt}");
        assert!(!prompt.contains(SELECTION_REVIEW), "{prompt}");
    }
}

#[test]
fn a_later_report_round_preserves_the_severity_of_new_in_scope_defects() {
    let audit: TaskContract = serde_json::from_value(serde_json::json!({
        "id": "audit", "title": "Audit", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal": "g", "change_kind": "none", "kind": "security",
        "reviewer": {"summary": "f"}}))
    .unwrap();
    let earlier =
        [serde_json::json!({"id": "f1", "check": "R1", "severity": "medium", "issue": "x"})];
    for fresh in [true, false] {
        let first = reviewer_turn_prompt("/p", &audit, "report", None, fresh, "", &[]);
        assert!(
            first.contains("New in-scope defects keep their actual severity"),
            "{first}"
        );
        let later = reviewer_turn_prompt("/p", &audit, "report", None, fresh, "", &earlier);
        assert!(
            later.contains("New in-scope defects keep their actual severity"),
            "{later}"
        );
    }
    let mut writer = audit.clone();
    writer.change_kind = crate::lean::tasks::ChangeKind::Code;
    let prompt = reviewer_turn_prompt("/p", &writer, "report", None, false, "", &earlier);
    assert!(
        prompt.contains("New in-scope defects keep their actual severity"),
        "{prompt}"
    );
}

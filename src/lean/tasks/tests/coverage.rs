use super::*;

fn verdict_with(coverage: serde_json::Map<String, Value>) -> Verdict {
    Verdict {
        approved: true,
        findings: vec![],
        coverage,
    }
}

#[test]
fn coverage_counts_real_obligations_and_splits_their_answers() {
    let obligations = vec![
        ("R1".to_owned(), "risk".to_owned()),
        ("V1".to_owned(), "check".to_owned()),
        ("V2".to_owned(), "check".to_owned()),
        ("C1".to_owned(), "check".to_owned()),
    ];
    let mut entries = serde_json::Map::new();
    entries.insert("R1".into(), json!({"status": "ok", "note": "fine"}));
    entries.insert("V1".into(), json!({"status": "ISSUE", "note": "breaks"}));
    entries.insert("V2".into(), json!({"status": "n/a", "note": "no i18n"}));
    entries.insert("C1".into(), json!({"status": "blocked", "note": ""}));
    let counted = coverage(&verdict_with(entries), &obligations);
    // C1's empty note is no answer, not an answer of another kind.
    assert_eq!(
        counted,
        json!({"total": 4, "ok": 1, "issue": 1, "blocked": 0,
                   "na": 1, "missing": 1, "unknown": [], "unknown_more": 0})
    );
}

#[test]
fn coverage_never_counts_an_unknown_id_as_an_obligation() {
    let obligations = vec![("R1".to_owned(), "risk".to_owned())];
    let mut entries = serde_json::Map::new();
    entries.insert("R1".into(), json!({"status": "ok", "note": "fine"}));
    entries.insert("X9".into(), json!({"status": "ok", "note": "invented"}));
    entries.insert(
        "R0".into(),
        json!({"status": "ok", "note": "before the first"}),
    );
    let counted = coverage(&verdict_with(entries), &obligations);
    assert_eq!(
        counted,
        json!({"total": 1, "ok": 1, "issue": 0, "blocked": 0,
                   "na": 0, "missing": 0, "unknown": ["R0", "X9"], "unknown_more": 0})
    );
    // No verdict yet: the obligations are missing, not silently ok.
    let none = coverage(&verdict_with(serde_json::Map::new()), &obligations);
    assert_eq!(none["total"], 1);
    assert_eq!(none["missing"], 1);
    assert_eq!(none["ok"], 0);
}

#[test]
fn coverage_previews_unknown_ids_bounded_and_counts_the_omitted() {
    let obligations = vec![("R1".to_owned(), "risk".to_owned())];
    let mut entries = serde_json::Map::new();
    entries.insert("R1".into(), json!({"status": "ok", "note": "fine"}));
    // Eleven unknown ids, one of them absurdly long: a hostile or
    // confused verdict must not make the card's preview unbounded.
    for i in 0..11 {
        entries.insert(format!("W{i}"), json!({"status": "ok", "note": "n"}));
    }
    let enormous = "x".repeat(500);
    entries.insert(enormous.clone(), json!({"status": "ok", "note": "n"}));
    let counted = coverage(&verdict_with(entries.clone()), &obligations);
    let shown = counted["unknown"].as_array().expect("unknown list");
    assert_eq!(shown.len(), UNKNOWN_ID_LIMIT);
    assert!(
        shown
            .iter()
            .all(|id| id.as_str().unwrap().len() <= UNKNOWN_ID_BYTES),
        "{shown:?}"
    );
    assert_eq!(
        counted["unknown_more"].as_u64().unwrap(),
        12 - UNKNOWN_ID_LIMIT as u64
    );
    // The counts never touched the preview's cut: the real obligations
    // settle exactly as before and the verdict keeps every id whole.
    assert_eq!(counted["total"], 1);
    assert_eq!(counted["ok"], 1);
    assert!(
        entries.contains_key(&enormous),
        "the full verdict still holds the whole id"
    );
    // An id longer than the preview keeps the mark of the cut.
    let mut long_only = serde_json::Map::new();
    long_only.insert(enormous.clone(), json!({"status": "ok", "note": "n"}));
    let only = coverage(&verdict_with(long_only), &obligations)["unknown"][0]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        only.ends_with(CUT_MARK) && only.len() <= UNKNOWN_ID_BYTES,
        "{only}"
    );
}

use super::*;

#[test]
fn card_outcomes_have_one_stable_name_each() {
    for (outcome, name) in [
        (Outcome::Committed, "committed"),
        (Outcome::Applied, "applied"),
        (Outcome::LeftOnBranch, "left_on_branch"),
        (Outcome::NoChanges, "no_changes"),
        (Outcome::Conflict, "conflict"),
        (Outcome::UserEdits, "user_edits"),
        (Outcome::NotApplicable, "not_applicable"),
        (Outcome::Cancelled, "cancelled"),
    ] {
        assert_eq!(serde_json::to_value(outcome).unwrap(), json!(name));
        assert_eq!(outcome.name(), name);
        assert_eq!(Outcome::named(name), Some(outcome));
    }
}

#[test]
fn every_outcome_has_its_own_honest_label() {
    let mut labels = std::collections::HashSet::new();
    for (outcome, _, label) in OUTCOMES {
        assert_eq!(outcome.label(), label);
        assert!(!label.is_empty() && label != "not reported");
        assert!(labels.insert(label), "label {label} used twice");
    }
    // An unknown or missing name is never dressed up as a landing.
    assert_eq!(Outcome::named("quantum-merge"), None);
}

#[test]
fn restarting_actions_are_named_once_for_both_layers() {
    for action in [action::RETRY_WITH_GUIDANCE, action::UNBLOCK] {
        assert!(resets_card(action), "{action} must clear the card");
    }
    for action in [
        action::INTEGRATE_WITH_CONSENT,
        action::KEEP_ON_BRANCH,
        action::VERIFY_MANUAL_SNAPSHOT,
        action::CANCEL_TASK,
        "anything-else",
    ] {
        assert!(!resets_card(action), "{action} keeps the round's card");
    }
    assert_eq!(
        recovery_outcome(action::KEEP_ON_BRANCH),
        Some(Outcome::LeftOnBranch.name())
    );
    assert_eq!(
        recovery_outcome(action::CANCEL_TASK),
        Some(Outcome::Cancelled.name())
    );
    assert_eq!(recovery_outcome(action::RETRY_WITH_GUIDANCE), None);
}

#[test]
fn card_data_stays_with_the_round_it_was_stamped_for() {
    assert!(feedback_is_current(&json!({"round": 2}), 2));
    assert!(!feedback_is_current(&json!({"round": 1}), 2));
    assert!(!feedback_is_current(&json!({"round": 3}), 2));
    assert!(!feedback_is_current(&Value::Null, 0));
    assert!(!feedback_is_current(&json!({}), 0));
}

#[test]
fn card_counts_and_unaccounted_read_one_status_the_same_way() {
    // `na` without the slash is not a status the verdict format allows:
    // it is no answer for the counters and none for the runtime alike.
    let obligations = vec![("R1".to_owned(), "risk".to_owned())];
    let mut entries = serde_json::Map::new();
    entries.insert("R1".into(), json!({"status": "na", "note": "excuse"}));
    let verdict = verdict_with(entries);
    assert_eq!(coverage(&verdict, &obligations)["na"], 0);
    assert_eq!(coverage(&verdict, &obligations)["missing"], 1);
    assert_eq!(
        unaccounted(&verdict, &obligations)
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>(),
        ["R1"]
    );
}

#[test]
fn the_effect_summary_is_bounded_and_names_the_files() {
    let files: Vec<String> = (0..25).map(|i| format!("src/m{i}.rs")).collect();
    let stat = " src/m0.rs | 2 ++\n 25 files changed, 50 insertions(+)";
    let (summary, _cut) = effect(stat, &files);
    assert!(summary.len() <= EFFECT_LIMIT, "{summary}");
    // The counts survive the cut; only trailing names are dropped, and
    // the summary always says how many more there are.
    assert!(summary.starts_with("25 files changed, 50 insertions(+)"));
    assert!(summary.contains("src/m0.rs"), "{summary}");
    assert!(summary.contains("more)"), "{summary}");
    let named = files
        .iter()
        .filter(|f| summary.contains(f.as_str()))
        .count();
    assert!(named >= 1 && named < files.len(), "named {named}");
    // A diff stat without a summary line still reports the file count.
    let (odd, odd_cut) = effect("binary files differ", &["a.png".to_owned()]);
    assert_eq!(odd, "1 file(s) changed — a.png");
    assert!(!odd_cut);
    assert_eq!(effect("", &[]), ("no file changes".to_owned(), false));
}

#[test]
fn a_read_only_effect_is_the_reports_opening_line() {
    let report = "```json\n{}\n```\nFound 3 leaks in store.rs.\nDetails follow.";
    let (found, cut) = report_effect(report);
    assert_eq!(found, "Found 3 leaks in store.rs.");
    assert!(!cut);
    assert_eq!(report_effect("  \n\n"), (String::new(), false));
    let long = "x".repeat(EFFECT_LIMIT + 50);
    let (bounded, cut) = report_effect(&long);
    // Bytes are the budget: whole ASCII clusters up to the limit, and
    // the last grapheme is the mark of the cut.
    assert_eq!(bounded.len(), EFFECT_LIMIT);
    assert!(cut && bounded.ends_with(CUT_MARK), "{bounded}");
}

use super::*;
use crate::lean::tasks::{UNKNOWN_ID_BYTES, UNKNOWN_ID_LIMIT};
use crate::lean::tui::view::TaskRow;
use serde_json::json;

fn row(status: &str, round: u64) -> TaskRow {
    TaskRow {
        id: "t1".into(),
        status: status.into(),
        round,
        ..TaskRow::default()
    }
}

#[test]
fn rounds_appear_only_once_work_reached_them() {
    assert_eq!(round_shown(0), None);
    assert_eq!(round_shown(1).as_deref(), Some("r1"));
    let worker = worker_state(&View::default(), &row("queued", 0));
    assert_eq!(worker, "queued");
    assert!(!worker.contains('r'));
    assert_eq!(
        worker_state(&View::default(), &row("working", 1)),
        "working r1"
    );
    assert_eq!(verifier_state(&row("working", 1)), "waits for worker r1");
    assert_eq!(verifier_state(&row("proposed", 0)), "waits for worker");
}

#[test]
fn the_activity_line_is_its_own_and_the_state_line_its_own() {
    // A queued task waiting on a dependency: the state stays short and
    // the concrete wait gets its own line, provider-independent.
    let mut view = View::default();
    let mut parent = TaskRow {
        id: "t0".into(),
        status: "working".into(),
        ..TaskRow::default()
    };
    parent.summary = Some("Parent work.".into());
    let mut waiting = row("queued", 2);
    waiting.depends_on = vec!["t0".into()];
    view.tasks = vec![parent, waiting.clone()];
    assert_eq!(worker_status(&waiting), "queued r2");
    assert_eq!(
        worker_activity(&view, &waiting).as_deref(),
        Some("waits for t0")
    );
    assert_eq!(worker_state(&view, &waiting), "queued r2 · waits for t0");
    assert_eq!(
        with_provider(&worker_status(&waiting), Some("codex")),
        "queued r2 · codex"
    );
    assert_eq!(with_provider("queued r2", Some("")), "queued r2");
    // A running worker shows its activity as its own line; the verifier
    // never borrows it, and the state line never carries it.
    view.agents.insert(
        "worker:t1".into(),
        crate::lean::tui::view::Agent {
            running: true,
            provider: "codex".into(),
            activity: "reading  src/a.rs".into(),
            ..Default::default()
        },
    );
    let working = row("working", 1);
    assert_eq!(worker_status(&working), "working r1");
    assert_eq!(
        worker_activity(&view, &working).as_deref(),
        Some("doing: reading src/a.rs")
    );
    assert!(!worker_status(&working).contains("reading"));
    // Placeholder activity is no activity.
    view.agents.insert(
        "worker:t1".into(),
        crate::lean::tui::view::Agent {
            running: true,
            activity: "starting turn".into(),
            ..Default::default()
        },
    );
    assert_eq!(worker_activity(&view, &working), None);
    // The verifier's own line belongs to its own status and agent.
    let checking = row("verifying", 1);
    assert_eq!(verifier_status(&checking), "checking r1");
    assert_eq!(verifier_activity(&view, &checking), None);
    view.agents.insert(
        "verifier:t1".into(),
        crate::lean::tui::view::Agent {
            running: true,
            activity: "checking  the diff".into(),
            ..Default::default()
        },
    );
    assert_eq!(
        verifier_activity(&view, &checking).as_deref(),
        Some("doing: checking the diff")
    );
    // A stop puts the cause on the activity line, and the combined
    // reading stays word-for-word what the split says.
    let mut failed = row("failed", 2);
    failed.reason = Some("  it conflicted\n".into());
    assert_eq!(worker_status(&failed), "failed r2");
    assert_eq!(
        worker_activity(&view, &failed).as_deref(),
        Some("it conflicted")
    );
    assert_eq!(worker_state(&view, &failed), "failed r2 · it conflicted");
    assert_eq!(
        verifier_activity(&view, &failed).as_deref(),
        Some("it conflicted")
    );
    let quiet = row("needs_user", 1);
    assert_eq!(worker_status(&quiet), "needs decision r1");
    assert_eq!(
        worker_activity(&view, &quiet).as_deref(),
        Some("cause not reported")
    );
    // An idle approved task has nothing to put on the line, and says so
    // by staying empty rather than inventing filler.
    let done = row("approved", 1);
    assert_eq!(worker_activity(&view, &done), None);
    assert_eq!(verifier_activity(&view, &done), None);
    assert_eq!(worker_state(&view, &done), "approved r1");
}

#[test]
fn live_activity_never_shows_the_placeholder_or_a_stopped_turn() {
    let mut view = View::default();
    view.agents.insert(
        "plan verifier".into(),
        crate::lean::tui::view::Agent {
            running: true,
            activity: "exec  two  words".into(),
            ..Default::default()
        },
    );
    assert_eq!(
        doing(&view, "plan verifier").as_deref(),
        Some("exec two words")
    );
    view.agents.insert(
        "auditor".into(),
        crate::lean::tui::view::Agent {
            running: true,
            activity: "starting turn".into(),
            ..Default::default()
        },
    );
    assert_eq!(
        doing(&view, "auditor"),
        None,
        "a placeholder is not activity"
    );
    view.agents.insert(
        "worker:t9".into(),
        crate::lean::tui::view::Agent {
            running: false,
            activity: "✗ failed: boom".into(),
            ..Default::default()
        },
    );
    assert_eq!(
        doing(&view, "worker:t9"),
        None,
        "a stopped turn is not live activity"
    );
    assert_eq!(doing(&view, "nobody"), None);
}

#[test]
fn stops_name_the_cause_and_never_invent_one() {
    let mut failed = row("failed", 2);
    failed.reason = Some("conflicts with the current branch in src/a.rs".into());
    assert_eq!(
        worker_state(&View::default(), &failed),
        "failed r2 · conflicts with the current branch in src/a.rs"
    );
    let quiet = row("failed", 1);
    assert_eq!(
        worker_state(&View::default(), &quiet),
        "failed r1 · cause not reported"
    );
    // A queued task with no reported reason gets no invented excuse.
    let queued = worker_state(&View::default(), &row("queued", 0));
    assert!(!queued.contains("limit") && !queued.contains("conflict"));
}

#[test]
fn coverage_is_counts_or_a_question_never_a_quiet_zero() {
    let bare = row("verifying", 1);
    assert_eq!(coverage_short(&bare), "?");
    assert_eq!(coverage_full(&bare), "not reported");
    let mut reported = row("verifying", 1);
    reported.obligations = Some(json!({
        "total": 8, "ok": 5, "issue": 1, "blocked": 0, "na": 0,
        "missing": 2, "unknown": ["X9"]
    }));
    assert_eq!(coverage_short(&reported), "6/8");
    let full = coverage_full(&reported);
    assert!(full.contains("6 of 8 accounted"), "{full}");
    assert!(full.contains("ids not in the plan: X9"), "{full}");
}

#[test]
fn effects_mark_fragments_and_say_when_none_came() {
    assert_eq!(effect_line(&row("working", 1)), None);
    assert_eq!(effect_front(&row("working", 1)), None);
    let mut running = row("working", 1);
    running.integration = Some(json!({"outcome": "applied"}));
    assert_eq!(
        effect_line(&running).as_deref(),
        Some("effect: not reported by worker")
    );
    assert_eq!(
        effect_front(&running).as_deref(),
        Some("effect: not reported by worker")
    );
    let mut cut = row("approved", 1);
    cut.effect = Some("3 files changed — src/a.rs, src/b…".into());
    cut.effect_cut = Some(true);
    assert!(effect_line(&cut).unwrap().ends_with("[fragment]"));
    // Both shapes come from the one reading: the same text and the one
    // fragment verdict, only the marker's seat differs.
    let trailing = effect_line(&cut).unwrap();
    let leading = effect_front(&cut).unwrap();
    let body = "3 files changed — src/a.rs, src/b…";
    assert_eq!(trailing, format!("effect: {body} [fragment]"));
    assert_eq!(leading, format!("effect: [fragment] {body}"));
    // The text is one normalized line for every view alike.
    let mut spaced = row("approved", 1);
    spaced.effect = Some("two  spaces\n\tand a\ttab".into());
    assert_eq!(
        effect_line(&spaced).as_deref(),
        Some("effect: two spaces and a tab")
    );
    assert_eq!(
        effect_front(&spaced).as_deref(),
        Some("effect: two spaces and a tab")
    );
}

#[test]
fn only_explicit_metadata_marks_an_effect_as_a_fragment() {
    // An honest sentence ending in an ellipsis is no fragment once the
    // daemon's stamp says the stored text was never cut.
    let mut honest = row("approved", 1);
    honest.effect = Some("fixed the ellipsis bug in src/a.rs…".into());
    honest.effect_cut = Some(false);
    assert_eq!(
        effect_line(&honest).as_deref(),
        Some("effect: fixed the ellipsis bug in src/a.rs…"),
        "a natural ellipsis never marks a fragment on stamped feedback"
    );
    // A cut the stamp reports shows even when the kept whole carries
    // no trailing mark (an oversized first cluster kept whole).
    let mut stamped = row("approved", 1);
    stamped.effect = Some("👩‍👩‍👧‍👦".into());
    stamped.effect_cut = Some(true);
    assert_eq!(
        effect_front(&stamped).as_deref(),
        Some("effect: [fragment] 👩‍👩‍👧‍👦"),
        "the stamp decides, and the card puts it in front of the text"
    );
    let mut unstamped = row("approved", 1);
    unstamped.effect = Some("An honest ellipsis…".into());
    assert!(!effect_line(&unstamped).unwrap().contains("[fragment]"));
}

#[test]
fn coverage_render_caps_fat_ids_and_counts_every_one_left_out() {
    // The runtime's preview shape: few short ids plus an honest count
    // of the rest.
    let mut preview = row("verifying", 1);
    preview.obligations = Some(json!({
        "total": 3, "ok": 1, "missing": 2,
        "unknown": ["XX1", "XX2"], "unknown_more": 6,
    }));
    let full = coverage_full(&preview);
    assert!(
        full.contains("ids not in the plan: XX1, XX2 (+6 more)"),
        "{full}"
    );
    // An old replay from before ids were bounded: an oversized list of
    // oversized ids. The renderer lists at most the preview's share,
    // bounds each id, and counts the rest — the true total stands.
    let fat: Vec<String> = (0..12)
        .map(|i| format!("x{i}{}", "y".repeat(200)))
        .collect();
    let mut replay = row("verifying", 1);
    replay.obligations = Some(json!({
        "total": 2, "ok": 0, "missing": 2, "unknown": fat,
    }));
    let full = coverage_full(&replay);
    let listed = full
        .split("ids not in the plan: ")
        .nth(1)
        .expect("ids named")
        .split(" (")
        .next()
        .expect("listing");
    let ids: Vec<&str> = listed.split(", ").collect();
    assert_eq!(ids.len(), UNKNOWN_ID_LIMIT, "{full}");
    assert!(
        ids.iter().all(|id| id.len() <= UNKNOWN_ID_BYTES),
        "every listed id is within the preview bound: {ids:?}"
    );
    assert!(
        full.contains(&format!("(+{} more)", 12 - UNKNOWN_ID_LIMIT)),
        "the ids left out are counted, not lost: {full}"
    );
    assert!(full.starts_with("0 of 2 accounted"), "{full}");
}

#[test]
fn integration_labels_are_the_reported_set() {
    for (outcome, label) in [
        ("committed", "committed"),
        ("applied", "applied uncommitted"),
        ("no_changes", "no changes"),
        ("conflict", "conflict"),
        ("user_edits", "user edits"),
        ("left_on_branch", "on branch"),
        ("not_applicable", "read-only none"),
        ("cancelled", "cancelled"),
    ] {
        let mut r = row("approved", 1);
        r.integration = Some(json!({"outcome": outcome}));
        assert_eq!(integration_label(&r), label);
    }
    assert_eq!(integration_label(&row("approved", 1)), "not reported");
    let mut odd = row("approved", 1);
    odd.integration = Some(json!({"outcome": "quantum-merge"}));
    assert_eq!(integration_label(&odd), "not reported");
}

#[test]
fn the_landing_line_pairs_label_with_the_whole_detail() {
    let mut r = row("approved", 2);
    r.integration = Some(json!({
        "outcome": "committed",
        "detail": "committed as deadbee",
    }));
    assert_eq!(integration_line(&r), "committed · committed as deadbee");
    assert_eq!(
        integration_line(&row("working", 1)),
        "not reported · detail not reported"
    );
}

#[test]
fn next_steps_name_keys_the_frontend_handles() {
    assert!(next_step(&View::default(), &row("proposed", 0)).contains("Ctrl+A"));
    assert!(next_step(&View::default(), &row("waiting", 1)).contains("Ctrl+R"));
    assert!(next_step(&View::default(), &row("failed", 1)).contains("u unblocks"));
    let mut waiting = row("queued", 0);
    waiting.depends_on = vec!["t0".into()];
    let mut view = View::default();
    view.tasks = vec![
        TaskRow {
            id: "t0".into(),
            status: "working".into(),
            ..TaskRow::default()
        },
        waiting.clone(),
    ];
    assert!(
        next_step(&view, &waiting).contains("waits for t0"),
        "dependencies are named, not hinted at"
    );
}

#[test]
fn card_meta_drops_the_least_important_part_first() {
    let mut view = View::default();
    let mut t1 = row("working", 2);
    t1.depends_on = vec!["t0".into()];
    view.tasks = vec![
        TaskRow {
            id: "t0".into(),
            status: "working".into(),
            ..TaskRow::default()
        },
        t1.clone(),
    ];
    let wide = card_meta("worker", &view, &t1, 40);
    assert!(
        wide.contains("D:t0") && wide.starts_with("not reported"),
        "{wide}"
    );
    let narrow = card_meta("worker", &view, &t1, 12);
    assert_eq!(narrow, "not reported", "deps give way, the label stays");
}

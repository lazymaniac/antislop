use super::*;

fn screen(terminal: &Terminal<TestBackend>) -> String {
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>()
}

#[test]
fn a_panel_without_a_terminal_says_what_it_waits_for() {
    let view = View::default();
    let mut details = Details::new("worker:t1".into());
    let mut terminal = Terminal::new(TestBackend::new(120, 20)).unwrap();
    terminal.draw(|frame| details.draw(frame, &view)).unwrap();
    assert!(screen(&terminal).contains("Attaching terminal"));
    details.native.insert(
        "worker:t1".into(),
        super::super::native_view::Snapshot {
            screen: None,
            error: Some("claude exited (exit status: 1): login required".into()),
        },
    );
    terminal.draw(|frame| details.draw(frame, &view)).unwrap();
    let drawn = screen(&terminal);
    assert!(drawn.contains("login required"), "the raw failure is shown");
    details.native.clear();
    details.selected_round = Some(1);
    terminal.draw(|frame| details.draw(frame, &view)).unwrap();
    assert!(screen(&terminal).contains("o resume this session"));
}

#[test]
fn an_attached_terminal_fills_the_session_panel() {
    let view = View::default();
    let mut details = Details::new("worker:t1".into());
    let mut terminal = Terminal::new(TestBackend::new(120, 20)).unwrap();
    // The first draw tells the panel how large its terminal may be.
    terminal.draw(|frame| details.draw(frame, &view)).unwrap();
    let area = details.native_areas[0];
    assert!(area.width > 0 && area.height > 0);
    details.native.insert(
        "worker:t1".into(),
        super::super::native_view::Snapshot {
            screen: Some(crate::lean::terminal::Screen {
                handle: "h".into(),
                revision: 1,
                columns: area.width,
                rows: area.height,
                cursor: None,
                cells: "agent says hello"
                    .chars()
                    .enumerate()
                    .map(|(x, ch)| crate::lean::terminal::Cell {
                        x: x as u16,
                        y: 0,
                        text: ch.to_string(),
                        width: 1,
                        foreground: None,
                        background: None,
                        modifiers: 0,
                    })
                    .collect(),
                error: None,
            }),
            error: None,
        },
    );
    terminal.draw(|frame| details.draw(frame, &view)).unwrap();
    let drawn = screen(&terminal);
    assert!(drawn.contains("agent says hello"));
    assert!(drawn.contains("i terminal"));
}

#[test]
fn footer_numbers_match_each_keys_visible_tabs() {
    for key in [
        "worker:t1",
        "verifier:t1",
        "auditor",
        "plan-verifier",
        "planner",
        "orchestrator",
        "conversation",
    ] {
        for tab in tabs_for(key) {
            for width in [45, 65, 80, 120] {
                let hint = details_footer_hint_for(key, tab, width);
                assert!(
                    hint.starts_with(&format!("1–{} /", tabs_for(key).len())),
                    "{key}: {hint}"
                );
            }
            assert!(!details_footer_hint_for(key, tab, 30).contains("1–"));
        }
    }
}

fn verifier_task(extra: Value) -> Value {
    let mut task = serde_json::json!({
        "spec": {"id": "t1", "title": "T", "goal": "g", "kind": "analysis"},
    });
    task.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    task
}

#[test]
fn checks_show_live_progress_before_any_verdict() {
    let task = verifier_task(serde_json::json!({"checks": [
        {"id": "goal", "state": "settled", "status": "ok", "note": "a.rs:3"},
        {"id": "R085", "state": "checking", "status": null, "note": ""},
    ]}));
    let rows = check_rows(&task);
    assert_eq!(rows[0].state, "settled");
    assert_eq!(rows[0].note.as_deref(), Some("a.rs:3"));
    assert_eq!(rows[1].state, "checking");
    assert_eq!(rows[2].state, "pending");
}

#[test]
fn checks_fall_back_to_the_verdict_and_flag_what_it_skipped() {
    let task = verifier_task(serde_json::json!({"verdict": {"coverage": {
            "goal": {"status": "issue", "note": "broken"}}}}));
    let rows = check_rows(&task);
    assert_eq!(rows[0].status.as_deref(), Some("issue"));
    assert_eq!(rows[0].note.as_deref(), Some("broken"));
    assert_eq!(rows[1].state, "unaccounted");
}

use ratatui::Terminal;
use ratatui::backend::TestBackend;

#[test]
fn ac_copy_01_tab_names_preserved_and_windowed_when_narrow() {
    // Wide: 100 columns -> all tabs rendered
    let line = format_tabs("orchestrator", 0, 100);
    let rendered: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    for tab in tabs_for("orchestrator") {
        let tab = TABS[tab];
        assert!(rendered.contains(tab), "missing tab {tab} in {rendered}");
    }
    assert!(!rendered.contains("Result") && !rendered.contains("Checks"));

    // Narrow: 50 columns -> active tab still visible
    let line = format_tabs("worker:t1", RESULT, 50);
    let rendered: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(
        rendered.contains("[Result]"),
        "active tab Result missing in {rendered}"
    );
}

#[test]
fn tab_window_keeps_the_active_tab_visible_at_minimum_width() {
    let line = format_tabs("worker:t1", COMMENTS, 12);
    let rendered: String = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();
    assert!(
        rendered.contains("[Comments]"),
        "active tab missing: {rendered}"
    );
}

#[test]
fn details_footer_hints_prioritized_by_width() {
    // COMMENTS tab
    assert!(details_footer_hint(COMMENTS, 100).contains("d diff"));
    assert!(!details_footer_hint(COMMENTS, 30).contains("d diff"));
    assert!(details_footer_hint(COMMENTS, 30).contains("Esc"));

    // Other tabs
    assert!(details_footer_hint(0, 100).contains("PgUp/PgDn"));
    assert!(!details_footer_hint(0, 40).contains("PgUp/PgDn"));
    assert!(details_footer_hint(0, 40).contains("Esc"));
}

#[test]
fn planner_plan_preview_tracks_submitted_and_published_versions() {
    let mut view = View::default();
    let mut details = Details::new("planner".into());
    details.switch_tab(PLAN);
    let mut terminal = Terminal::new(TestBackend::new(80, 16)).unwrap();
    terminal.draw(|frame| details.draw(frame, &view)).unwrap();
    assert!(screen(&terminal).contains("No plan submitted yet"));
    view.apply(&serde_json::json!({"seq":1,"turn_id":"p","kind":"turn_started","payload":{"role":"planner"}}));
    let draft = |seq, document| {
        serde_json::json!({
            "seq":seq, "turn_id":"p", "kind":"agent_final", "payload":{
                "text":serde_json::json!({"type":"plan","document":document,"tasks":[]}).to_string()
            }
        })
    };
    view.apply(&draft(2, "# First draft"));
    terminal.draw(|frame| details.draw(frame, &view)).unwrap();
    assert!(screen(&terminal).contains("First draft"));
    assert!(view.plan_doc.is_empty());
    view.apply(&draft(3, "# Updated draft"));
    terminal.draw(|frame| details.draw(frame, &view)).unwrap();
    let drawn = screen(&terminal);
    assert!(drawn.contains("Updated draft"));
    assert!(!drawn.contains("First draft"));
    view.apply(
        &serde_json::json!({"seq":4,"kind":"plan_proposed","payload":{
            "plan":"plan1","document":"# Published plan","tasks":[]
        }}),
    );
    terminal.draw(|frame| details.draw(frame, &view)).unwrap();
    let drawn = screen(&terminal);
    assert!(drawn.contains("Published plan"));
    assert!(drawn.contains("Current plan"));
    assert!(!drawn.contains("Updated draft"));
    for (width, height) in [(11, 4), (2, 2)] {
        let mut tiny = Terminal::new(TestBackend::new(width, height)).unwrap();
        tiny.draw(|frame| details.draw(frame, &view)).unwrap();
    }
}

#[test]
fn details_empty_states() {
    let view = View::default();
    let mut details = Details::new("worker:t1".into());

    // Messages empty state
    details.tab = 1;
    let lines = details.lines(&view);
    assert!(lines.iter().any(|(_, text)| text.contains("No messages")));

    // Tab 5: Comments empty state
    details.tab = COMMENTS;
    let lines = details.lines(&view);
    assert!(lines.iter().any(|(_, text)| text.contains("No comments")));
}

#[test]
fn details_draw_on_tiny_terminals() {
    let view = View::default();
    let mut details = Details::new("orchestrator".into());

    // 60x12 guard floor
    let backend = TestBackend::new(60, 12);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            details.draw(frame, &view);
        })
        .unwrap();

    // 10x4 tiny terminal (must not panic)
    let backend = TestBackend::new(10, 4);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            details.draw(frame, &view);
        })
        .unwrap();

    // 2x2 extreme tiny terminal (must not panic)
    let backend = TestBackend::new(2, 2);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            details.draw(frame, &view);
        })
        .unwrap();
}

#[test]
fn each_agent_has_only_the_tabs_that_mean_something_for_it() {
    let names =
        |key: &str| -> Vec<&str> { tabs_for(key).into_iter().map(|tab| TABS[tab]).collect() };
    assert_eq!(
        names("worker:t1"),
        [
            "Overview",
            "Messages",
            "Result",
            "Verifier",
            "Checks",
            "Comments",
            "Requirements"
        ]
    );
    assert_eq!(
        names("verifier:t1"),
        [
            "Overview",
            "Messages",
            "Result",
            "Verifier",
            "Checks",
            "Comments",
            "Requirements"
        ]
    );
    assert_eq!(names("auditor"), ["Overview", "Review", "Comments"]);
    assert_eq!(names("plan-verifier"), ["Overview", "Review", "Comments"]);
    assert_eq!(names("conversation"), ["Overview", "Messages", "Comments"]);
    assert_eq!(
        names("planner"),
        ["Overview", "Plan", "Messages", "Comments"]
    );

    // Navigation skips what is not there and wraps around.
    let mut details = Details::new("worker:t1".into());
    details.switch_tab(details.neighbour_tab(1));
    assert_eq!(details.tab, 1);
    details.switch_tab(RESULT);
    details.switch_tab(details.neighbour_tab(1));
    assert_eq!(details.tab, VERIFIER);
    details.switch_tab(details.neighbour_tab(1));
    assert_eq!(details.tab, CHECKS);
    details.switch_tab(details.neighbour_tab(1));
    assert_eq!(details.tab, COMMENTS);
    details.switch_tab(details.neighbour_tab(1));
    assert_eq!(details.tab, REQUIREMENTS);
    details.switch_tab(details.neighbour_tab(1));
    assert_eq!(details.tab, 0);
    details.switch_tab(details.neighbour_tab(-1));
    assert_eq!(details.tab, REQUIREMENTS);
    // Number keys count the visible tabs equally for both task roles.
    assert_eq!(details.nth_tab(2), Some(RESULT));
    assert_eq!(details.nth_tab(3), Some(VERIFIER));
    assert_eq!(details.nth_tab(4), Some(CHECKS));
    assert_eq!(details.nth_tab(5), Some(COMMENTS));
    assert_eq!(details.nth_tab(6), Some(REQUIREMENTS));
    assert_eq!(details.nth_tab(7), None);
    let verifier = Details::new("verifier:t1".into());
    assert_eq!(verifier.nth_tab(4), Some(CHECKS));
    // Each section opens on its own agent's terminal.
    assert_eq!(details.session_keys(), ["worker:t1"]);
    details.switch_tab(VERIFIER);
    assert_eq!(details.session_keys(), ["verifier:t1"]);
    // The sections are told apart by a divider and the first label.
    let tabs = format_tabs("worker:t1", 0, 120)
        .spans
        .iter()
        .map(|span| span.content.to_string())
        .collect::<String>();
    assert!(tabs.contains("Worker") && tabs.contains("│"), "{tabs}");
}

#[test]
fn reviewers_show_their_latest_verdict_on_the_review_tab() {
    let mut view = View::default();
    view.reviews
        .insert("auditor".into(), vec!["✗ failed: missing tests".into()]);
    let mut details = Details::new("auditor".into());
    details.switch_tab(REVIEW);
    let shown: Vec<String> = details.lines(&view).into_iter().map(|(_, l)| l).collect();
    assert_eq!(shown, ["✗ failed: missing tests"]);
    details.switch_tab(0);
    assert!(
        details
            .lines(&view)
            .iter()
            .all(|(_, line)| !line.contains("missing tests"))
    );
    let mut empty = Details::new("plan-verifier".into());
    empty.switch_tab(REVIEW);
    assert!(empty.lines(&View::default())[0].1.contains("No review yet"));
}

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
        "reviewer:t1",
        "auditor",
        "plan-reviewer",
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

fn reviewer_task(extra: Value) -> Value {
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
    let task = reviewer_task(serde_json::json!({"checks": [
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
    let task = reviewer_task(serde_json::json!({"verdict": {"coverage": {
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
        let tab = tab_name("orchestrator", tab);
        assert!(rendered.contains(tab), "missing tab {tab} in {rendered}");
    }
    assert!(rendered.contains("Response") && !rendered.contains("Checks"));

    // Narrow: 50 columns -> active tab still visible
    let line = format_tabs("worker:t1", RESULT, 50);
    let rendered: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(
        rendered.contains("[Results]"),
        "active tab Results missing in {rendered}"
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
    details.switch_tab(RESULT);
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
    let names = |key: &str| -> Vec<&str> {
        tabs_for(key)
            .into_iter()
            .map(|tab| tab_name(key, tab))
            .collect()
    };
    assert_eq!(names("worker:t1"), ["Worker", "Results", "Comments"]);
    assert_eq!(
        names("reviewer:t1"),
        ["Reviewer", "Review", "Checks", "Comments"]
    );
    assert_eq!(names("auditor"), ["Overview", "Audit", "Comments"]);
    assert_eq!(names("plan-reviewer"), ["Overview", "Review", "Comments"]);
    assert_eq!(names("conversation"), ["Overview", "Comments"]);
    assert_eq!(names("planner"), ["Overview", "Plan", "Comments"]);

    for key in ["worker:t1", "reviewer:t1"] {
        let mut details = Details::new(key.into());
        let tabs = tabs_for(key);
        assert_eq!(details.tab, tabs[0]);
        assert_eq!(details.session_keys(), [key]);
        for tab in tabs.iter().skip(1).chain(tabs.first()) {
            details.switch_tab(details.neighbour_tab(1));
            assert_eq!(details.tab, *tab);
        }
        details.switch_tab(details.neighbour_tab(-1));
        assert_eq!(details.tab, COMMENTS);
        for (n, tab) in tabs.iter().enumerate() {
            assert_eq!(details.nth_tab(n), Some(*tab));
        }
        assert_eq!(details.nth_tab(tabs.len()), None);
        let hidden = if key.starts_with("worker:") {
            CHECKS
        } else {
            PLAN
        };
        details.switch_tab(hidden);
        assert_eq!(details.tab, COMMENTS, "hidden tabs cannot be selected");
        let rendered = format_tabs(key, tabs[0], 120)
            .spans
            .iter()
            .map(|span| span.content.to_string())
            .collect::<String>();
        assert!(!rendered.contains(if key.starts_with("worker:") {
            "Reviewer"
        } else {
            "Worker"
        }));
    }
}

#[test]
fn switching_roles_preserves_round_and_each_roles_search() {
    let mut details = Details::new("worker:t1".into());
    details.task_uuid = Some("uuid".into());
    details.selected_round = Some(2);
    details.query = "worker query".into();
    details.scroll = 8;
    details.fetch_in_flight = true;
    details.switch_role();
    assert!(!details.fetch_in_flight);
    assert_eq!(details.key, "reviewer:t1");
    assert_eq!(details.tab, REVIEWER);
    assert_eq!(details.composer_key().1, "reviewer:uuid");
    assert_eq!(details.selected_round, Some(2));
    assert!(details.query.is_empty());
    details.query = "reviewer query".into();
    details.switch_role();
    assert_eq!(details.key, "worker:t1");
    assert_eq!(details.tab, 0);
    assert_eq!(details.query, "worker query");
    assert_eq!(details.scroll, 8);
    details.switch_key("reviewer:t2".into());
    assert_eq!(details.tab, REVIEWER);
    assert_eq!(details.session_keys(), ["reviewer:t2"]);
}

#[test]
fn reviewers_show_their_latest_verdict_on_the_review_tab() {
    let mut view = View::default();
    view.reviews
        .insert("auditor".into(), vec!["✗ failed: missing tests".into()]);
    let mut details = Details::new("auditor".into());
    details.switch_tab(RESULT);
    let shown: Vec<String> = details.lines(&view).into_iter().map(|(_, l)| l).collect();
    assert_eq!(shown, ["✗ failed: missing tests"]);
    details.switch_tab(0);
    assert!(
        details
            .lines(&view)
            .iter()
            .all(|(_, line)| !line.contains("missing tests"))
    );
    let mut empty = Details::new("plan-reviewer".into());
    empty.switch_tab(RESULT);
    assert!(
        empty.lines(&View::default())[0]
            .1
            .contains("No submitted results")
    );
}

#[test]
fn every_agent_exposes_results_and_keeps_reviewer_output_separate() {
    let roles = [
        "orchestrator",
        "planner",
        "plan-reviewer",
        "auditor",
        "requirements-extraction-analyst",
        "requirements-audit-analyst",
        "requirements-audit-reviewer",
        "comment-orchestrator",
        "worker:t1",
        "reviewer:t1",
    ];
    let state = tempfile::tempdir().unwrap();
    for key in roles {
        let mut details = Details::new(key.into());
        assert_eq!(details.nth_tab(1), Some(RESULT), "{key}");
        details.switch_tab(RESULT);
        let mut builder = super::super::document::Builder::new(state.path()).unwrap();
        builder
            .append("", |sink| sink("Own result: café 👩‍💻"))
            .unwrap();
        let own = builder.finish().unwrap();
        let mut other = super::super::document::Builder::new(state.path()).unwrap();
        other
            .append("", |sink| sink("Another agent's report"))
            .unwrap();
        if key.starts_with("reviewer:") {
            details
                .documents
                .insert("result".into(), other.finish().unwrap());
            details.documents.insert("reviewer_result".into(), own);
        } else {
            details.documents.insert("result".into(), own);
        }
        let view = View::default();
        let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            terminal.draw(|frame| details.draw(frame, &view)).unwrap();
            let shown = screen(&terminal);
            if shown.contains("Own result: café") {
                assert!(!shown.contains("Another agent's report"));
                break;
            }
            assert!(std::time::Instant::now() < deadline, "{key}: {shown}");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}

#[test]
fn result_tab_names_describe_the_roles_deliverable() {
    for (key, name) in [
        ("worker:t1", "Results"),
        ("reviewer:t1", "Review"),
        ("planner", "Plan"),
        ("orchestrator", "Response"),
        ("plan-reviewer", "Review"),
        ("auditor", "Audit"),
        ("requirements-extraction-analyst", "Requirements"),
        ("requirements-audit-analyst", "Audit"),
        ("requirements-audit-reviewer", "Audit review"),
    ] {
        assert_eq!(tab_name(key, RESULT), name);
        assert_eq!(
            tabs_for(key).iter().filter(|tab| **tab == RESULT).count(),
            1
        );
        assert!(!tabs_for(key).contains(&PLAN) && !tabs_for(key).contains(&REVIEW));
    }
}

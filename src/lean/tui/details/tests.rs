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
    assert!(drawn.contains("t terminal"));
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
        "spec": {"id": "t1", "title": "T", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal": "g", "change_kind": "none", "kind": "analysis"},
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
        {"id": "C1", "state": "checking", "status": null, "note": ""},
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
    assert!(details_footer_hint(COMMENTS, 100).contains("Shift+→ changes"));
    assert!(!details_footer_hint(COMMENTS, 30).contains("Shift+→ changes"));
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
        let hidden = PLAN;
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
fn result_documents_are_individually_selectable_and_readable_in_the_session() {
    let state = tempfile::tempdir().unwrap();
    let mut details = Details::new("orchestrator".into());
    details.switch_tab(RESULT);
    let mut result = super::super::document::Builder::new(state.path()).unwrap();
    result
        .append("Session response\n", |sink| sink("Original answer"))
        .unwrap();
    details
        .documents
        .insert("result".into(), result.finish().unwrap());
    for (id, path, text) in [
        ("one", "brief.md", "First output: café"),
        ("two", "notes.txt", "Second output: 👩‍💻"),
    ] {
        let mut builder = super::super::document::Builder::new(state.path()).unwrap();
        builder
            .append(&format!("{path}\n"), |sink| sink(text))
            .unwrap();
        details
            .documents
            .insert(format!("result_document:{id}"), builder.finish().unwrap());
    }
    details.rebuild_result_index();
    let view = View::default();
    let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        terminal.draw(|frame| details.draw(frame, &view)).unwrap();
        let shown = screen(&terminal);
        if shown.contains("First output: café") {
            assert!(shown.contains("brief.md"), "{shown}");
            assert!(!shown.contains("Original answer"), "{shown}");
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    details.change_result_document(1);
    loop {
        terminal.draw(|frame| details.draw(frame, &view)).unwrap();
        let shown = screen(&terminal);
        if shown.contains("Second output:") {
            assert!(shown.contains("notes.txt"), "{shown}");
            assert!(!shown.contains("Original answer"), "{shown}");
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn result_document_loading_state_keeps_document_navigation_visible() {
    let mut details = Details::new("orchestrator".into());
    details.switch_tab(RESULT);
    details.task = Some(serde_json::json!({
        "_result_documents": [
            {"id": "one", "path": "brief.md"},
            {"id": "two", "path": "notes.txt"},
        ],
    }));
    details.rebuild_result_index();
    let selected = details.selected_result_document.clone().unwrap();
    details.result_loading = Some((selected, details.result_selection_generation, 0));

    let mut terminal = Terminal::new(TestBackend::new(140, 16)).unwrap();
    terminal
        .draw(|frame| details.draw(frame, &View::default()))
        .unwrap();
    let shown = screen(&terminal);
    assert!(shown.contains("Loading result document"), "{shown}");
    assert!(shown.contains("Alt+←/→ documents"), "{shown}");
}

#[test]
fn workbench_summary_and_conversation_keep_independent_scroll_and_search() {
    let mut details = Details::new("conversation".into());
    details.scroll = 7;
    details.query = "summary filter".into();
    details.prepare_workbench_details(
        Some(crate::lean::tui::WorkbenchTab::Summary),
        crate::lean::tui::WorkbenchTab::Conversation,
    );
    assert_eq!(details.scroll, 0);
    assert!(details.query.is_empty());
    details.scroll = 3;
    details.query = "conversation filter".into();
    details.prepare_workbench_details(
        Some(crate::lean::tui::WorkbenchTab::Conversation),
        crate::lean::tui::WorkbenchTab::Summary,
    );
    assert_eq!(details.scroll, 7);
    assert_eq!(details.query, "summary filter");
    details.prepare_workbench_details(
        Some(crate::lean::tui::WorkbenchTab::Summary),
        crate::lean::tui::WorkbenchTab::Conversation,
    );
    assert_eq!(details.scroll, 3);
    assert_eq!(details.query, "conversation filter");
}

#[test]
fn result_index_membership_scales_across_metadata_and_cached_documents() {
    let state = tempfile::tempdir().unwrap();
    let mut builder = super::super::document::Builder::new(state.path()).unwrap();
    builder.append("cached", |_| Ok(())).unwrap();
    let document = builder.finish().unwrap();
    let mut details = Details::new("orchestrator".into());
    details.task = Some(serde_json::json!({
        "_result_documents": (0..3_000).map(|index| serde_json::json!({
            "id": format!("metadata-{index:04}"), "path": "result.md"
        })).collect::<Vec<_>>(),
    }));
    for index in 0..3_000 {
        details.documents.insert(
            format!("result_document:cached-{index:04}"),
            document.clone(),
        );
    }
    details.rebuild_result_index();
    assert_eq!(details.visible_result_keys().len(), 6_000);
    assert!(details.has_result_key("result_document:metadata-2999"));
    assert!(details.has_result_key("result_document:cached-2999"));
    assert!(!details.has_result_key("result_document:missing"));
}

#[test]
fn task_refresh_keeps_open_documents_still_present_in_metadata() {
    let state = tempfile::tempdir().unwrap();
    let mut builder = super::super::document::Builder::new(state.path()).unwrap();
    builder.append("cached", |_| Ok(())).unwrap();
    let cached = builder.finish().unwrap();
    let mut details = Details::new("orchestrator".into());
    details.documents.insert("result".into(), cached.clone());
    details
        .documents
        .insert("result_document:kept".into(), cached.clone());
    details
        .documents
        .insert("result_document:removed".into(), cached);
    details.load_task(serde_json::json!({
        "_result_documents": [{"id":"kept","path":"kept.md"},{"id":"new","path":"new.md"}],
    }));
    assert!(details.documents.contains_key("result"));
    assert!(details.documents.contains_key("result_document:kept"));
    assert!(!details.documents.contains_key("result_document:removed"));
    assert!(details.has_result_key("result_document:new"));
    details.load_documents(Default::default());
    assert!(!details.documents.contains_key("result"));
    assert!(details.documents.contains_key("result_document:kept"));
}

#[test]
fn session_snapshot_loads_result_artifact_from_session_api_for_display() {
    use serde_json::json;
    use std::os::unix::net::UnixListener;

    let state = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(crate::lean::daemon::socket(state.path())).unwrap();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let server_seen = std::sync::Arc::clone(&seen);
    let server = std::thread::spawn(move || {
        loop {
            let (mut stream, _) = listener.accept().unwrap();
            let envelope: serde_json::Value =
                crate::transport::read(&mut std::io::BufReader::new(stream.try_clone().unwrap()))
                    .unwrap()
                    .unwrap();
            let request = &envelope["request"];
            let op = request["op"].as_str().unwrap().to_owned();
            server_seen.lock().unwrap().push(op.clone());
            let data = match op.as_str() {
                "sessions" => {
                    json!([{"turn_id":"turn-1","role":"orchestrator","status":"completed"}])
                }
                "session_events" => json!({"events":[],"has_more":false}),
                "result_documents" => {
                    json!([{"id":"doc-1","path":"brief.md","source_commit":"abcdef123456"}])
                }
                "result_document_content" => {
                    assert_eq!(request["id"], "doc-1");
                    json!({"text":"Deliverable loaded through the session API: café 👩‍💻","next_offset":45,"done":true})
                }
                other => panic!("unexpected session request {other}"),
            };
            crate::transport::write(&mut stream, &json!({"data":data,"error":null})).unwrap();
            if op == "result_document_content" {
                break;
            }
        }
    });

    let client = crate::lean::daemon::Client::at(state.path());
    let snapshot =
        crate::lean::tui::inspection::run_snapshot(&client, "conversation-1", "orchestrator", None)
            .unwrap();
    assert_eq!(snapshot.task["_result_documents"][0]["id"], "doc-1");
    assert_eq!(
        snapshot.documents.len(),
        2,
        "snapshot contains only result/review"
    );
    assert!(
        !seen
            .lock()
            .unwrap()
            .contains(&"result_document_content".to_owned()),
        "metadata snapshot must not request unopened artifact content"
    );
    let (updates, receiver) = crate::lean::tui::update_channel(8);
    let mut app = crate::lean::tui::tests::test_app();
    app.client = client;
    app.conversation = "conversation-1".into();
    app.updates = updates;
    app.task_request_id = 1;
    let mut details = Details::new("orchestrator".into());
    details.switch_tab(RESULT);
    app.details = Some(details);
    let round = snapshot.task["round"].as_u64().unwrap_or(0);
    app.apply_update(crate::lean::tui::Update::Task {
        conversation: "conversation-1".into(),
        plan: None,
        generation: app.generation,
        request_id: 1,
        key: "orchestrator".into(),
        round,
        task: snapshot.task,
        documents: snapshot.documents,
    });
    assert_eq!(
        app.details
            .as_ref()
            .and_then(|details| details.selected_result_document.as_deref()),
        Some("result_document:doc-1"),
        "the visible artifact is selected when metadata arrives"
    );
    let update = receiver
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    app.apply_update(update);
    server.join().unwrap();
    let seen = seen.lock().unwrap();
    assert!(seen.contains(&"result_documents".to_owned()));
    assert!(seen.contains(&"result_document_content".to_owned()));

    let details = app.details.as_mut().unwrap();
    assert!(details.documents.contains_key("result_document:doc-1"));
    let view = View::default();
    let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        terminal.draw(|frame| details.draw(frame, &view)).unwrap();
        let shown = screen(&terminal);
        if shown.contains("Deliverable loaded through the session API: café") {
            assert!(shown.contains("brief.md"), "{shown}");
            break;
        }
        assert!(std::time::Instant::now() < deadline, "{shown}");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn result_document_replies_are_scoped_to_the_current_conversation_and_selection() {
    let state = tempfile::tempdir().unwrap();
    let mut builder = super::super::document::Builder::new(state.path()).unwrap();
    builder
        .append("", |sink| sink("foreign stale body"))
        .unwrap();
    let stale_document = builder.finish().unwrap();

    let mut app = crate::lean::tui::tests::test_app();
    app.conversation = "current-conversation".into();
    app.generation = 4;
    let mut details = Details::new("orchestrator".into());
    details.selected_result_document = Some("result_document:doc-current".into());
    details.result_selection_generation = 3;
    details.result_loading = Some(("result_document:doc-current".into(), 3, 4));
    details.result_metadata.insert(
        "result_document:doc-current".into(),
        serde_json::json!({"id":"doc-current","path":"current.md"}),
    );
    app.details = Some(details);

    app.apply_update(crate::lean::tui::Update::ResultDocument {
        conversation: "previous-conversation".into(),
        generation: 3,
        key: "orchestrator".into(),
        selection: 3,
        id: "result_document:doc-current".into(),
        result: Ok(stale_document.clone()),
    });
    app.apply_update(crate::lean::tui::Update::ResultDocument {
        conversation: "current-conversation".into(),
        generation: 4,
        key: "orchestrator".into(),
        selection: 2,
        id: "result_document:doc-current".into(),
        result: Ok(stale_document),
    });
    let details = app.details.as_ref().unwrap();
    assert!(
        details
            .documents
            .contains_key("result_document:doc-current")
    );
    assert_eq!(details.result_loading.as_ref().unwrap().1, 3);
    app.details
        .as_mut()
        .unwrap()
        .documents
        .remove("result_document:doc-current");

    app.apply_update(crate::lean::tui::Update::ResultDocument {
        conversation: "current-conversation".into(),
        generation: 4,
        key: "orchestrator".into(),
        selection: 3,
        id: "result_document:doc-current".into(),
        result: Err("artifact was removed".into()),
    });
    let details = app.details.as_ref().unwrap();
    assert!(details.result_loading.is_none());
    assert_eq!(
        details
            .result_error
            .as_ref()
            .map(|(_, error)| error.as_str()),
        Some("artifact was removed")
    );
    assert!(
        !details
            .documents
            .contains_key("result_document:doc-current")
    );
    let mut details = app.details.take().unwrap();
    details.switch_tab(RESULT);
    details.result_index = vec!["result_document:doc-current".into()];
    let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
    terminal
        .draw(|frame| details.draw(frame, &View::default()))
        .unwrap();
    assert!(screen(&terminal).contains("artifact was removed"));
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
    ] {
        assert_eq!(tab_name(key, RESULT), name);
        assert_eq!(
            tabs_for(key).iter().filter(|tab| **tab == RESULT).count(),
            1
        );
        assert!(!tabs_for(key).contains(&PLAN) && !tabs_for(key).contains(&REVIEW));
    }
}

#[test]
fn the_record_of_a_task_before_its_first_round_is_not_a_round() {
    let mut details = Details::new("worker:t1".into());
    details.load_task(serde_json::json!({
        "round": 1,
        "_rounds": [{"round": 0, "status": "continued"}, {"round": 1, "status": "approved"}],
    }));
    assert_eq!(details.available_rounds, vec![1]);
    // A task not started yet still has its round 0.
    details.load_task(serde_json::json!({
        "round": 0,
        "_rounds": [{"round": 0, "status": "queued"}],
    }));
    assert_eq!(details.available_rounds, vec![0]);
}

#[test]
fn the_footer_hint_fits_beside_the_tab_numbers_and_offers_rounds_only_to_tasks() {
    for key in ["worker:t1", "orchestrator", "conversation"] {
        for tab in tabs_for(key) {
            for width in 30..140 {
                let hint = details_footer_hint_for(key, tab, width);
                assert!(
                    unicode_width::UnicodeWidthStr::width(hint.as_str()) <= width,
                    "{key} {tab} {width}: {hint}"
                );
                assert!(
                    hint.ends_with("Esc") || hint.ends_with("Esc back"),
                    "{hint}"
                );
                assert!(!hint.contains("rounds") || key.contains(':'), "{hint}");
                assert!(key != "conversation" || !hint.contains(" m "), "{hint}");
                assert!(
                    hint.contains("rounds") || !key.contains(':') || width < 72,
                    "{hint}"
                );
            }
        }
    }
}

#[test]
fn reviewer_summary_displays_accepted_pending_and_issue_checks() {
    let mut details = Details::new("reviewer:t1".into());
    details.prepare_workbench_details(None, crate::lean::tui::WorkbenchTab::Summary);
    assert_eq!(details.tab, CHECKS);
    let mut view = View::default();
    view.tasks.push(super::super::view::TaskRow {
        id: "t1".into(),
        ..Default::default()
    });
    details.load_task(reviewer_task(serde_json::json!({})));
    let pending = details
        .lines(&view)
        .into_iter()
        .map(|(_, line)| line)
        .collect::<Vec<_>>()
        .join("\n");
    for row in check_rows(details.task.as_ref().unwrap()) {
        assert!(
            pending.contains(&row.id) && pending.contains(&row.text),
            "{pending}"
        );
        assert_eq!(row.state, "pending");
    }
    assert!(pending.contains("all required checks are listed below"));
    details.load_task(reviewer_task(serde_json::json!({"checks":[
        {"id":"goal","state":"settled","status":"ok","note":"Verified goal"},
        {"id":"C1","state":"settled","status":"issue","note":"Missing test"}
    ]})));

    let text = details
        .records(&view)
        .iter()
        .map(|record| record.search_text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        text.contains("1 accepted") && text.contains("1 with issues"),
        "{text}"
    );
    assert!(
        text.contains("Verified goal")
            && text.contains("Missing test")
            && text.contains("not checked yet"),
        "{text}"
    );
    let coverage = check_rows(details.task.as_ref().unwrap())
        .into_iter()
        .map(|row| {
            (
                row.id.clone(),
                serde_json::json!({"status":"ok", "note":format!("Final evidence {}", row.id)}),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    details.load_task(reviewer_task(serde_json::json!({
        "checks":[{"id":"goal","state":"checking","note":"Old live evidence"}],
        "verdict":{"approved":true,"coverage":coverage,
            "findings":[{"severity":"low","issue":"Review note café 👩‍💻","where":"a.rs:3"}]}
    })));
    let final_text = details
        .lines(&view)
        .into_iter()
        .map(|(_, line)| line)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        final_text.contains("minor notes") && final_text.contains("a.rs:3"),
        "{final_text}"
    );
    assert_eq!(final_text.matches("Review note café 👩‍💻").count(), 1);
    assert_eq!(final_text.matches("Final evidence goal").count(), 1);
    assert!(!final_text.contains("Old live evidence") && !final_text.contains("not checked yet"));
    for row in check_rows(details.task.as_ref().unwrap()) {
        assert_eq!(row.state, "settled");
        assert_eq!(row.status.as_deref(), Some("ok"));
    }
    let mut rejected = details.task.clone().unwrap();
    rejected["verdict"]["approved"] = serde_json::json!(false);
    rejected["verdict"]["findings"] = serde_json::json!([]);
    rejected["verdict"]["coverage"]["goal"] =
        serde_json::json!({"status":"issue","note":"Goal needs repair"});
    details.load_task(rejected);
    let rejected_text = details
        .lines(&view)
        .into_iter()
        .map(|(_, line)| line)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(rejected_text.contains("issues found") && rejected_text.contains("Goal needs repair"));
    assert!(!rejected_text.contains("reviewer approved"));
    assert_eq!(
        check_rows(details.task.as_ref().unwrap())[0]
            .status
            .as_deref(),
        Some("issue")
    );
}

#[test]
fn first_session_run_is_available_as_round_zero_but_pre_task_round_is_hidden() {
    let metadata = serde_json::json!({"round":1,"_rounds":[{"round":0},{"round":1}]});
    let mut orchestrator = Details::new("orchestrator".into());
    orchestrator.load_task(metadata.clone());
    assert_eq!(orchestrator.available_rounds, [0, 1]);
    let mut worker = Details::new("worker:t1".into());
    worker.load_task(metadata);
    assert_eq!(worker.available_rounds, [1]);
}

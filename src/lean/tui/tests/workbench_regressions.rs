use super::*;

#[test]
fn numbered_tabs_and_global_agent_keys_respect_native_input_and_editors() {
    let mut app = test_app();
    app.view.tasks = vec![view::TaskRow {
        id: "t1".into(),
        change_kind: crate::lean::tasks::ChangeKind::Code,
        reviewer_summary: Some("review".into()),
        ..Default::default()
    }];
    app.open_details("worker:t1".into());
    assert!(app.navigation.presentation().terminal_only_default);
    assert_eq!(app.navigation.focus, NavigationFocus::WorkbenchTerminal);
    assert!(app.diff.is_none());
    let tabs = workbench::tabs_for("worker:t1", true);
    assert_eq!(tabs.last(), Some(&WorkbenchTab::Conversation));
    assert!(!tabs.contains(&WorkbenchTab::Checks));
    for (index, tab) in tabs.iter().enumerate() {
        app.key(key_press(KeyCode::Char(char::from(b'1' + index as u8))));
        assert_eq!(app.navigation.presentation().tab, *tab);
    }
    app.key(key_press(KeyCode::Right));
    assert_eq!(app.navigation.presentation().tab, WorkbenchTab::Summary);
    app.key(event::KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL));
    assert_eq!(app.navigation.presentation().tab, WorkbenchTab::Summary);
    app.key(event::KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT));
    assert_eq!(app.navigation.presentation().tab, WorkbenchTab::Results);
    let input = app.bind_native_input_for_test();
    app.key(key_press(KeyCode::Char('t')));
    app.key(key_press(KeyCode::Char('1')));
    assert!(
        matches!(input.try_recv(), Ok(native_view::Command::Input(crate::lean::terminal::Input::Key {event})) if event.code == KeyCode::Char('1'))
    );
    assert_eq!(app.navigation.presentation().tab, WorkbenchTab::Results);
    app.key(event::KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT));
    assert_eq!(app.details.as_ref().unwrap().key, "reviewer:t1");
    assert!(!app.native_focused());
    assert!(workbench::tabs_for("reviewer:t1", true).contains(&WorkbenchTab::Checks));
    app.details.as_mut().unwrap().searching = true;
    app.key(event::KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT));
    assert_eq!(app.details.as_ref().unwrap().key, "reviewer:t1");
    app.details.as_mut().unwrap().searching = false;
    for _ in 0..8 {
        app.key(event::KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT));
    }
    assert_eq!(app.details.as_ref().unwrap().key, ORCHESTRATOR);
}

#[test]
fn wheel_over_session_tasks_stops_at_orchestrator_without_scrolling_content() {
    let mut app = test_app();
    app.view.tasks = vec![view::TaskRow {
        id: "t1".into(),
        ..Default::default()
    }];
    app.open_details(ORCHESTRATOR.into());
    app.workbench_select_tab(WorkbenchTab::Conversation);
    let mut terminal = Terminal::new(TestBackend::new(160, 50)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let area = app.workbench_areas.list;
    for _ in 0..20 {
        app.mouse(event::MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: area.x + 1,
            row: area.y + 2,
            modifiers: KeyModifiers::NONE,
        });
    }
    assert_eq!(app.details.as_ref().unwrap().key, ORCHESTRATOR);
    assert_eq!(app.details.as_ref().unwrap().scroll, 0);
    assert_eq!(app.offset, (0, 0));
}

#[test]
fn refreshed_results_keep_ready_layout_while_scrolling_until_content_changes() {
    let state = tempfile::tempdir().unwrap();
    let build = |suffix: &str| {
        let mut builder = document::Builder::new(state.path()).unwrap();
        builder
            .append("", |sink| {
                sink(&format!(
                    "{}\n{suffix}",
                    (0..80)
                        .map(|i| format!("result row {i}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                ))
            })
            .unwrap();
        builder.finish().unwrap()
    };
    let mut details = Details::new("worker:t1".into());
    let old = build("original");
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while old.viewport(80, 10, 0, false).unwrap().is_none() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    details.load_documents(std::collections::HashMap::from([(
        "result".into(),
        old.clone(),
    )]));
    for scroll in 1..40 {
        details.load_documents(std::collections::HashMap::from([(
            "result".into(),
            build("original"),
        )]));
        let current = &details.documents["result"];
        assert_eq!(
            current, &old,
            "reuse the ready document rather than flash a loading frame"
        );
        let page = current
            .viewport(80, 10, scroll, false)
            .unwrap()
            .expect("layout stays ready");
        assert_eq!(page.lines[0].1, format!("result row {scroll}"));
    }
    details.load_documents(std::collections::HashMap::from([(
        "result".into(),
        build("changed"),
    )]));
    assert_ne!(details.documents["result"], old);
}

#[test]
fn merged_agent_shortcuts_wait_for_modal_owner_and_keep_typed_tab_memory() {
    let mut app = test_app();
    app.view.tasks.push(view::TaskRow {
        id: "t1".into(),
        change_kind: crate::lean::tasks::ChangeKind::Code,
        reviewer_summary: Some("review".into()),
        ..Default::default()
    });
    app.open_details("worker:t1".into());
    app.workbench_select_tab(WorkbenchTab::Results);
    app.details.as_mut().unwrap().query = "saved 👩‍💻 é".into();
    app.details.as_mut().unwrap().scroll = 7;
    let input = app.bind_native_input_for_test();
    app.key(key_press(KeyCode::Char('t')));
    app.show_help(Help::Details);
    let navigation = app.navigation.clone();
    for key in [
        key_press(KeyCode::Char('1')),
        event::KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT),
    ] {
        app.key(key);
    }
    app.paste("blocked");
    assert_eq!(app.navigation, navigation);
    assert!(input.try_recv().is_err());
    app.key(key_press(KeyCode::Esc));
    app.key(event::KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT));
    assert_eq!(app.details.as_ref().unwrap().key, "reviewer:t1");
    assert!(!app.native_focused());
    app.key(key_press(KeyCode::Char('3')));
    assert_eq!(
        app.details.as_ref().unwrap().content_tab(),
        Some(WorkbenchTab::Checks)
    );
    app.key(event::KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT));
    let details = app.details.as_ref().unwrap();
    assert_eq!(details.key, "worker:t1");
    assert_eq!(details.content_tab(), Some(WorkbenchTab::Results));
    assert_eq!(details.query, "saved 👩‍💻 é");
    assert_eq!(details.scroll, 7);
}

#[test]
fn merged_canvas_message_scroll_is_modal_and_conversation_scoped() {
    let mut app = test_app();
    app.conversation = "merge-session".into();
    app.set_focus(Focus::Graph);
    app.view.messages.push(view::MessageEntry {
        id: "long".into(),
        sent_at: None,
        sender: "user".into(),
        target: ORCHESTRATOR.into(),
        text: format!(
            "\x1b]52;c;hidden\x07# Heading\n{}\nCOMPLETE TAIL",
            "row 👩‍💻 é\n".repeat(80)
        ),
        state: "sent".into(),
        kind: "message".into(),
    });
    let mut terminal = Terminal::new(TestBackend::new(160, 40)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(
        app.inspector_messages
            .matches(&app.conversation, ORCHESTRATOR)
    );
    let end = event::KeyEvent::new(KeyCode::End, KeyModifiers::SHIFT);
    app.show_help(Help::Graph);
    app.key(end);
    app.key(key_press(KeyCode::Esc));
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(!terminal_text(&terminal).contains("COMPLETE TAIL"));
    let offset = app.offset;
    app.key(end);
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let text = terminal_text(&terminal);
    assert!(text.contains("COMPLETE TAIL"), "{text}");
    assert!(!text.contains("hidden"));
    assert_eq!(app.offset, offset);
    let original = app.conversation.clone();
    let replayed_messages = app.view.messages.clone();
    app.input = InputBuffer::with_text("draft 👩‍💻 é");
    app.open("another-merge-session".into());
    app.input = InputBuffer::with_text("other draft");
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(!terminal_text(&terminal).contains("COMPLETE TAIL"));
    app.open(original);
    terminal.draw(|frame| app.draw(frame)).unwrap();
    // The poller replays conversation data; snapshots retain UI state only.
    app.view.messages = replayed_messages;
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(terminal_text(&terminal).contains("COMPLETE TAIL"));
    assert_eq!(app.input.text, "draft 👩‍💻 é");
    for (width, height) in [(1, 1), (11, 4), (12, 5)] {
        let mut small = Terminal::new(TestBackend::new(width, height)).unwrap();
        small.draw(|frame| app.draw(frame)).unwrap();
    }
}

#[test]
fn reviewer_summary_reveals_the_selected_check_without_changing_typed_tab() {
    let mut app = test_app();
    app.view.tasks.push(view::TaskRow {
        id: "t1".into(),
        ..Default::default()
    });
    app.open_details("reviewer:t1".into());
    app.workbench_select_tab(WorkbenchTab::Summary);
    app.details.as_mut().unwrap().load_task(serde_json::json!({
        "spec": {"id":"t1", "title":"Task", "kind":"analysis", "change_kind":"none", "goal":"Goal", "acceptance_criteria": (0..40).map(|i| serde_json::json!({"id":format!("C{i}"), "text":format!("criterion{i}")})).collect::<Vec<_>>()}
    }));
    let mut terminal = Terminal::new(TestBackend::new(160, 24)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    for _ in 0..20 {
        app.key(key_press(KeyCode::Down));
    }
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let details = app.details.as_ref().unwrap();
    assert_eq!(details.selected_check().as_deref(), Some("C19"));
    assert!(details.scroll > 0);
    assert!(!details.reveal_check);
    assert_eq!(details.content_tab(), Some(WorkbenchTab::Summary));
    assert_eq!(app.navigation.presentation().tab, WorkbenchTab::Summary);
    assert!(terminal_text(&terminal).contains("criterion19"));
}

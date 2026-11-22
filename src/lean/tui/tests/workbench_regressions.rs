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

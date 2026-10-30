use super::catalog::COMMANDS;

use super::*;

#[test]
fn typing_narrows_commands_by_every_word() {
    let view = View::default();
    let mut picker = Picker::new(Kind::Commands);
    assert_eq!(picker.items(&view).len(), COMMANDS.len());
    picker.query = "plan res".into();
    let items = picker.items(&view);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].1, Action::Run(Command::Resume));
}

#[test]
fn the_conversation_is_searched_newest_first() {
    let mut view = View::default();
    for (seq, text) in [(1, "plan importu"), (2, "inne"), (3, "Import gotowy")] {
        view.apply(&serde_json::json!({"seq": seq, "kind": "message",
                "payload": {"author": "orchestrator", "text": text}}));
    }
    let mut picker = Picker::new(Kind::History);
    picker.query = "import".into();
    let shown: Vec<Action> = picker
        .items(&view)
        .into_iter()
        .map(|(_, action)| action)
        .collect();
    assert_eq!(shown.len(), 2);
    let Action::Show(newest) = shown[0] else {
        panic!("{shown:?}")
    };
    assert!(view.transcript[newest].text.contains("Import gotowy"));
}

#[test]
fn agents_are_found_by_role_or_task_title() {
    let mut view = View::default();
    view.apply(
        &serde_json::json!({"seq": 1, "kind": "plan_proposed", "payload": {
            "plan": "p",
            "tasks": [{"id": "t1", "title": "Parser"}, {"id": "t2", "title": "Docs"}]}}),
    );
    let mut picker = Picker::new(Kind::Agents);
    picker.query = "pars veri".into();
    let items = picker.items(&view);
    assert_eq!(
        items,
        [(
            "t1 task · Parser · verifier".to_owned(),
            Action::Select("verifier:t1".into())
        )]
    );
}

#[test]
fn picker_cursor_bounded_and_display_width_aware() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let view = View::default();
    let mut picker = Picker::new(Kind::Commands);

    // 1. Standard ASCII query
    picker.query = "pla".into();
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            picker.draw(frame, &view);
        })
        .unwrap();
    // Box width is min(80-4, 64) = 64. Left x is (80-64)/2 = 8.
    // Inner cursor: prefix "> " (2 cols) + "pla" (3 cols) = 5 cols from box_area.x + 1.
    // So cursor_x should be 8 + 1 + 5 = 14.
    let (cursor_x, cursor_y) = terminal.get_cursor_position().unwrap().into();
    assert_eq!(cursor_x, 14);
    assert_eq!(cursor_y, 5);

    // 2. Double-width CJK character takes 2 visual columns
    picker.query = "漢".into();
    terminal
        .draw(|frame| {
            picker.draw(frame, &view);
        })
        .unwrap();
    let (cursor_x, _) = terminal.get_cursor_position().unwrap().into();
    // "> " (2) + "漢" (2) = 4 cols -> 8 + 1 + 4 = 13
    assert_eq!(cursor_x, 13);

    // 3. Overflowing long query: cursor must be bounded within box interior!
    picker.query = "a".repeat(200);
    terminal
        .draw(|frame| {
            picker.draw(frame, &view);
        })
        .unwrap();
    let (cursor_x, _) = terminal.get_cursor_position().unwrap().into();
    // Inner width is 64 - 2 = 62. Max col is 61.
    // Bounded cursor_x should be 8 + 1 + 61 = 70.
    // Box right is 8 + 64 = 72, interior right is 71. Cursor <= 70 is safely inside border!
    assert_eq!(cursor_x, 70);
    assert!(cursor_x < 8 + 64 - 1);
}

#[test]
fn picker_and_help_render_deterministically_on_narrow_and_tiny_terminals() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let view = View::default();
    let mut picker = Picker::new(Kind::Commands);

    // Narrow terminal: 40x12
    let backend = TestBackend::new(40, 12);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            picker.draw(frame, &view);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    assert!(buffer.area.width == 40 && buffer.area.height == 12);

    // Tiny terminal: 10x4
    let backend = TestBackend::new(10, 4);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            picker.draw(frame, &view);
        })
        .unwrap();

    // Extreme tiny terminal: 2x2 (should not panic)
    let backend = TestBackend::new(2, 2);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            picker.draw(frame, &view);
            Help::Chat.draw(frame);
        })
        .unwrap();
}

#[test]
fn picker_empty_state_and_narrow_title() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let view = View::default();
    let mut picker = Picker::new(Kind::History);
    assert!(picker.items(&view).is_empty());

    let backend = TestBackend::new(60, 15);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            picker.draw(frame, &view);
        })
        .unwrap();

    let buffer = terminal.backend().buffer();
    let rendered: String = buffer.content.iter().map(|c| c.symbol()).collect();
    // Empty state for history
    assert!(
        rendered.contains("No entries in conversation history") || rendered.contains("No entries")
    );
}

#[test]
fn picker_at_60x12_keeps_selected_item_visible_when_items_len_ge_10() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let view = View::default();
    let mut picker = Picker::new(Kind::Commands);
    let items = picker.items(&view);
    assert!(items.len() >= 10, "Commands should have >= 10 items");

    // Select the last item (e.g. index 11)
    picker.selected = items.len() - 1;

    let backend = TestBackend::new(60, 12);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            picker.draw(frame, &view);
        })
        .unwrap();

    let buffer = terminal.backend().buffer();
    // Verify that the selected item is rendered with REVERSED modifier
    let mut found_reversed_selected = false;
    let last_item_label = &items[picker.selected].0;
    let last_word = last_item_label.split_whitespace().next().unwrap();

    for y in 0..12 {
        let row_chars: String = (0..60).map(|x| buffer[(x, y)].symbol()).collect();
        if row_chars.contains(last_word) {
            for x in 0..60 {
                let cell = &buffer[(x, y)];
                if cell.modifier.contains(Modifier::REVERSED) {
                    found_reversed_selected = true;
                    break;
                }
            }
        }
    }
    assert!(
        found_reversed_selected,
        "Selected item at index {} must be visible with REVERSED modifier at 60x12",
        picker.selected
    );
}

#[test]
fn help_at_60x12_renders_all_controls_and_documents_shortcuts() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let backend = TestBackend::new(60, 12);
    let mut terminal = Terminal::new(backend).unwrap();

    // 1. Initial page (scroll = 0): verify first set of shortcuts and full descriptions
    terminal
        .draw(|frame| {
            Help::Chat.draw(frame);
        })
        .unwrap();

    let buffer = terminal.backend().buffer();
    let rendered: String = buffer.content.iter().map(|c| c.symbol()).collect();

    // Check shortcuts and their complete descriptions
    assert!(rendered.contains("send message to orchestrator"));
    assert!(rendered.contains("newline in draft"));
    assert!(rendered.contains("line in draft"));
    assert!(rendered.contains("scroll conversation"));
    assert!(rendered.contains("accept/resume"));
    assert!(rendered.contains("delete to start / end"));

    // Verify every scroll position so new help entries cannot make an
    // earlier shortcut disappear from the coverage of the final page.
    let mut rendered2 = String::new();
    for scroll in 0..=Help::Chat.max_scroll(Rect::new(0, 0, 60, 12)) {
        terminal
            .draw(|frame| Help::Chat.draw_scroll(frame, scroll))
            .unwrap();
        rendered2.extend(
            terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol()),
        );
    }

    assert!(rendered2.contains("cancel running work"));
    assert!(rendered2.contains("pause new steps / continue"));
    assert!(rendered2.contains("autonomous / supervised mode"));
    assert!(rendered2.contains("always sent as chat"));
    assert!(rendered2.contains("search conversation"));
    assert!(rendered2.contains("command palette"));
    assert!(rendered2.contains("back/clear draft"));

    // 3. Graph help at 60x12: verify long filter relation description is fully accessible without clipping
    terminal
        .draw(|frame| {
            Help::Graph.draw_scroll(frame, 2);
        })
        .unwrap();

    let buffer_graph = terminal.backend().buffer();
    let rendered_graph: String = buffer_graph.content.iter().map(|c| c.symbol()).collect();
    assert!(rendered_graph.contains("show / hide"));
    assert!(rendered_graph.contains("dependencies"));
    assert!(rendered_graph.contains("verification"));
    assert!(rendered_graph.contains("messages"));
    assert!(rendered_graph.contains("supervision"));
}

use super::catalog::COMMANDS;

use super::*;

#[test]
fn typing_narrows_commands_by_every_word() {
    let view = View::default();
    let mut picker = Picker::new(Kind::Commands);
    picker.query = "approvals orchestrator".into();
    let items = picker.items(&view);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].1, Action::Run(Command::ApprovalsAutonomous));
}

#[test]
fn command_palette_keeps_only_actions_without_a_direct_route() {
    let view = View::default();
    let picker = Picker::new(Kind::Commands);
    let items = picker.items(&view);
    let actions: Vec<_> = items.into_iter().map(|(_, action)| action).collect();

    assert_eq!(
        actions,
        [
            Action::Run(Command::PendingApprovals),
            Action::Run(Command::ApprovalsSupervised),
            Action::Run(Command::ApprovalsAutonomous),
            Action::Run(Command::ConversationDetails),
            Action::Run(Command::TakeControl),
            Action::Run(Command::Projects),
            Action::Run(Command::SwitchConversation),
            Action::Run(Command::NewConversation),
        ]
    );
    assert_eq!(COMMANDS.len(), 8);
    assert!(
        Help::Chat
            .lines_for_width(100)
            .iter()
            .any(|line| line.contains("Ctrl+T") && line.contains("session Workbench"))
    );
    assert!(
        Help::Graph
            .lines_for_width(100)
            .iter()
            .any(|line| line.contains("Ctrl+T") && line.contains("session Workbench"))
    );
}

#[test]
fn small_change_picker_labels_share_the_options_summary() {
    let view = View::default();
    let expected = "options when enabled; static analysis + tests";

    let mut commands = Picker::new(Kind::Commands);
    commands.query = "small-change".into();
    assert_eq!(
        commands.items(&view)[0].0,
        format!("Rigor: small-change · {expected}")
    );

    let rigor = Picker::new(Kind::Rigor);
    assert!(
        rigor.items(&view).iter().any(|(label, _)| {
            label == &format!("small-change · light ceremony · {expected}")
        })
    );

    let slash_commands = Picker::new(Kind::SlashCommands);
    assert!(
        slash_commands
            .items(&view)
            .iter()
            .any(|(label, _)| { label == &format!("/small-change · light rigor ({expected})") })
    );
}

#[test]
fn code_review_is_visible_and_selected_in_every_rigor_surface() {
    use crate::lean::tasks::Team;
    let mut view = View::default();
    let action = Action::Run(Command::SelectTeam(Team::CodeReview));

    let rigor = Picker::new(Kind::Rigor).items(&view);
    assert!(rigor.iter().any(|(label, item)| {
        label == "code-review · review code changes · no generated quality gates" && *item == action
    }));

    let mut commands = Picker::new(Kind::Commands);
    commands.query = "code-review".into();
    assert!(commands.items(&view).iter().any(|(label, item)| {
        label == "Rigor: code-review · code review without generated quality gates"
            && *item == action
    }));

    let mut slash = Picker::new(Kind::SlashCommands);
    slash.query = "code-review".into();
    assert!(slash.items(&view).iter().any(|(label, item)| {
        label == "/code-review · code review without generated quality gates" && *item == action
    }));

    view.apply(
        &serde_json::json!({"seq": 1, "kind": "team_selected", "payload": {
            "team": "code-review", "reason": "Review-only run", "origin": "user"
        }}),
    );
    assert_eq!(view.rigor_label(), "code-review");
    let rigor = Picker::new(Kind::Rigor).items(&view);
    assert!(rigor.iter().any(|(label, item)| {
        label == "code-review (current) · review code changes · no generated quality gates"
            && *item == action
    }));
}

#[test]
fn no_quality_gates_is_visible_and_selected_in_every_rigor_surface() {
    use crate::lean::tasks::Team;
    let mut view = View::default();
    let action = Action::Run(Command::SelectTeam(Team::NoQualityGates));
    assert!(Picker::new(Kind::Rigor).items(&view).iter().any(|(label, item)| {
        label == "no-quality-gates · requested work · ordinary acceptance + final integration only"
            && *item == action
    }));
    let mut commands = Picker::new(Kind::Commands);
    commands.query = "no-quality-gates".into();
    assert!(commands.items(&view).iter().any(|(label, item)| {
        label == "Rigor: no-quality-gates · requested work + final integration only"
            && *item == action
    }));
    let mut slash = Picker::new(Kind::SlashCommands);
    slash.query = "no-quality-gates".into();
    assert!(slash.items(&view).iter().any(|(label, item)| {
        label == "/no-quality-gates · requested work + final integration only" && *item == action
    }));
    view.apply(
        &serde_json::json!({"seq": 1, "kind": "team_selected", "payload": {
            "team":"no-quality-gates", "reason":"No gates", "origin":"user"
        }}),
    );
    assert_eq!(view.rigor_label(), "no-quality-gates");
}

#[test]
fn direct_and_all_eight_rigors_are_selectable_from_composer_and_palette() {
    use crate::lean::tasks::Team;

    let view = View::default();
    let rigors = [
        ("small-change", Team::SmallChange),
        ("feature", Team::Feature),
        ("refactoring", Team::Refactoring),
        ("security-fix", Team::SecurityFix),
        ("code-review", Team::CodeReview),
        ("no-quality-gates", Team::NoQualityGates),
        ("research", Team::Research),
        ("document", Team::Document),
    ];
    for (name, team) in rigors {
        let action = Action::Run(Command::SelectTeam(team));
        for kind in [Kind::SlashCommands, Kind::Commands] {
            let mut picker = Picker::new(kind);
            picker.query = name.into();
            assert!(
                picker
                    .items(&view)
                    .iter()
                    .any(|(_, candidate)| *candidate == action),
                "{name} is available from {kind:?}"
            );
        }
    }

    let direct = Action::Run(Command::DirectExecute);
    let mut slash = Picker::new(Kind::SlashCommands);
    slash.query = "direct".into();
    assert!(
        slash
            .items(&view)
            .iter()
            .any(|(_, action)| *action == direct)
    );
    let mut palette = Picker::new(Kind::Commands);
    palette.query = "direct".into();
    assert!(
        palette
            .items(&view)
            .iter()
            .any(|(_, action)| *action == direct)
    );
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
            "tasks": [{"change_kind":"none","id": "t1", "title": "Parser", "reviewer":{"summary":"Review parser"}}, {"change_kind":"none","id": "t2", "title": "Docs"}]}}),
    );
    let mut picker = Picker::new(Kind::Agents);
    picker.query = "pars revi".into();
    let items = picker.items(&view);
    assert_eq!(
        items,
        [(
            "t1 task · Parser · reviewer".to_owned(),
            Action::Select("reviewer:t1".into())
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
    // The list content is capped at 72 columns; the surrounding box adds two.
    // Inner cursor follows "> " plus the three query columns.
    let (cursor_x, cursor_y) = terminal.get_cursor_position().unwrap().into();
    assert_eq!(cursor_x, 9);
    let query_row: String = (0..80)
        .map(|x| terminal.backend().buffer()[(x, cursor_y)].symbol())
        .collect();
    assert!(query_row.contains("> pla"));

    // 2. Double-width CJK character takes 2 visual columns
    picker.query = "漢".into();
    terminal
        .draw(|frame| {
            picker.draw(frame, &view);
        })
        .unwrap();
    let (cursor_x, _) = terminal.get_cursor_position().unwrap().into();
    // "> " (2) + "漢" (2) = 4 display columns.
    assert_eq!(cursor_x, 8);

    // 3. Overflowing long query: cursor must be bounded within box interior!
    picker.query = "a".repeat(200);
    terminal
        .draw(|frame| {
            picker.draw(frame, &view);
        })
        .unwrap();
    let (cursor_x, _) = terminal.get_cursor_position().unwrap().into();
    // The 72-column inner query keeps the cursor inside the box.
    assert_eq!(cursor_x, 75);
    assert!(cursor_x < 3 + 74 - 1);
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
fn picker_at_60x12_keeps_selected_item_visible_with_compact_command_list() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let view = View::default();
    let mut picker = Picker::new(Kind::Commands);
    let items = picker.items(&view);
    assert_eq!(
        items.len(),
        8,
        "All eight navigation commands remain available"
    );

    // Select the last item.
    picker.selected = items.len() - 1;

    let backend = TestBackend::new(60, 12);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            picker.draw(frame, &view);
        })
        .unwrap();

    let buffer = terminal.backend().buffer();
    // Review that the selected item is rendered with REVERSED modifier
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
fn picker_layout_uses_a_72_column_scrollable_viewport_without_item_cap() {
    let area = ratatui::layout::Rect::new(0, 0, 120, 40);
    let layout = super::picker_layout::PickerLayout::new(area, 1_000, 999, 1).unwrap();
    assert_eq!(layout.inner, 72);
    assert!(layout.visible_rows > 12);
    assert_eq!(layout.start + layout.visible_rows, 1_000);
}

#[test]
fn help_at_60x12_renders_all_controls_and_documents_shortcuts() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let backend = TestBackend::new(60, 12);
    let mut terminal = Terminal::new(backend).unwrap();

    // 1. Initial page (scroll = 0): review first set of shortcuts and full descriptions
    terminal
        .draw(|frame| {
            Help::Chat.draw(frame);
        })
        .unwrap();

    let buffer = terminal.backend().buffer();
    let rendered: String = buffer.content.iter().map(|c| c.symbol()).collect();

    // Check shortcuts and their complete descriptions
    assert!(rendered.contains("send to the displayed recipient"));
    assert!(rendered.contains("newline in draft"));
    assert!(rendered.contains("line in draft"));
    assert!(rendered.contains("scroll conversation"));

    // Review every scroll position so new help entries cannot make an
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

    assert!(rendered2.contains("line start / end"));
    assert!(rendered2.contains("delete to start / end"));
    assert!(rendered2.contains("attach an image from the system clipboard"));
    assert!(rendered2.contains("remove the latest pending image"));
    assert!(rendered2.contains("cancel running work"));
    assert!(rendered2.contains("pause / continue outside editors"));
    assert!(rendered2.contains("always sent as chat"));
    assert!(rendered2.contains("search conversation"));
    assert!(rendered2.contains("command palette"));
    assert!(rendered2.contains("close composer and retain draft"));

    // 3. Graph help at 60x12: review long filter relation description is fully accessible without clipping
    let mut rendered_graph = String::new();
    for scroll in 0..=Help::Graph.max_scroll(Rect::new(0, 0, 60, 12)) {
        terminal
            .draw(|frame| Help::Graph.draw_scroll(frame, scroll))
            .unwrap();
        rendered_graph.extend(
            terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol()),
        );
    }
    assert!(rendered_graph.contains("show / hide"));
    assert!(rendered_graph.contains("dependencies"));
    assert!(rendered_graph.contains("review"));
    assert!(rendered_graph.contains("messages"));
    assert!(rendered_graph.contains("supervision"));
}

#[test]
fn a_wide_help_box_wraps_a_long_description_under_its_column() {
    let lines = Help::Graph.lines_for_width(68);
    assert!(lines.iter().all(|line| line.chars().count() <= 68));
    assert!(!lines.iter().any(|line| line.contains('…')));
    let at = lines
        .iter()
        .position(|line| line.starts_with("u "))
        .unwrap();
    let column = lines[at].find("agent").unwrap();
    let next = &lines[at + 1];
    assert!(next[..column].trim().is_empty() && !next[column..].starts_with(' '));
}

#[test]
fn workbench_help_documents_navigation_and_native_key_ownership() {
    let help = Help::Details.lines_for_width(200).join("\n");
    for entry in [
        "Ctrl+Left/Right",
        "Tab/Shift+Tab",
        "Ctrl+B",
        "Alt+Up/Down",
        "pin / unpin",
        "Alt+Left/Right",
        "Ctrl+]",
        "F1 / F2",
        "Ctrl+N / Ctrl+O",
        "Ctrl+G / Ctrl+L",
        "F3 / F4 / F5",
        "F6 / F7 / F8",
        "Ctrl+Y",
        "Ctrl+C",
    ] {
        assert!(help.contains(entry), "missing {entry}");
    }
    assert!(!help.contains("1–4, Tab/Shift+Tab"));
    assert!(help.contains("every other key goes to CLI"));
}

use super::details::Composer;
use super::view::render_header;
use super::*;
use ratatui::buffer::Buffer;
use ratatui::{Terminal, backend::TestBackend};

#[test]
fn ascii_replaces_box_drawing_marks_and_letters() {
    let mut buffer = Buffer::empty(Rect::new(0, 0, 12, 1));
    buffer.set_string(0, 0, "╭─✓ ąębć 漢", Style::default());
    in_ascii(&mut buffer);
    let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
    assert_eq!(text, "+-+ aebc ?  ");
}

#[test]
fn rgb_falls_back_to_the_nearest_256_color() {
    assert_eq!(nearest_indexed(0, 0, 0), 16);
    assert_eq!(nearest_indexed(255, 255, 255), 231);
    assert_eq!(nearest_indexed(0xFF, 0x00, 0x00), 196);
    // Ayu's near-black background lands on the gray ramp, not the cube.
    assert!((232..=235).contains(&nearest_indexed(0x0B, 0x0E, 0x14)));
    let mut buffer = Buffer::empty(Rect::new(0, 0, 1, 1));
    buffer.set_string(
        0,
        0,
        "a",
        Style::default().fg(Color::Rgb(255, 0, 0)).bg(Color::Red),
    );
    to_indexed_colors(&mut buffer);
    assert_eq!(buffer.content[0].fg, Color::Indexed(196));
    assert_eq!(buffer.content[0].bg, Color::Red);
}

#[test]
fn without_color_keeps_dim_and_reversed_as_modifiers() {
    let mut buffer = Buffer::empty(Rect::new(0, 0, 3, 1));
    buffer.set_string(0, 0, "a", dim());
    buffer.set_string(1, 0, "b", Style::default().fg(Color::Red).bg(Color::Blue));
    buffer.set_string(2, 0, "c", Style::default().add_modifier(Modifier::REVERSED));
    without_color(&mut buffer);
    assert!(
        buffer
            .content
            .iter()
            .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset)
    );
    assert!(buffer.content[0].modifier.contains(Modifier::DIM));
    assert!(!buffer.content[1].modifier.contains(Modifier::DIM));
    assert!(buffer.content[2].modifier.contains(Modifier::REVERSED));
}

#[test]
fn shared_surface_palette_and_panel_geometry_follow_d3_bounds() {
    assert_eq!(theme::ayu::BG, Color::Rgb(0x10, 0x16, 0x19));
    assert_eq!(theme::ayu::PANEL, Color::Rgb(0x19, 0x22, 0x26));
    assert_eq!(theme::ayu::SELECTION, Color::Rgb(0x2A, 0x3C, 0x40));
    assert_eq!(theme::ayu::RAISED, Color::Rgb(0x22, 0x30, 0x35));
    assert_eq!(theme::ayu::FG, Color::Rgb(0xE7, 0xED, 0xEB));
    assert_eq!(theme::ayu::DIM, Color::Rgb(0xA1, 0xB1, 0xB3));
    assert_eq!(theme::ayu::CYAN, Color::Rgb(0xA4, 0xDF, 0xCC));
    assert_eq!(theme::ayu::GOLD, Color::Rgb(0xE9, 0xC5, 0x93));
    assert_eq!(
        panel_area(Rect::new(0, 0, 12, 5), 84),
        Rect::new(0, 0, 12, 5)
    );
    let form = panel_area(Rect::new(0, 0, 120, 40), 84);
    assert_eq!(form.width, 84);
    assert_eq!(form.x, 18);
    assert_eq!(form.y, 4);
    assert_eq!(form.height, 32);
    let list = list_area(Rect::new(0, 0, 120, 40));
    assert_eq!(list.width, 72);
    assert_eq!(list.x, 24);
    assert_eq!(list.y, 4);
}

#[test]
fn ctrl_t_cycles_delivery_while_details_composer_is_editing() {
    let mut app = test_app();
    let mut details = Details::new("worker:t1".into());
    details.editing = true;
    app.details = Some(details);
    let draft_key = app.details.as_ref().unwrap().composer_key();
    app.details
        .as_ref()
        .unwrap()
        .drafts
        .borrow_mut()
        .entry(draft_key)
        .or_default();

    app.key(key_ctrl('t'));

    let details = app.details.as_ref().unwrap();
    let composer = details.drafts.borrow();
    assert_eq!(
        composer.values().next().unwrap().mode,
        crate::lean::store::Delivery::Steer
    );
    assert_eq!(app.navigation.screen, Screen::Graph);
}

#[test]
fn image_file_drop_is_an_attachment_and_removal_preserves_the_text_draft() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let directory = tempfile::tempdir().unwrap();
    let image = directory.path().join("wireframe.png");
    std::fs::write(&image, crate::lean::content::valid_png_fixture()).unwrap();
    let mut app = test_app();
    app.input = InputBuffer::with_text("please inspect");
    app.paste(&image.to_string_lossy());
    assert_eq!(app.input.text, "please inspect");
    assert_eq!(app.image_drafts[&app.conversation], vec![image.clone()]);
    app.key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT));
    assert_eq!(app.input.text, "please inspect");
    assert!(
        app.image_drafts
            .get(&app.conversation)
            .is_none_or(Vec::is_empty)
    );
}

#[test]
fn pasted_prose_ending_in_an_image_filename_remains_chat_text() {
    let mut app = test_app();
    let pasted = "the mockup is in screenshot.png";

    app.paste(pasted);

    assert_eq!(app.input.text, pasted);
    assert!(
        app.image_drafts
            .get(&app.conversation)
            .is_none_or(Vec::is_empty)
    );
}

#[test]
fn multiline_prose_with_an_image_path_remains_chat_text() {
    let directory = tempfile::tempdir().unwrap();
    let image = directory.path().join("wireframe.png");
    std::fs::write(&image, crate::lean::content::valid_png_fixture()).unwrap();
    let pasted = format!("Please review this draft:\n{}", image.display());
    let mut app = test_app();

    app.paste(&pasted);

    assert_eq!(app.input.text, pasted);
    assert!(
        app.image_drafts
            .get(&app.conversation)
            .is_none_or(Vec::is_empty)
    );
}

#[test]
fn invalid_image_drop_reports_error_without_replacing_unsent_text() {
    let directory = tempfile::tempdir().unwrap();
    let image = directory.path().join("broken.png");
    std::fs::write(&image, b"plain text").unwrap();
    let mut app = test_app();
    app.input = InputBuffer::with_text("keep this draft");
    app.paste(&image.to_string_lossy());
    assert_eq!(app.input.text, "keep this draft");
    assert!(
        app.image_drafts
            .get(&app.conversation)
            .is_none_or(Vec::is_empty)
    );
    assert!(
        app.notice
            .as_deref()
            .unwrap()
            .contains("unsupported or invalid image attachment")
    );
}

#[test]
fn missing_image_file_uri_reports_error_without_replacing_unsent_text() {
    let mut app = test_app();
    app.input = InputBuffer::with_text("preserve this draft");

    app.paste("file:///definitely/missing/attachment.png");

    assert_eq!(app.input.text, "preserve this draft");
    assert!(
        app.image_drafts
            .get(&app.conversation)
            .is_none_or(Vec::is_empty)
    );
    assert!(
        app.notice
            .as_deref()
            .unwrap()
            .contains("Image attachment failed")
    );
}

#[test]
fn multiple_dropped_file_uris_attach_as_a_batch() {
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("first image.png");
    let second = directory.path().join("second.png");
    std::fs::write(&first, crate::lean::content::valid_png_fixture()).unwrap();
    std::fs::write(&second, crate::lean::content::valid_png_fixture()).unwrap();
    let first_uri = first.to_string_lossy().replace(' ', "%20");
    let pasted = format!("file://{first_uri}\n{}", second.display());
    let mut app = test_app();
    app.input = InputBuffer::with_text("keep my text");
    app.paste(&pasted);
    assert_eq!(app.input.text, "keep my text");
    assert_eq!(app.image_drafts[&app.conversation], vec![first, second]);
}

#[test]
fn graph_enter_expands_compacts_and_transfers_the_only_expanded_card() {
    let mut app = test_app();
    app.focus = Focus::Graph;
    app.graph_viewport = Some((120, 48));
    app.view.tasks = vec![
        view::TaskRow {
            id: "t1".into(),
            ..Default::default()
        },
        view::TaskRow {
            id: "t2".into(),
            depends_on: vec!["t1".into()],
            ..Default::default()
        },
    ];
    app.selected = "worker:t1".into();

    app.key(key_press(KeyCode::Enter));
    assert_eq!(app.graph_expanded.as_deref(), Some("worker:t1"));
    // The arrows act inside the expanded card: the selection stays on it.
    app.key(key_press(KeyCode::Down));
    app.key(key_press(KeyCode::Right));
    assert_eq!(app.selected, "worker:t1");
    assert_eq!(app.graph_tab, 1);
    app.key(key_press(KeyCode::Left));
    app.key(key_press(KeyCode::Left));
    assert_eq!(app.graph_tab, 2, "tabs wrap around");
    app.key(key_press(KeyCode::Enter));
    assert!(app.graph_expanded.is_none());
    app.key(key_press(KeyCode::Down));
    assert_ne!(app.selected, "worker:t1");
    app.key(key_press(KeyCode::Enter));
    assert_eq!(app.graph_expanded.as_deref(), Some(app.selected.as_str()));
    app.key(key_press(KeyCode::Enter));
    assert!(app.graph_expanded.is_none());
}

#[test]
fn expanded_card_keeps_one_round_across_tabs_and_never_attaches_historical_pty() {
    let state = tempfile::tempdir().unwrap();
    let mut app = test_app();
    app.focus = Focus::Graph;
    app.graph_viewport = Some((100, 28));
    app.graph_expanded = Some("worker:t1".into());
    app.selected = "worker:t1".into();
    app.view.plan = Some("plan".into());
    app.view.tasks = vec![view::TaskRow {
        id: "t1".into(),
        title: "current round two".into(),
        round: 2,
        ..Default::default()
    }];
    let mut details = Details::new("worker:t1".into());
    details.available_rounds = vec![1, 2];
    details.task = Some(serde_json::json!({
        "round": 2,
        "title": "current round two",
        "status": "complete",
        "_rounds": [{"round": 1}, {"round": 2}]
    }));
    app.details = Some(details);

    // The actual graph key path selects a retained round and fetches its snapshot.
    app.key(key_press(KeyCode::Char(',')));
    assert_eq!(app.details.as_ref().unwrap().selected_round, Some(1));
    let details = app.details.as_mut().unwrap();
    details.task = Some(serde_json::json!({
        "round": 1,
        "title": "retained round one details",
        "status": "complete",
        "_rounds": [{"round": 1}, {"round": 2}]
    }));
    details
        .documents
        .insert("result".into(), result_for_test(state.path()));

    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    for (tab, expected) in [
        (0, "retained round one details"),
        (1, "Historical PTY is read-only"),
        (2, "result for retained round one"),
        (3, "No historical diff is retained"),
    ] {
        if tab > 0 {
            app.key(key_press(KeyCode::Char(char::from(b'1' + tab as u8))));
        }
        terminal.draw(|frame| app.draw(frame)).unwrap();
        if tab == 2 {
            // The disk-backed document builds its line index asynchronously.
            for _ in 0..20 {
                terminal.draw(|frame| app.draw(frame)).unwrap();
                let text = terminal_text(&terminal);
                if text.contains(expected) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        assert_eq!(app.details.as_ref().unwrap().selected_round, Some(1));
        assert!(
            terminal_text(&terminal).contains(expected),
            "tab {tab}: {}",
            terminal_text(&terminal)
        );
        assert!(terminal_text(&terminal).contains("Round 1"));
        assert!(terminal_text(&terminal).contains(". next"));
        if tab == 1 {
            assert!(!app.native_focused());
            app.native_send(crate::lean::terminal::Input::Key {
                event: key_press(KeyCode::Char('x')),
            });
            assert!(
                app.native_views.is_empty(),
                "historical PTY must not attach a process"
            );
        }
        if tab == 3 {
            app.key(key_press(KeyCode::Char('c')));
            assert!(
                app.editor.is_none(),
                "a missing historical diff cannot receive a comment"
            );
            assert!(
                app.notice
                    .as_deref()
                    .unwrap()
                    .contains("No historical diff")
            );
        }
    }

    // A new live round does not dislodge the historical selection. Advancing
    // through the newest retained round returns to follow mode.
    app.view.tasks[0].round = 3;
    app.key(key_press(KeyCode::Char('.')));
    assert_eq!(app.details.as_ref().unwrap().selected_round, Some(2));
    assert_eq!(app.details.as_ref().unwrap().selected_round, Some(2));
    app.key(key_press(KeyCode::Char('.')));
    assert_eq!(app.details.as_ref().unwrap().selected_round, None);
}

fn result_for_test(state: &std::path::Path) -> document::Document {
    let mut result = document::Builder::new(state).unwrap();
    result
        .append("", |sink| sink("result for retained round one"))
        .unwrap();
    result.finish().unwrap()
}

fn terminal_text(terminal: &Terminal<TestBackend>) -> String {
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

fn graph_layout_rebuild_count(app: &App) -> usize {
    let cache = &app.graph_layout_cache as *const _ as usize;
    super::GRAPH_LAYOUT_REBUILDS
        .with(|counts| counts.borrow().get(&cache).copied().unwrap_or_default())
}

#[test]
fn clicking_now_focuses_and_scrolls_to_active_agent() {
    let mut app = test_app();
    app.focus = Focus::Chat;
    app.view.tasks = (0..12)
        .map(|index| view::TaskRow {
            id: format!("t{index}"),
            title: format!("Task {index}"),
            status: if index == 11 { "working" } else { "proposed" }.into(),
            ..Default::default()
        })
        .collect();
    let mut terminal = Terminal::new(TestBackend::new(140, 40)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let now = app.now_area.expect("Now panel rendered");
    let initial_offset = app.offset;
    app.mouse(event::MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: now.x + 1,
        row: now.y + 1,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(app.selected, "worker:t11");
    assert_eq!(app.focus, Focus::Graph);

    terminal.draw(|frame| app.draw(frame)).unwrap();
    let scope = app.graph_scope_with_exception();
    let graph = Graph::new_with_viewport(
        &app.view,
        &scope,
        &app.graph_collapsed,
        app.graph_grouping,
        app.graph_viewport,
    );
    let target = graph.node("worker:t11").expect("target card exists");
    assert!(
        target.visible,
        "the clicked target overrides the active filter"
    );
    let (width, height) = app.graph_viewport.unwrap();
    assert_ne!(
        app.offset, initial_offset,
        "the distant target pans into view"
    );
    assert!(target.x >= app.offset.0 && target.x < app.offset.0 + width);
    assert!(target.y >= app.offset.1 && target.y < app.offset.1 + height);
    assert!(!app.sending);
}

#[test]
fn now_click_mapping_contains_only_the_rows_shown_in_the_bar() {
    let mut app = test_app();
    app.view.tasks = (0..12)
        .map(|index| view::TaskRow {
            id: format!("t{index}"),
            status: "working".into(),
            ..Default::default()
        })
        .collect();
    let mut terminal = Terminal::new(TestBackend::new(140, 40)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let now = app.now_area.expect("Now panel rendered");
    assert_eq!(app.now_click_targets.len(), 6);

    app.mouse(event::MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: now.x + 1,
        row: now.y + 6,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(app.selected, "worker:t5");
    assert_eq!(app.focus, Focus::Graph);
}

#[test]
fn clicking_now_keeps_an_already_visible_agent_in_view() {
    let mut app = test_app();
    app.view.tasks = vec![view::TaskRow {
        id: "t1".into(),
        status: "working".into(),
        ..Default::default()
    }];
    app.selected = "worker:t1".into();
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let initial_offset = app.offset;
    let initial_revision = app.view.layout_revision;
    let initial_scope = app.graph_scope_with_exception();
    let initial_rebuild_count = graph_layout_rebuild_count(&app);
    let now = app.now_area.expect("Now panel rendered");
    app.mouse(event::MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: now.x + 1,
        row: now.y + 1,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(app.selected, "worker:t1");
    assert_eq!(app.focus, Focus::Graph);
    assert!(app.graph_visibility_exception.is_none());
    assert_eq!(app.view.layout_revision, initial_revision);
    assert_eq!(app.graph_scope_with_exception(), initial_scope);
    assert_eq!(
        app.now_click_targets[0],
        Some(("worker:t1".into(), true)),
        "the row mapping retains the cached layout visibility"
    );
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert_eq!(app.offset, initial_offset);
    assert_eq!(
        graph_layout_rebuild_count(&app),
        initial_rebuild_count,
        "rendering after the click reuses the layout for the unchanged model and scope"
    );
}

#[test]
fn clicking_now_temporarily_exposes_a_filtered_agent() {
    let mut app = test_app();
    app.view.tasks = (0..12)
        .map(|index| view::TaskRow {
            id: format!("t{index}"),
            status: if index == 0 { "working" } else { "proposed" }.into(),
            ..Default::default()
        })
        .collect();
    app.selected = "worker:t0".into();
    app.graph_scope.query = Some("does-not-match".into());
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let now = app.now_area.expect("Now panel rendered");
    let initial_offset = app.offset;
    app.graph_revealed = Some("worker:t1".into());
    app.focus = Focus::Chat;
    app.mouse(event::MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: now.x + 1,
        row: now.y + 1,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(app.selected, "worker:t0");
    assert_eq!(app.focus, Focus::Graph);
    assert_eq!(
        app.offset, initial_offset,
        "an already visible target stays put"
    );
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert_eq!(
        app.offset.1, 0,
        "the selected target is brought back into view"
    );
    let scope = app.graph_scope_with_exception();
    let graph = Graph::new_with_viewport(
        &app.view,
        &scope,
        &app.graph_collapsed,
        app.graph_grouping,
        app.graph_viewport,
    );
    assert!(graph.node("worker:t0").is_some_and(|node| node.visible));
    assert_eq!(app.graph_scope.query.as_deref(), Some("does-not-match"));
}

#[test]
fn clicking_now_without_an_active_agent_leaves_graph_state_unchanged() {
    let mut app = test_app();
    app.view.tasks = vec![view::TaskRow {
        id: "t1".into(),
        status: "waiting".into(),
        ..Default::default()
    }];
    app.selected = "worker:t1".into();
    app.offset = (3, 4);
    app.focus = Focus::Chat;
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let now = app.now_area.expect("Now panel rendered");
    let graph_state = (app.selected.clone(), app.offset, app.focus);
    app.mouse(event::MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: now.x + 1,
        row: now.y + 1,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!((app.selected.clone(), app.offset, app.focus), graph_state);
    assert!(app.graph_visibility_exception.is_none());
}

#[test]
fn keyboard_graph_filter_change_clears_now_visibility_exception() {
    let mut app = test_app();
    app.graph_visibility_exception = Some("worker:t1".into());
    app.selected = "worker:t1".into();
    app.graph_scope.query = Some("does-not-match".into());
    app.graph_filters = Some(Default::default());

    for _ in 0..4 {
        app.key(key_press(KeyCode::Down));
    }
    app.key(key_press(KeyCode::Enter));

    assert!(app.graph_visibility_exception.is_none());
    assert_eq!(app.graph_scope.query.as_deref(), Some("does-not-match"));
    assert!(app.graph_scope.states.contains(&graph::Health::ALL[0]));
}

#[test]
fn pointer_graph_filter_change_clears_now_visibility_exception() {
    let mut app = test_app();
    app.graph_visibility_exception = Some("worker:t1".into());
    app.selected = "worker:t1".into();
    app.graph_scope.query = Some("does-not-match".into());
    app.graph_filters = Some(Default::default());
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();

    let provider_count = graph::providers(&app.view).len() as u16;
    let row_count = 4 + graph::Health::ALL.len() as u16 + provider_count;
    let wanted = row_count + 9;
    let boxed_y = (24_u16.saturating_sub(wanted.min(24))) / 2;
    let state_row_y = boxed_y + 8;
    app.mouse(event::MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 30,
        row: state_row_y,
        modifiers: KeyModifiers::NONE,
    });

    assert!(app.graph_visibility_exception.is_none());
    assert_eq!(app.graph_scope.query.as_deref(), Some("does-not-match"));
    assert!(app.graph_scope.states.contains(&graph::Health::ALL[0]));
}

#[test]
fn embedded_diff_keeps_graph_tabs_and_selection_navigation_available() {
    let mut app = test_app();
    app.focus = Focus::Graph;
    app.graph_viewport = Some((120, 48));
    app.view.tasks = vec![
        view::TaskRow {
            id: "t1".into(),
            ..Default::default()
        },
        view::TaskRow {
            id: "t2".into(),
            depends_on: vec!["t1".into()],
            ..Default::default()
        },
    ];
    app.selected = "worker:t1".into();
    app.key(key_press(KeyCode::Enter));
    app.key(key_press(KeyCode::Char('d')));
    assert!(app.diff.is_some());
    assert_eq!(app.graph_tab, 3);

    app.key(key_press(KeyCode::Char('1')));
    assert_eq!(
        app.graph_tab, 0,
        "Details tab remains reachable inside Diff"
    );
    app.key(key_press(KeyCode::Char('2')));
    assert_eq!(app.graph_tab, 1, "PTY tab remains reachable inside Diff");
    assert!(
        app.diff.is_none(),
        "the DiffView closes with its tab so it stops taking keys"
    );
    app.key(key_press(KeyCode::Char('3')));
    assert_eq!(
        app.graph_tab, 2,
        "Results tab remains reachable inside Diff"
    );
    app.key(key_press(KeyCode::Char('4')));
    assert_eq!(app.graph_tab, 3, "Diff tab remains selectable");
    assert!(app.diff.is_some(), "the Diff tab reopens the DiffView");

    // Inside the diff, ←/→ still switch tabs and the selection stays put.
    app.key(key_press(KeyCode::Up));
    app.key(key_press(KeyCode::Down));
    assert_eq!(app.selected, "worker:t1");
    assert_eq!(app.graph_tab, 3);
    app.key(key_press(KeyCode::Right));
    assert_eq!(app.graph_tab, 0, "→ wraps from Diff to Details");
    app.key(key_press(KeyCode::Left));
    assert_eq!(
        app.graph_tab, 2,
        "without a branch the card has no Diff tab"
    );
    assert_eq!(app.graph_expanded.as_deref(), Some("worker:t1"));
}

#[test]
fn code_task_card_offers_diff_and_brackets_walk_its_files() {
    let mut app = test_app();
    app.focus = Focus::Graph;
    app.graph_viewport = Some((120, 48));
    app.view.tasks = vec![view::TaskRow {
        id: "t1".into(),
        change_kind: crate::lean::tasks::ChangeKind::Code,
        ..Default::default()
    }];
    app.selected = "worker:t1".into();
    app.key(key_press(KeyCode::Enter));
    app.key(key_press(KeyCode::Left));
    assert_eq!(
        app.graph_tab, 3,
        "a code task shows Diff even before a branch is reported"
    );
    let file = |path: &str| {
        serde_json::json!({
            "path": path, "old_path": null, "status": "modified",
            "added": 1, "removed": 0, "binary": false,
            "lines": [{"kind": "context", "old": 1, "new": 1, "text": "x"}],
        })
    };
    app.diff
        .as_mut()
        .expect("the Diff tab opens the DiffView")
        .load(Ok(
            serde_json::json!({"files": [file("a.rs"), file("b.rs")]}),
        ));
    let target = |app: &App| app.diff.as_ref().and_then(|diff| diff.target());
    assert_eq!(target(&app).as_deref(), Some("diff:t1:a.rs:1"));
    app.key(key_press(KeyCode::Char(']')));
    assert_eq!(target(&app).as_deref(), Some("diff:t1:b.rs:1"));
    app.key(key_press(KeyCode::Char('[')));
    assert_eq!(target(&app).as_deref(), Some("diff:t1:a.rs:1"));
    assert_eq!(app.graph_tab, 3, "file keys keep the Diff tab open");
}

#[test]
fn expanded_card_panels_stay_inside_the_card_when_the_graph_scrolls() {
    let mut app = test_app();
    app.focus = Focus::Graph;
    app.view.tasks = (1..=4)
        .map(|n| view::TaskRow {
            id: format!("t{n}"),
            title: format!("task {n}"),
            depends_on: if n > 1 {
                vec![format!("t{}", n - 1)]
            } else {
                Vec::new()
            },
            ..Default::default()
        })
        .collect();
    app.selected = "worker:t2".into();
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    app.key(key_press(KeyCode::Enter));
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let before = app
        .graph_card_area
        .expect("the expanded card has a content area");
    // Scroll the graph so the card's top leaves the viewport.
    app.offset.1 += 6;
    let scrolled = app.offset.1;
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert_eq!(app.offset.1, scrolled, "the draw kept the scroll");
    let after = app
        .graph_card_area
        .expect("the card's lower part is still visible");
    assert_eq!(
        after.bottom(),
        before.bottom() - 6,
        "the panel's bottom follows the card frame up, not the viewport top"
    );
    assert!(
        after.height < before.height,
        "the panel is clipped with the card"
    );
}

#[test]
fn expanded_task_frame_keeps_one_cell_margin_across_viewport_resizes() {
    let mut app = test_app();
    app.focus = Focus::Graph;
    app.view.tasks = vec![
        view::TaskRow {
            id: "t1".into(),
            ..Default::default()
        },
        view::TaskRow {
            id: "t2".into(),
            depends_on: vec!["t1".into()],
            ..Default::default()
        },
        view::TaskRow {
            id: "t3".into(),
            depends_on: vec!["t1".into()],
            ..Default::default()
        },
    ];
    app.selected = "worker:t3".into();
    app.graph_expanded = Some(app.selected.clone());
    app.graph_revealed = None;
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    let mut previous_frame_origin = None;
    for (width, height) in [(120, 40), (80, 24), (12, 5)] {
        if width == 80 {
            app.view.tasks[2].depends_on = vec!["t2".into()];
            app.generation += 1;
            // A resize can leave an offset for the previous expanded geometry.
            app.offset = (0, 0);
        }
        terminal.backend_mut().resize(width, height);
        terminal.resize(Rect::new(0, 0, width, height)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();

        let viewport = app.graph_area.expect("the full graph view rendered");
        let graph = graph::Graph::new_with_expanded(
            &app.view,
            &app.graph_scope_with_exception(),
            &app.graph_collapsed,
            app.graph_grouping,
            app.graph_viewport,
            app.graph_expanded.clone(),
            app.graph_tab,
        );
        let node = graph.node("worker:t3").expect("expanded worker node");
        let frame = graph
            .frames
            .iter()
            .find(|frame| frame.key == node.group.as_deref().unwrap())
            .unwrap();
        let frame_origin = (frame.x, frame.y);
        if width == 80 {
            assert_ne!(
                frame_origin,
                previous_frame_origin.unwrap(),
                "resized layout must move the selected frame"
            );
        }
        previous_frame_origin = Some(frame_origin);
        let frame_screen = (
            i32::from(viewport.x) + i32::from(frame.x) - i32::from(app.offset.0),
            i32::from(viewport.y) + i32::from(frame.y) - i32::from(app.offset.1),
        );
        let node_screen = (
            i32::from(viewport.x) + i32::from(node.x) - i32::from(app.offset.0),
            i32::from(viewport.y) + i32::from(node.y) - i32::from(app.offset.1),
        );
        let viewport_right = i32::from(viewport.right());
        let viewport_bottom = i32::from(viewport.bottom());

        if width >= 40 && height >= 16 {
            for (label, (x, y), card_width, card_height) in [
                ("frame", frame_screen, frame.width, frame.height),
                ("node", node_screen, node.width, node.height),
            ] {
                assert_eq!(
                    x,
                    i32::from(viewport.x) + 1,
                    "{label} left {width}x{height}"
                );
                assert_eq!(y, i32::from(viewport.y) + 1, "{label} top {width}x{height}");
                assert_eq!(
                    x + i32::from(card_width),
                    viewport_right - 1,
                    "{label} right {width}x{height}"
                );
                assert_eq!(
                    y + i32::from(card_height),
                    viewport_bottom - 1,
                    "{label} bottom {width}x{height}"
                );
            }
            assert_eq!(
                (node.x, node.y, node.width, node.height),
                (frame.x, frame.y, frame.width, frame.height)
            );
        } else {
            let [clipped_card, title, content] =
                graph::expanded_card_areas(viewport, node_screen, (node.width, node.height));
            let frame_bounds =
                graph::expanded_card_areas(viewport, frame_screen, (frame.width, frame.height))[0];
            for rect in [clipped_card, frame_bounds, title, content] {
                if !rect.is_empty() {
                    assert!(rect.x >= viewport.x && rect.y >= viewport.y);
                    assert!(rect.right() <= viewport.right());
                    assert!(rect.bottom() <= viewport.bottom());
                }
            }
        }

        let normal = graph::Graph::new_with_viewport(
            &app.view,
            &app.graph_scope_with_exception(),
            &app.graph_collapsed,
            app.graph_grouping,
            app.graph_viewport,
        );
        let normal_node = normal.node("worker:t2").unwrap();
        let expanded_sibling = graph.node("worker:t2").unwrap();
        assert_eq!(
            (expanded_sibling.width, expanded_sibling.height),
            (normal_node.width, normal_node.height),
            "ordinary card dimensions changed at {width}x{height}"
        );
    }
}

#[test]
fn expanded_task_long_title_keeps_incoming_arrow_after_resize() {
    let mut app = test_app();
    app.focus = Focus::Graph;
    app.view.tasks = (0..5)
        .map(|index| view::TaskRow {
            id: format!("parent-{index}"),
            title: format!("Parent {index}"),
            ..Default::default()
        })
        .chain([view::TaskRow {
            id: "expanded".into(),
            title: "A long task title whose status and round reach the arrow".into(),
            status: "running".into(),
            depends_on: (0..5).map(|index| format!("parent-{index}")).collect(),
            ..Default::default()
        }])
        .collect();
    app.selected = "worker:expanded".into();
    app.graph_expanded = Some(app.selected.clone());
    app.graph_revealed = None;

    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    for (width, height) in [(120, 40), (80, 24)] {
        for (theme, expected_arrow) in [
            (theme::Theme::new(true, false), "▼"),
            (theme::Theme::new(true, true), "v"),
        ] {
            terminal.backend_mut().resize(width, height);
            terminal.resize(Rect::new(0, 0, width, height)).unwrap();
            terminal
                .draw(|frame| app.draw_with_theme(frame, &theme))
                .unwrap();

            let viewport = app.graph_area.expect("graph rendered after resize");
            let graph = graph::Graph::new_with_expanded(
                &app.view,
                &app.graph_scope_with_exception(),
                &app.graph_collapsed,
                app.graph_grouping,
                app.graph_viewport,
                app.graph_expanded.clone(),
                app.graph_tab,
            );
            let node = graph.node("worker:expanded").expect("expanded task node");
            let left = i32::from(viewport.x) + i32::from(node.x) - i32::from(app.offset.0);
            let top = i32::from(viewport.y) + i32::from(node.y) - i32::from(app.offset.1);
            let title_rect =
                graph::expanded_card_areas(viewport, (left, top), (node.width, node.height))[1];
            let routes = graph.edge_routes(app.relations);
            let target_group = node.group.as_deref().unwrap();
            let entry_cells = routes
                .iter()
                .filter(|(_, to, path)| {
                    *to == target_group
                        && path
                            .last()
                            .is_some_and(|(_, y)| y.saturating_add(1) == node.y)
                })
                .filter_map(|(_, _, path)| path.last().map(|(x, _)| (*x, node.y)))
                .collect::<std::collections::BTreeSet<_>>();
            let mut overlaps_title = false;
            assert!(
                !entry_cells.is_empty(),
                "an incoming route reaches the frame"
            );
            for (route_x, route_y) in entry_cells {
                let screen_x = i32::from(viewport.x) + i32::from(route_x) - i32::from(app.offset.0);
                let screen_y = i32::from(viewport.y) + i32::from(route_y) - i32::from(app.offset.1);
                assert!(
                    screen_x >= i32::from(viewport.x) && screen_x < i32::from(viewport.right())
                );
                assert!(
                    screen_y >= i32::from(viewport.y) && screen_y < i32::from(viewport.bottom())
                );
                let overlaps = screen_y == i32::from(title_rect.y)
                    && screen_x >= i32::from(title_rect.x)
                    && screen_x < i32::from(title_rect.right());
                overlaps_title |= overlaps;
                assert!(
                    overlaps,
                    "fixture route endpoint lies in title at {width}x{height}"
                );
                let full_heading = format!(
                    "{} · running · round unavailable",
                    app.view.tasks.last().unwrap().title
                );
                assert!(
                    unicode_width::UnicodeWidthStr::width(full_heading.as_str())
                        > (screen_x - i32::from(title_rect.x)) as usize,
                    "heading text reaches the route endpoint at {width}x{height}"
                );
                let cell = terminal
                    .backend()
                    .buffer()
                    .cell((screen_x as u16, screen_y as u16))
                    .unwrap();
                assert_eq!(
                    cell.symbol(),
                    expected_arrow,
                    "the incoming arrow remains visible at {width}x{height}"
                );
                let (_, _, expected_style) = graph
                    .expanded_entry_arrows()
                    .into_iter()
                    .find(|(x, y, _)| *x == route_x && *y == route_y)
                    .expect("entry arrow style");
                assert_eq!(cell.fg, expected_style.fg.unwrap_or(Color::Reset));
                assert_eq!(
                    cell.bg,
                    expected_style
                        .bg
                        .or(theme.panel().bg)
                        .unwrap_or(Color::Reset)
                );
                assert_eq!(cell.modifier, expected_style.add_modifier);
                if theme.ascii {
                    assert_ne!(cell.symbol(), "▼", "ASCII arrow must not be Unicode");
                }
            }
            let rendered = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(overlaps_title, "long task heading overlaps route endpoint");
            assert!(
                rendered.contains("round unavailable"),
                "round label remains visible"
            );
        }
    }

    terminal.backend_mut().resize(12, 5);
    terminal.resize(Rect::new(0, 0, 12, 5)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(app.graph_area.is_some(), "minimum canvas still renders");
    assert_eq!(terminal.backend().buffer().area, Rect::new(0, 0, 12, 5));
}

#[test]
fn expanded_standalone_agent_keeps_one_cell_canvas_margin_across_resizes() {
    let mut app = test_app();
    app.focus = Focus::Graph;
    app.view
        .agents
        .insert(view::ORCHESTRATOR.into(), view::Agent::default());
    app.selected = view::ORCHESTRATOR.into();
    app.graph_expanded = Some(app.selected.clone());
    app.graph_revealed = None;
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();

    for (width, height) in [(120, 40), (80, 24), (12, 5)] {
        terminal.backend_mut().resize(width, height);
        terminal.resize(Rect::new(0, 0, width, height)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let viewport = app
            .graph_area
            .expect("graph rendered at minimum dimensions");
        let graph = graph::Graph::new_with_expanded(
            &app.view,
            &app.graph_scope_with_exception(),
            &app.graph_collapsed,
            app.graph_grouping,
            app.graph_viewport,
            app.graph_expanded.clone(),
            app.graph_tab,
        );
        let node = graph
            .node(view::ORCHESTRATOR)
            .expect("expanded standalone agent");
        assert!(node.group.is_none());
        let screen = (
            i32::from(viewport.x) + i32::from(node.x) - i32::from(app.offset.0),
            i32::from(viewport.y) + i32::from(node.y) - i32::from(app.offset.1),
        );
        if width >= 40 && height >= 16 {
            assert_eq!(screen.0, i32::from(viewport.x) + 1, "{width}x{height} left");
            assert_eq!(screen.1, i32::from(viewport.y) + 1, "{width}x{height} top");
            assert_eq!(node.width + 2, app.graph_viewport.unwrap().0);
            assert_eq!(node.height + 2, app.graph_viewport.unwrap().1);
        } else {
            let [card, title, content] =
                graph::expanded_card_areas(viewport, screen, (node.width, node.height));
            for rect in [card, title, content] {
                if !rect.is_empty() {
                    assert!(rect.x >= viewport.x && rect.y >= viewport.y);
                    assert!(rect.right() <= viewport.right());
                    assert!(rect.bottom() <= viewport.bottom());
                }
            }
        }
    }
}

#[test]
fn wheel_over_an_expanded_card_scrolls_its_tab_not_the_graph() {
    let mut app = test_app();
    app.focus = Focus::Graph;
    app.view.tasks = (1..=4)
        .map(|n| view::TaskRow {
            id: format!("t{n}"),
            title: format!("task {n}"),
            depends_on: if n > 1 {
                vec![format!("t{}", n - 1)]
            } else {
                Vec::new()
            },
            ..Default::default()
        })
        .collect();
    app.selected = "worker:t2".into();
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    app.key(key_press(KeyCode::Enter));
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let area = app
        .graph_card_area
        .expect("the expanded card has a content area");
    let offset = app.offset;
    let wheel = |kind| event::MouseEvent {
        kind,
        column: area.x + area.width / 2,
        row: area.y + area.height / 2,
        modifiers: KeyModifiers::NONE,
    };
    app.mouse(wheel(event::MouseEventKind::ScrollDown));
    app.mouse(wheel(event::MouseEventKind::ScrollDown));
    assert_eq!(
        app.offset, offset,
        "the wheel moved the graph under the card"
    );
    assert_eq!(app.details.as_ref().unwrap().dash_scroll, 6);
    app.mouse(wheel(event::MouseEventKind::ScrollUp));
    assert_eq!(app.details.as_ref().unwrap().dash_scroll, 3);
}

#[test]
fn graph_comment_popup_shows_scoped_history_and_keeps_the_element_draft() {
    let mut app = test_app();
    app.focus = Focus::Graph;
    app.selected = "worker:t1".into();
    app.comment_drafts.insert(
        App::comment_key(&app.conversation, app.view.plan.as_deref(), "worker:t1"),
        "draft comment".into(),
    );
    app.view.comments.insert(
        7,
        view::Comment {
            id: 7,
            plan_id: None,
            target: "worker:t1".into(),
            text: "existing element note".into(),
            status: "open".into(),
        },
    );

    app.key(key_press(KeyCode::Char('c')));
    assert_eq!(app.editor.as_ref().unwrap().target, "worker:t1");
    assert_eq!(app.editor.as_ref().unwrap().input.text, "draft comment");

    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(text.contains("existing element note"));
    assert!(text.contains("draft comment"));
}

pub fn test_app() -> App {
    App::test_app()
}

pub fn key_press(code: KeyCode) -> event::KeyEvent {
    event::KeyEvent::new(code, KeyModifiers::NONE)
}

pub fn key_ctrl(ch: char) -> event::KeyEvent {
    event::KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)
}

#[test]
fn global_navigation_shortcuts_route_before_screens_and_keep_editor_ctrl_k() {
    let mut app = test_app();
    app.diff = Some(DiffView::new("t1".into()));
    assert!(app.key_inner(key_ctrl('p')));
    assert_eq!(app.picker.as_ref().unwrap().kind, Kind::Commands);
    app.picker = None;
    assert!(app.key_inner(key_ctrl('f')));
    assert_eq!(app.picker.as_ref().unwrap().kind, Kind::History);

    app.picker = None;
    assert!(app.key_inner(key_ctrl('t')));
    assert_eq!(app.navigation.screen, Screen::Workbench);
    assert_eq!(app.navigation.target, WorkbenchTarget::Session);
    assert!(app.details.is_some());
    app.details = None;
    app.navigation.screen = Screen::Graph;
    app.editor = Some(Editor {
        target: "comment:test".into(),
        input: InputBuffer {
            text: "hello world".into(),
            cursor: 5,
            preferred_col: None,
        },
    });
    assert!(app.key_inner(key_ctrl('k')));
    assert_eq!(app.editor.as_ref().unwrap().input.text, "hello");
    assert_eq!(app.navigation.screen, Screen::Graph);
}

#[test]
fn active_picker_owns_global_shortcuts_and_escape_before_zoom() {
    let mut app = test_app();
    app.navigation.screen = Screen::Workbench;
    app.navigation
        .update_presentation(|presentation| presentation.split = true);
    app.navigation.zoom(true);
    app.focus = Focus::Graph;
    app.picker = Some(Picker::new(Kind::History));
    app.picker.as_mut().unwrap().query = "keep query".into();

    for key in [
        key_ctrl('p'),
        key_ctrl('f'),
        key_ctrl('t'),
        key_ctrl('k'),
        key_press(KeyCode::F(2)),
    ] {
        assert!(app.key_inner(key));
        assert_eq!(app.picker.as_ref().unwrap().kind, Kind::History);
        assert_eq!(app.picker.as_ref().unwrap().query, "keep query");
        assert!(app.navigation.presentation().zoomed);
        assert_eq!(app.navigation.screen, Screen::Workbench);
    }
    assert!(app.approval.is_none());
    assert!(app.details.is_none());

    assert!(app.key_inner(key_press(KeyCode::Esc)));
    assert!(app.picker.is_none());
    assert!(app.navigation.presentation().zoomed);
    assert!(app.navigation.presentation().split);
    assert_eq!(app.navigation.screen, Screen::Workbench);
}

#[test]
fn interview_owns_global_navigation_shortcuts_except_its_command_palette() {
    let mut app = test_app();
    let question = serde_json::from_value(serde_json::json!({
        "id": "scope",
        "context": "Choose a scope.",
        "question": "Which scope?",
        "options": [{"label": "Small"}, {"label": "Full"}]
    }))
    .unwrap();
    app.view.workflow_phase = Some("interview".into());
    app.view.interview_question = Some((42, question));
    app.sync_interview();
    app.interview_forms
        .get_mut(&app.conversation)
        .unwrap()
        .paste("preserve this answer");

    assert!(app.key_inner(key_ctrl('t')));
    assert!(app.key_inner(key_ctrl('f')));
    assert!(app.details.is_none());
    assert!(app.picker.is_none());
    assert_eq!(
        app.interview_forms[&app.conversation].input.text,
        "preserve this answer"
    );

    assert!(app.key_inner(key_ctrl('p')));
    assert_eq!(app.picker.as_ref().unwrap().kind, Kind::Commands);
    assert_eq!(
        app.interview_forms[&app.conversation].input.text,
        "preserve this answer"
    );
}

#[test]
fn interview_escape_closes_the_form_before_restoring_zoomed_split() {
    let mut app = test_app();
    app.navigation.screen = Screen::Workbench;
    app.navigation.update_presentation(|p| p.split = true);
    app.navigation.zoom(true);
    let question = serde_json::from_value(serde_json::json!({
        "id": "scope",
        "context": "Choose a scope.",
        "question": "Which scope?",
        "options": [{"label": "Small"}, {"label": "Full"}]
    }))
    .unwrap();
    app.view.workflow_phase = Some("interview".into());
    app.view.interview_question = Some((42, question));
    app.sync_interview();

    assert!(app.key_inner(key_press(KeyCode::Esc)));
    assert!(!app.interview_forms[&app.conversation].visible);
    assert!(app.navigation.presentation().zoomed);
    assert!(app.navigation.presentation().split);
    assert_eq!(app.navigation.screen, Screen::Workbench);

    assert!(app.key_inner(key_press(KeyCode::Esc)));
    assert!(!app.navigation.presentation().zoomed);
    assert!(app.navigation.presentation().split);
    assert_eq!(app.navigation.screen, Screen::Workbench);
}

#[test]
fn ctrl_k_deletes_to_end_in_editing_details_composer() {
    let mut app = test_app();
    let mut details = Details::default();
    details.editing = true;
    details.key = "worker:t1".into();
    details.conversation = app.conversation.clone();
    details.input_width = [40, 40];
    let composer_key = details.composer_key();
    details.drafts.borrow_mut().insert(
        composer_key,
        Composer {
            input: InputBuffer {
                text: "keep delete this".into(),
                cursor: "keep ".len(),
                preferred_col: None,
            },
            ..Composer::default()
        },
    );
    app.details = Some(details);

    assert!(app.key_inner(key_ctrl('k')));

    let details = app.details.as_ref().unwrap();
    let composer = details.drafts.borrow();
    let input = &composer[&details.composer_key()].input;
    assert_eq!(input.text, "keep ");
    assert_eq!(input.cursor, "keep ".len());
    assert!(details.editing);
}

#[test]
fn escape_restores_zoom_split_before_leaving_workbench() {
    let mut app = test_app();
    app.navigation.screen = Screen::Workbench;
    app.navigation.update_presentation(|p| p.split = true);
    app.navigation.zoom(true);
    assert!(app.key_inner(key_press(KeyCode::Esc)));
    assert!(!app.navigation.presentation().zoomed);
    assert!(app.navigation.presentation().split);
    assert_eq!(app.navigation.screen, Screen::Workbench);
    assert!(app.key_inner(key_press(KeyCode::Esc)));
    assert_eq!(app.navigation.screen, Screen::Graph);
}

#[test]
fn escape_closes_details_editor_before_restoring_zoomed_split() {
    let mut app = test_app();
    app.navigation.screen = Screen::Workbench;
    app.navigation.update_presentation(|p| p.split = true);
    app.navigation.zoom(true);
    let mut details = Details::default();
    details.editing = true;
    app.details = Some(details);

    assert!(app.key_inner(key_press(KeyCode::Esc)));
    assert!(!app.details.as_ref().unwrap().editing);
    assert!(app.navigation.presentation().zoomed);
    assert!(app.navigation.presentation().split);

    assert!(app.key_inner(key_press(KeyCode::Esc)));
    assert!(!app.navigation.presentation().zoomed);
    assert!(app.navigation.presentation().split);
    assert_eq!(app.navigation.screen, Screen::Workbench);
}

#[test]
fn escape_closes_comment_editor_before_restoring_zoomed_split() {
    let mut app = test_app();
    app.navigation.screen = Screen::Workbench;
    app.navigation.update_presentation(|p| p.split = true);
    app.navigation.zoom(true);
    app.editor = Some(Editor {
        target: "worker:t1".into(),
        input: InputBuffer::with_text("unsent comment"),
    });

    assert!(app.key_inner(key_press(KeyCode::Esc)));

    assert!(app.editor.is_none());
    assert_eq!(app.get_comment_draft("worker:t1").text, "unsent comment");
    assert!(app.navigation.presentation().zoomed);
    assert!(app.navigation.presentation().split);
}

#[test]
fn escape_closes_plan_view_before_restoring_zoomed_split() {
    let mut app = test_app();
    app.navigation.screen = Screen::Workbench;
    app.navigation.update_presentation(|p| p.split = true);
    app.navigation.zoom(true);
    app.plan_view = Some(PlanView::new("p1".into()));

    assert!(app.key_inner(key_press(KeyCode::Esc)));

    assert!(app.plan_view.is_none());
    assert!(app.navigation.presentation().zoomed);
    assert!(app.navigation.presentation().split);
}

#[test]
fn f2_reopens_pending_decision_cards() {
    let mut app = test_app();
    app.view.approvals.push(view::PendingApproval {
        request: "r1".into(),
        role: "worker:t1".into(),
        provider: "codex".into(),
        summary: "Allow the requested action?".into(),
        orchestrator: false,
        options: vec![("allow".into(), "Allow".into(), "allow".into())],
    });
    app.approval_hidden.insert("r1".into());
    assert!(app.key_inner(key_press(KeyCode::F(2))));
    assert!(app.approval.is_some());
    assert!(app.approval_hidden.is_empty());
}

#[test]
fn ctrl_m_opens_workflow_rigor_picker_from_chat_and_graph() {
    let mut app = test_app();
    app.input = InputBuffer::with_text("draft stays here");
    assert!(app.key(key_ctrl('m')));
    assert!(matches!(
        app.picker.as_ref().map(|picker| picker.kind),
        Some(Kind::Rigor)
    ));
    assert_eq!(app.input.text, "draft stays here");

    app.picker = None;
    app.focus = Focus::Graph;
    assert!(app.key(key_ctrl('m')));
    assert!(matches!(
        app.picker.as_ref().map(|picker| picker.kind),
        Some(Kind::Rigor)
    ));
    assert!(app.graph_expanded.is_none());
}

#[test]
fn ctrl_enter_opens_workflow_rigor_picker_for_terminals_that_encode_ctrl_m_as_enter() {
    let mut app = test_app();
    app.focus = Focus::Graph;
    let key = event::KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL);

    assert!(app.key(key));
    assert!(matches!(
        app.picker.as_ref().map(|picker| picker.kind),
        Some(Kind::Rigor)
    ));
    assert!(app.graph_expanded.is_none());
}

#[test]
fn help_and_picker_keep_ownership_when_routing_keys() {
    let mut app = test_app();
    app.input = InputBuffer::with_text("chat draft 👩‍💻");
    app.picker = Some(Picker::new(Kind::History));
    app.show_help(Help::Picker);

    assert!(app.key(key_press(KeyCode::Char('x'))));
    assert_eq!(app.picker.as_ref().unwrap().query, "");
    assert_eq!(app.input.text, "chat draft 👩‍💻");

    assert!(app.key(key_press(KeyCode::Esc)));
    assert!(app.help.is_none());
    assert!(app.picker.is_some());
    assert!(app.key(key_press(KeyCode::Char('x'))));
    assert_eq!(app.picker.as_ref().unwrap().query, "x");
    assert_eq!(app.input.text, "chat draft 👩‍💻");
}

#[test]
fn conversation_delete_requires_confirmation_and_explains_current_session_lock() {
    let mut app = test_app();
    app.conversation = "current-session".into();
    let mut picker = Picker::new(Kind::Conversations);
    picker
        .conversations
        .push(crate::lean::tui::palette::ConversationItem {
            id: "current-session".into(),
            project: app.project.clone(),
            title: "Current work".into(),
        });
    app.picker = Some(picker);
    assert!(app.key(key_press(KeyCode::Char('x'))));
    assert!(app.picker.as_ref().unwrap().confirm_delete);
    assert!(app.key(key_press(KeyCode::Char('y'))));
    assert!(!app.picker.as_ref().unwrap().confirm_delete);
    assert!(app.notice.as_deref().unwrap().contains("Stop this session"));
}

#[test]
fn native_escape_accepts_legacy_and_enhanced_ctrl_bracket() {
    for ch in [']', '5'] {
        let mut app = test_app();
        app.input = InputBuffer::with_text("Unsent draft 👩‍💻");
        let mut details = Details::new("worker:t1".into());
        details.native_focus = true;
        app.details = Some(details);
        assert!(app.native_focused());

        app.key(key_ctrl(ch));

        assert!(!app.native_focused());
        assert!(app.details.is_some());
        assert_eq!(app.input.text, "Unsent draft 👩‍💻");
    }
}

#[test]
fn ac_key_01_unconditional_pageup_transcript_scroll() {
    let mut app = test_app();
    app.input = InputBuffer::with_text("multiline\ndraft\nmessage");
    app.input.cursor = 5;
    app.key(key_press(KeyCode::PageUp));
    assert_eq!(app.scroll, 10);
    assert_eq!(app.input.cursor, 5);
    app.key(key_press(KeyCode::PageDown));
    assert_eq!(app.scroll, 0);
    assert_eq!(app.input.cursor, 5);
}

#[test]
fn ac_key_02_empty_input_history_scroll() {
    let mut app = test_app();
    assert!(app.input.is_empty());
    app.key(key_press(KeyCode::Up));
    assert_eq!(app.scroll, 1);
    app.key(key_press(KeyCode::Down));
    assert_eq!(app.scroll, 0);
}

#[test]
fn chat_history_escape_restores_grapheme_safe_draft_cursor() {
    let mut app = test_app();
    let draft = "e\u{301}👩‍💻 draft";
    let cursor = "e\u{301}👩‍💻".len();
    app.input = InputBuffer::with_text(draft);
    app.input.cursor = cursor;
    app.history = vec!["previous message".into()];

    app.key(key_press(KeyCode::Up));
    assert_eq!(app.input.text, "previous message");
    app.key(key_press(KeyCode::Esc));

    assert_eq!(app.input.text, draft);
    assert_eq!(app.input.cursor, cursor);
    app.input.backspace();
    assert_eq!(app.input.text, "e\u{301} draft");
}

#[test]
fn ac_key_03_multiline_boundary_up() {
    let mut app = test_app();
    app.input = InputBuffer::with_text("line1\nline2");
    app.input.cursor = 2; // on line 0 ('n')
    app.key(key_press(KeyCode::Up));
    // Stays on line 0, does not jump or scroll transcript
    assert_eq!(app.scroll, 0);
    assert_eq!(app.input.cursor, 2);
}

#[test]
fn ac_modal_02_comment_editor_dismissal_preserves_graph_selection() {
    let mut app = test_app();
    app.focus = Focus::Graph;
    app.selected = "reviewer:t1".into();
    app.editor = Some(Editor {
        target: app.selected.clone(),
        input: InputBuffer::with_text("draft comment"),
    });
    assert!(app.key(key_press(KeyCode::Esc)));
    assert!(app.editor.is_none());
    assert_eq!(app.selected, "reviewer:t1");
}

#[test]
fn ac_modal_03_detach_protection_on_non_empty_chat() {
    let mut app = test_app();
    app.input = InputBuffer::with_text("test");
    let open = app.key(key_press(KeyCode::Esc));
    assert!(open);
    assert!(app.input.is_empty());
}

#[test]
fn ac_modal_04_esc_stays_open_and_explicit_quit_detaches() {
    let mut app = test_app();
    assert!(app.input.is_empty());
    assert!(app.key(key_press(KeyCode::Esc)));
    assert!(!app.key(key_press(KeyCode::Char('q'))));
    assert!(!app.key(event::KeyEvent::new(
        KeyCode::Char('c'),
        KeyModifiers::CONTROL
    )));
}

#[test]
fn ac_modal_05_modal_isolated_plan_shortcuts() {
    let mut app = test_app();
    app.view.apply(&serde_json::json!({
        "seq": 1,
        "kind": "plan_proposed",
        "payload": {"plan": "p1", "tasks": [{"change_kind":"none","id": "t1", "title": "A"}]}
    }));
    assert!(app.view.plan_proposed());
    app.editor = Some(Editor {
        target: "orchestrator".into(),
        input: InputBuffer::with_text("draft comment"),
    });
    app.editor.as_mut().unwrap().input.cursor = 7;
    app.key(key_ctrl('a'));
    // Moves cursor to line start in editor, does NOT trigger /accept
    assert_eq!(app.editor.as_ref().unwrap().input.cursor, 0);
}

#[test]
fn draft_preservation_across_panel_switching_and_modals() {
    let mut app = test_app();
    app.input = InputBuffer::with_text("@t2 please investigate");
    app.key(key_press(KeyCode::Tab));
    assert_eq!(app.focus, Focus::Graph);
    assert_eq!(app.input.text, "@t2 please investigate");

    app.key(key_press(KeyCode::Char('c')));
    assert!(app.editor.is_some());
    assert_eq!(app.input.text, "@t2 please investigate");

    app.key(key_press(KeyCode::Esc));
    assert!(app.editor.is_none());
    assert_eq!(app.input.text, "@t2 please investigate");

    app.key(key_press(KeyCode::Tab));
    assert_eq!(app.focus, Focus::Chat);
    assert_eq!(app.input.text, "@t2 please investigate");
}

#[test]
fn comment_targets_from_graph_and_details() {
    let mut app = test_app();
    app.selected = "worker:t2".into();
    app.focus = Focus::Graph;
    app.key(key_press(KeyCode::Char('c')));
    assert_eq!(app.editor.as_ref().unwrap().target, "worker:t2");

    app.editor = None;
    app.details = Some(Details::new("reviewer:t1".into()));
    app.key(key_press(KeyCode::Char('c')));
    assert_eq!(app.editor.as_ref().unwrap().target, "reviewer:t1");
}

#[test]
fn execution_decision_opens_the_question_form_without_discarding_chat() {
    let mut app = test_app();
    app.input = InputBuffer::with_text("Unsent chat draft 👩‍💻");
    app.view.workflow_phase = Some("implementation".into());
    app.view.user_decision = true;
    let question = serde_json::from_value(serde_json::json!({"id":"metadata","context":"Scope decision","question":"Allow metadata?","options":[{"label":"Allow"},{"label":"Reject"}]})).unwrap();
    app.view.interview_question = Some((42, question));
    app.sync_interview();
    assert!(app.interview_visible());
    assert_eq!(app.input.text, "Unsent chat draft 👩‍💻");
    app.view.user_decision = false;
    assert!(!app.interview_visible());
}

#[test]
fn interview_draft_survives_background_failure_and_conversation_switches() {
    let mut app = test_app();
    app.conversation = "interview-a".into();
    app.input = InputBuffer::with_text("Unsent chat draft");
    let question = serde_json::from_value(serde_json::json!({"id":"scope","context":"Scope affects timing.","question":"Which scope?","options":[{"label":"Small"},{"label":"Full"}]})).unwrap();
    app.view.workflow_phase = Some("interview".into());
    app.view.interview_question = Some((42, question));
    app.sync_interview();
    app.interview_forms
        .get_mut("interview-a")
        .unwrap()
        .paste("My own answer 👩‍💻");
    app.help = Some(Help::Chat);
    app.key(event::KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
    app.paste("must stay out of every draft");
    assert_eq!(
        app.interview_forms["interview-a"].input.text,
        "My own answer 👩‍💻"
    );
    app.help = None;
    app.interview_forms.get_mut("interview-a").unwrap().sending = true;
    app.open("interview-b".into());
    app.stop_poller.store(true, Ordering::SeqCst);
    app.apply_update(Update::InterviewResult {
        conversation: "interview-a".into(),
        question: 42,
        result: Err("socket closed".into()),
    });
    assert_eq!(
        app.interview_forms["interview-a"].error.as_deref(),
        Some("socket closed")
    );
    assert!(!app.interview_forms["interview-a"].sending);
    app.open("interview-a".into());
    app.stop_poller.store(true, Ordering::SeqCst);
    assert_eq!(app.input.text, "Unsent chat draft");
    assert_eq!(
        app.interview_forms["interview-a"].input.text,
        "My own answer 👩‍💻"
    );
    app.apply_update(Update::InterviewResult {
        conversation: "interview-a".into(),
        question: 41,
        result: Ok(()),
    });
    assert!(!app.interview_forms["interview-a"].sent);
}

#[test]
fn interview_home_preserves_unsent_answers_without_requesting_approval() {
    let mut app = test_app();
    app.view.workflow_phase = Some("interview".into());
    app.view.requirements_summary = "Goal and criteria".into();
    app.input = InputBuffer::with_text("Correction 👩‍💻");
    app.key(key_ctrl('a'));
    assert_eq!(app.input.text, "Correction 👩‍💻");
    assert_eq!(app.input.cursor, 0);
    assert!(app.notice.is_none());
    assert!(!app.sending);
}

#[test]
fn chat_ctrl_a_plan_accept_vs_readline_home() {
    let mut app = test_app();
    app.view.apply(&serde_json::json!({
        "seq": 1,
        "kind": "plan_proposed",
        "payload": {"plan": "p1", "tasks": [{"change_kind":"none","id": "t1", "title": "A"}]}
    }));

    // Non-empty draft: Ctrl+A acts as Home
    app.input = InputBuffer::with_text("draft message");
    app.input.cursor = 5;
    app.key(key_ctrl('a'));
    assert_eq!(app.input.cursor, 0);
    assert_eq!(app.input.text, "draft message");
}

#[test]
fn paste_routing_respects_help_modal_and_active_overlays() {
    let mut app = test_app();
    app.input = InputBuffer::with_text("draft");

    // When Help is open, paste does NOT touch chat draft
    app.help = Some(Help::Chat);
    app.paste("pasted while help open");
    assert_eq!(app.input.text, "draft");

    app.help = None;

    // When comment editor is open, paste routes to comment editor, not chat
    app.editor = Some(Editor {
        target: "t1".into(),
        input: InputBuffer::with_text("ab"),
    });
    app.editor.as_mut().unwrap().input.cursor = 1;
    app.paste("XYZ");
    assert_eq!(app.editor.as_ref().unwrap().input.text, "aXYZb");
    assert_eq!(app.input.text, "draft");

    app.editor = None;

    // When chat is focused and unobstructed, paste routes to chat at cursor
    app.input.cursor = 0;
    app.paste("new ");
    assert_eq!(app.input.text, "new draft");
}

#[test]
fn comment_drafts_restore_grapheme_cursor_per_conversation() {
    let mut app = test_app();
    app.conversation = "conversation-one".into();
    let mut input = InputBuffer::with_text("e\u{301}👩‍💻");
    input.cursor = 1;
    input.preferred_col = Some(4);
    app.save_comment_draft("task", input);

    app.conversation = "conversation-two".into();
    assert!(app.get_comment_draft("task").is_empty());
    app.conversation = "conversation-one".into();
    let restored = app.get_comment_draft("task");
    assert_eq!(restored.text, "e\u{301}👩‍💻");
    assert_eq!(restored.cursor, "e\u{301}".len());
    assert_eq!(restored.preferred_col, Some(4));
}

#[test]
fn failed_background_comment_keeps_its_grapheme_cursor() {
    let mut app = test_app();
    app.open("conversation-a".into());
    app.sending_comment = true;
    app.comment_request_id = 17;
    app.editor = Some(Editor {
        target: "worker:t1".into(),
        input: InputBuffer::with_text("active draft"),
    });
    app.open("conversation-b".into());

    let mut failed_input = InputBuffer::with_text("👩‍💻 note");
    failed_input.cursor = "👩‍💻".len();
    failed_input.preferred_col = Some(4);
    app.apply_update(Update::CommentResult {
        conversation: "conversation-a".into(),
        plan: None,
        operation_id: crate::id(),
        generation: 1,
        request_id: 17,
        editor: Editor {
            target: "worker:t3".into(),
            input: failed_input,
        },
        result: Err("connection interrupted".into()),
    });

    app.open("conversation-a".into());
    let draft = app.get_comment_draft("worker:t3");
    assert_eq!(draft.text, "👩‍💻 note");
    assert_eq!(draft.cursor, "👩‍💻".len());
    assert_eq!(draft.preferred_col, Some(4));
    assert_eq!(app.editor.as_ref().unwrap().input.text, "active draft");
}

#[test]
fn render_coordination_guards_below_minimum_and_draws_at_minimum() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let mut app = test_app();
    let backend = TestBackend::new(11, 4);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            app.draw(frame);
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let text: String = buffer.content.iter().map(|c| c.symbol()).collect();
    let norm = text.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(norm.contains("Terminal too small (min. 12x5 required)"));

    let mut app = test_app();
    let mut terminal = Terminal::new(TestBackend::new(12, 5)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(!text.contains("Terminal too small"));
    assert_eq!(app.terminal_area, Rect::new(0, 0, 12, 5));
}

#[test]
fn interview_form_is_drawn_inside_the_chat_panel_beside_the_canvas() {
    use ratatui::{Terminal, backend::TestBackend};

    let mut app = test_app();
    app.conversation = "interview-a".into();
    let question = serde_json::from_value(serde_json::json!({"id":"scope","context":"Scope affects timing.","question":"Which scope?","options":[{"label":"Small"},{"label":"Full"}]})).unwrap();
    app.view.workflow_phase = Some("interview".into());
    app.view.interview_question = Some((42, question));
    let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();

    let chat = app.chat_area;
    let graph = app.graph_area.expect("canvas stays visible");
    assert!(chat.width > 0 && chat.right() <= graph.x);
    let buffer = terminal.backend().buffer();
    let row = |y: u16, x0: u16, x1: u16| -> String {
        (x0..x1).map(|x| buffer[(x, y)].symbol()).collect()
    };
    let title_row = (0..24)
        .find(|&y| row(y, 0, 120).contains("Question · choose an answer"))
        .expect("form is drawn");
    let line = row(title_row, 0, 120);
    let col = line[..line.find("Question").unwrap()].chars().count() as u16;
    assert!(col < chat.right(), "form title must sit in the chat panel");
    assert!(
        (0..24).any(|y| row(y, graph.x, graph.right()).contains("orchestrator")),
        "canvas must keep rendering next to the form"
    );
}

#[test]
fn agent_canvas_remains_available_in_every_workflow_phase() {
    use ratatui::{Terminal, backend::TestBackend};

    for phase in ["interview", "planning", "review", "executing", "done"] {
        for width in [84, 120] {
            let mut app = test_app();
            app.view.workflow_phase = Some(phase.into());
            app.input.insert_str("unsent draft");
            let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
            terminal.draw(|frame| app.draw(frame)).unwrap();
            if width >= SIDE_BY_SIDE_BREAKPOINT {
                assert!(app.graph_area.is_some(), "{phase}");
                assert!(app.text_area.is_some(), "{phase}");
            }
            app.key(event::KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
            terminal.draw(|frame| app.draw(frame)).unwrap();
            assert_eq!(app.focus, Focus::Graph, "{phase}");
            assert!(app.graph_area.is_some(), "{phase}");
            assert_eq!(app.input.text, "unsent draft");
        }
    }
}

#[test]
fn ac_resp_01_app_draw_main_compact_switch_at_84_cols() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let mut app = test_app();
    app.focus = Focus::Chat;
    let backend = TestBackend::new(84, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            app.draw(frame);
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let text: String = buffer.content.iter().map(|c| c.symbol()).collect();
    assert!(text.contains("[Chat]"));
    assert!(text.contains("Team"));
    assert!(text.contains("(Tab: switch)"));

    // Switch to Focus::Graph
    app.focus = Focus::Graph;
    terminal
        .draw(|frame| {
            app.draw(frame);
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let text: String = buffer.content.iter().map(|c| c.symbol()).collect();
    assert!(text.contains("Chat"));
    assert!(text.contains("[Team]"));
    assert!(text.contains("(Tab: switch)"));
}

#[test]
fn ac_resp_02_app_draw_main_wide_split_at_120_cols() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let mut app = test_app();
    app.view.idle = true;
    let backend = TestBackend::new(120, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            app.draw(frame);
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let text: String = buffer.content.iter().map(|c| c.symbol()).collect();
    assert!(!text.contains("[Chat]"));
    assert!(!text.contains("(Tab: switch)"));
    assert!(text.contains("test"));
    assert!(!text.contains("antislop"));
    let strip = view::Stage::Fresh.strip(crate::lean::tui::theme::Theme::from_env().glyphs());
    assert!(text.contains(&strip), "{text}");
}

#[test]
fn the_header_shows_every_step_of_the_run_and_where_it_stands() {
    let unicode = crate::lean::tui::theme::Theme::new(true, false);
    let ascii = crate::lean::tui::theme::Theme::new(false, true);
    assert_eq!(
        view::Stage::Review.strip(unicode.glyphs()),
        "✓✓●○○○ plan review"
    );
    assert_eq!(
        view::Stage::Blocked.strip(unicode.glyphs()),
        "✓✓✓●○○ blocked"
    );
    assert_eq!(view::Stage::Done.strip(unicode.glyphs()), "✓✓✓✓✓✓ done");
    assert_eq!(view::Stage::Audit.strip(ascii.glyphs()), "xxxx*o audit");
}

#[test]
fn app_draw_transcript_empty_state() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let mut app = test_app();
    assert!(app.view.transcript.is_empty());
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            app.draw(frame);
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let text: String = buffer.content.iter().map(|c| c.symbol()).collect();
    assert!(text.contains("No messages in this conversation"));
}

#[test]
fn ac_edit_exact_width_chat_and_comment_cursor_stays_inside_inner_viewport() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    // 1. Chat input with text exactly matching inner width (60 cols wide -> inner width is 58)
    let mut app = test_app();
    app.input = InputBuffer::with_text("a".repeat(58));
    app.input.move_end();

    let backend = TestBackend::new(60, 12);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            app.draw(frame);
        })
        .unwrap();

    let (cursor_x, cursor_y) = terminal.get_cursor_position().unwrap().into();
    // Inner x is 1..=58. Border is at 0 (left) and 59 (right).
    // Cursor must be strictly inside the inner viewport: x <= 58, x >= 1.
    assert!(
        (1..=58).contains(&cursor_x),
        "Chat cursor x ({}) must be inside inner viewport [1, 58] and not land on right border 59",
        cursor_x
    );
    assert!(
        (1..=10).contains(&cursor_y),
        "Chat cursor y ({}) must be inside inner viewport",
        cursor_y
    );

    // 2. Comment editor with exact inner width text
    let mut app = test_app();
    app.editor = Some(Editor {
        target: "worker:t1".into(),
        input: InputBuffer::with_text("b".repeat(58)),
    });
    app.editor.as_mut().unwrap().input.move_end();

    terminal
        .draw(|frame| {
            app.draw(frame);
        })
        .unwrap();

    let (cursor_x, cursor_y) = terminal.get_cursor_position().unwrap().into();
    assert!(
        (1..=58).contains(&cursor_x),
        "Comment editor cursor x ({}) must be inside inner viewport and not on border",
        cursor_x
    );
    assert!(
        (1..=10).contains(&cursor_y),
        "Comment editor cursor y ({}) must be inside inner viewport",
        cursor_y
    );

    // 3. Resizing from wider to narrower terminal viewports
    let mut app = test_app();
    app.input = InputBuffer::with_text("first line of text\nsecond line of text");
    app.input.move_end();

    // Draw at 100x30 first
    let backend100 = TestBackend::new(100, 30);
    let mut term100 = Terminal::new(backend100).unwrap();
    term100.draw(|frame| app.draw(frame)).unwrap();

    // Resize down to 60x12
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let (rx, ry) = terminal.get_cursor_position().unwrap().into();
    assert!((1..=58).contains(&rx));
    assert!((1..=10).contains(&ry));
}

#[test]
fn ac_resp_app_draw_picker_visible_and_reversed_at_60x12() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let mut app = test_app();
    let mut picker = Picker::new(Kind::Commands);
    let items = picker.items(&app.view);
    assert_eq!(items.len(), 5, "Only five commands remain in the palette");
    picker.selected = items.len() - 1;
    app.picker = Some(picker);

    let backend = TestBackend::new(60, 12);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            app.draw(frame);
        })
        .unwrap();

    let buffer = terminal.backend().buffer();
    let last_label = &items[items.len() - 1].0;
    let last_word = last_label.split_whitespace().next().unwrap();

    let mut found_reversed = false;
    for y in 0..12 {
        let row: String = (0..60).map(|x| buffer[(x, y)].symbol()).collect();
        if row.contains(last_word) {
            for x in 0..60 {
                if buffer[(x, y)].modifier.contains(Modifier::REVERSED) {
                    found_reversed = true;
                    break;
                }
            }
        }
    }
    assert!(
        found_reversed,
        "Last picker item must be visible and reversed in App::draw at 60x12"
    );
}

#[test]
fn ac_help_app_draw_all_shortcuts_accessible_at_60x12() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let mut app = test_app();
    app.input = InputBuffer::with_text("draft message to preserve");
    app.show_help(Help::Chat);

    let backend = TestBackend::new(60, 12);
    let mut terminal = Terminal::new(backend).unwrap();

    // 1. Initial page (scroll = 0): review first set of shortcuts and full descriptions
    terminal
        .draw(|frame| {
            app.draw(frame);
        })
        .unwrap();

    let buffer1 = terminal.backend().buffer();
    let text1: String = buffer1.content.iter().map(|c| c.symbol()).collect();

    // Check shortcuts and their full descriptions
    assert!(text1.contains("send message to orchestrator"));
    assert!(text1.contains("newline in draft"));
    assert!(text1.contains("line in draft"));
    assert!(text1.contains("scroll conversation"));

    // 2. Navigate/scroll help down and review modal isolation
    let bottom = app.help_max_scroll();
    for _ in 0..bottom {
        app.key(event::KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    }
    assert_eq!(
        app.help_scroll, bottom,
        "Help should scroll down to max_scroll"
    );
    assert_eq!(
        app.input.text, "draft message to preserve",
        "Help navigation must not modify underlying input draft"
    );

    // 3. Scrolled view: review subsequent shortcuts and complete descriptions
    terminal
        .draw(|frame| {
            app.draw(frame);
        })
        .unwrap();

    let buffer2 = terminal.backend().buffer();
    let mut text2: String = buffer2.content.iter().map(|c| c.symbol()).collect();
    // Additional help entries may move a shortcut off the last page.
    for _ in 0..bottom {
        app.key(event::KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        terminal.draw(|frame| app.draw(frame)).unwrap();
        text2.extend(
            terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol()),
        );
    }

    assert!(text2.contains("line start / resume"));
    assert!(text2.contains("delete to start / end of line"));
    assert!(text2.contains("attach an image from the system clipboard"));
    assert!(text2.contains("remove the latest pending image"));
    assert!(text2.contains("cancel running work"));
    assert!(text2.contains("pause new steps / continue"));
    assert!(text2.contains("always sent as chat"));
    assert!(text2.contains("search conversation"));
    assert!(text2.contains("command palette"));
    assert!(text2.contains("back/clear draft"));

    // 4. Review End followed by Down, PageDown, Up (clamping and no overflow panic)
    let max_scroll = app.help_max_scroll();
    assert!(max_scroll >= 8);

    app.key(event::KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
    assert_eq!(app.help_scroll, max_scroll);

    // Down at bottom stays clamped to max_scroll without integer overflow
    app.key(event::KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.help_scroll, max_scroll);

    // PageDown at bottom stays clamped to max_scroll without integer overflow
    app.key(event::KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
    assert_eq!(app.help_scroll, max_scroll);

    // Up immediately decrements to max_scroll - 1 without deadlock
    app.key(event::KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.help_scroll, max_scroll - 1);

    // 5. Review resizing clamps stored help scroll
    let mut terminal_tall = Terminal::new(TestBackend::new(60, 24)).unwrap();
    terminal_tall
        .draw(|frame| {
            app.draw(frame);
        })
        .unwrap();
    assert!(app.help_max_scroll() < max_scroll);
    assert_eq!(
        app.help_scroll,
        app.help_max_scroll(),
        "Resizing must clamp stored scroll to new max_scroll"
    );

    // Resize back to 60x12
    terminal
        .draw(|frame| {
            app.draw(frame);
        })
        .unwrap();
    assert_eq!(app.help_max_scroll(), max_scroll);

    // 6. Graph help at 60x12: review long filter relation description is fully accessible without clipping
    app.show_help(Help::Graph);
    for _ in 0..2 {
        app.key(event::KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    }
    terminal
        .draw(|frame| {
            app.draw(frame);
        })
        .unwrap();
    let buffer_graph = terminal.backend().buffer();
    let text_graph: String = buffer_graph.content.iter().map(|c| c.symbol()).collect();
    assert!(text_graph.contains("show / hide"));
    assert!(text_graph.contains("dependencies"));
    assert!(text_graph.contains("review"));
    assert!(text_graph.contains("messages"));
    assert!(text_graph.contains("supervision"));

    // 7. Modal dismissal and draft preservation
    app.key(event::KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.help.is_none(), "Esc must close help modal");
    assert_eq!(
        app.input.text, "draft message to preserve",
        "Closing help modal must preserve the input draft"
    );
}

#[test]
fn ac_resp_app_draw_progress_overlay_collision_free() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    // 1. Render App with active progress in wide mode (100x30)
    let mut app = test_app();
    app.view.activity = Some("compiling tests".into());
    app.view.tasks = vec![view::TaskRow {
        id: "t1".into(),
        status: "working".into(),
        ..view::TaskRow::default()
    }];
    app.focus = Focus::Graph;
    app.selected = ORCHESTRATOR.into();

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            app.draw(frame);
        })
        .unwrap();

    let buffer = terminal.backend().buffer();
    let text: String = buffer.content.iter().map(|c| c.symbol()).collect();
    // Progress overlay renders with "now" title and doesn't obscure orchestrator
    assert!(text.contains("now"));
    assert!(text.contains("orchestrator") || text.contains(ORCHESTRATOR));

    // Review orchestrator node cells remain intact in the buffer
    let mut orchestrator_visible = false;
    for y in 0..30 {
        let row: String = (0..100).map(|x| buffer[(x, y)].symbol()).collect();
        if row.contains("orchestrator") || row.contains(ORCHESTRATOR) {
            orchestrator_visible = true;
            break;
        }
    }
    assert!(orchestrator_visible, "Selected node must remain visible");

    // 2. Compact render with tiny inner area where all overlay placements collide
    // The overlay is only hidden when the viewport is too small for it
    let mut app_compact = test_app();
    app_compact.view.activity = Some("very long activity line".into());
    app_compact.view.tasks = app.view.tasks.clone();
    app_compact.focus = Focus::Graph;

    // Create a fake test terminal where inner area forces collision
    let backend_tiny = TestBackend::new(60, 12);
    let mut term_tiny = Terminal::new(backend_tiny).unwrap();
    term_tiny
        .draw(|frame| {
            app_compact.draw(frame);
        })
        .unwrap();

    let tiny_buf = term_tiny.backend().buffer();
    let tiny_text: String = tiny_buf.content.iter().map(|c| c.symbol()).collect();
    // Even at 60x12 in graph focus, orchestrator node remains visible
    assert!(tiny_text.contains("orchestrator") || tiny_text.contains(ORCHESTRATOR));
}

#[test]
fn the_plan_opens_from_every_panel_and_rejecting_asks_for_a_reason() {
    let mut app = test_app();
    app.view.plan = Some("p1".into());
    app.view.tasks = vec![view::TaskRow {
        id: "t1".into(),
        status: "proposed".into(),
        ..view::TaskRow::default()
    }];
    app.key(key_ctrl('l'));
    assert!(
        app.plan_view.is_some(),
        "Ctrl+L opens the plan from the chat"
    );
    app.plan_view = None;
    // Even with the cursor in the draft, the shortcut does not type.
    app.input.insert_char('x');
    app.key(key_ctrl('l'));
    assert!(app.plan_view.is_some());
    assert_eq!(app.input.text, "x");

    app.handle_plan_action(PlanAction::Reject);
    let editor = app.editor.as_ref().expect("a reason editor opens");
    assert_eq!(editor.target, "reject:p1");
    // An empty reason may be submitted; the plan view closes.
    app.submit_comment();
    assert!(app.plan_view.is_none());
    assert!(app.editor.is_none());

    // Nothing to reject once the plan is decided.
    let mut decided = test_app();
    decided.view.plan = Some("p1".into());
    decided.handle_plan_action(PlanAction::Reject);
    assert!(decided.editor.is_none());
}

#[test]
fn the_narrow_header_keeps_a_clickable_plan_button() {
    for width in [50, 70, 94] {
        let line = render_header("p", "conversation-id", "ready", PanelFocus::Chat, width);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("[Plan]"), "{width}: {text}");
    }
    let wide = render_header("p", "conversation-id", "ready", PanelFocus::Chat, 120);
    let text: String = wide.spans.iter().map(|s| s.content.as_ref()).collect();
    assert_eq!(text.matches("[Plan]").count(), 1);
}

#[test]
fn chat_shows_sent_datetime_and_hides_interview_and_message_identifiers() {
    for question_id in [
        "f9ada472b97",
        "scope-confirmation-a6894657-4111-42bc-bac2-246e0c777f16",
    ] {
        let sent_at = 1_700_000_000_000_i64;
        let mut app = test_app();
        let event = serde_json::json!({"seq":1019,"kind":"message","created_ms":sent_at,"payload":{
            "author":"user","seq":4,
            "text":format!("Interview answer ({question_id})\nQuestion: Confirm scope?\nAnswer: Confirm scope 👩‍💻")
        }});
        app.view.apply(&event);
        assert_eq!(app.view.messages[0].sent_at, Some(sent_at));
        assert_eq!(app.view.messages[0].id, "m1019");
        assert!(!app.view.messages[0].text.contains(question_id));
        assert!(app.view.messages[0].text.contains("Confirm scope 👩‍💻"));
        for width in [100, 50, 12] {
            let mut terminal = Terminal::new(TestBackend::new(width, 12)).unwrap();
            terminal
                .draw(|frame| app.draw_transcript(frame, frame.area(), &Theme::new(false, false)))
                .unwrap();
            let rendered: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            assert!(!rendered.contains(question_id));
            assert!(!rendered.contains("m1019"));
            if width >= 50 {
                assert!(rendered.contains("2023-"), "{rendered}");
                assert!(rendered.contains(":20"), "{rendered}");
            }
        }
        app.view.apply(
            &serde_json::json!({"seq":1020,"kind":"message","created_ms":sent_at+1000,"payload":{
                "author":"orchestrator","text":"Scope confirmed"
            }}),
        );
        app.view.apply(&serde_json::json!({"seq":1021,"kind":"mail","created_ms":sent_at+2000,"payload":{
            "id":7,"sender":"user","recipient":"orchestrator","kind":"agent_work","text":"Continue"
        }}));
        assert_eq!(app.view.messages[1].sent_at, Some(sent_at + 1000));
        assert_eq!(app.view.messages[2].sent_at, Some(sent_at + 2000));
        let mut replay = view::View::default();
        replay.apply(&event);
        assert_eq!(replay.messages[0].sent_at, Some(sent_at));
    }
}

#[test]
fn transcript_cache_reuses_rows_and_matches_fresh_layout() {
    use ratatui::{Terminal, backend::TestBackend};
    let mut app = test_app();
    for i in 0..40 {
        app.view.cards.push(TranscriptCard::Message(MessageEntry {
            id: i.to_string(),
            sent_at: None,
            sender: "You".into(),
            target: "orchestrator".into(),
            text: format!("message {i}: {}", "👩‍💻 e\u{301} tekst ".repeat(20)),
            state: "sent".into(),
            kind: "user".into(),
        }));
    }
    let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
    let theme = Theme::new(true, false);
    let draw = |app: &mut App, term: &mut Terminal<TestBackend>, theme: &Theme| {
        term.draw(|frame| app.draw_transcript(frame, frame.area(), theme))
            .unwrap();
        term.backend().buffer().clone()
    };
    draw(&mut app, &mut term, &theme);
    draw(&mut app, &mut term, &theme);
    assert!(app.transcript_layout.retained_rows() <= 4096);
    if let TranscriptCard::Message(msg) = &mut app.view.cards[39] {
        msg.text.push_str(" updated");
        msg.state = "answered".into();
    }
    draw(&mut app, &mut term, &theme);
    assert!(app.transcript_layout.retained_rows() <= 4096);
    for (scroll, found, theme) in [
        (0, None, theme),
        (15, None, theme),
        (usize::MAX, Some(20), theme),
        (usize::MAX, None, Theme::new(false, true)),
    ] {
        app.scroll = scroll;
        app.found = found;
        let cached = draw(&mut app, &mut term, &theme);
        let settled_scroll = app.scroll;
        app.transcript_layout = TranscriptLayout::default();
        app.scroll = settled_scroll;
        assert_eq!(cached, draw(&mut app, &mut term, &theme));
    }
    term.resize(Rect::new(0, 0, 40, 16)).unwrap();
    let cached = draw(&mut app, &mut term, &theme);
    app.transcript_layout = TranscriptLayout::default();
    assert_eq!(cached, draw(&mut app, &mut term, &theme));
    app.view.cards.truncate(1);
    draw(&mut app, &mut term, &theme);
    assert_eq!(app.transcript_layout.entries.len(), 1);
    app.view.cards.push(TranscriptCard::Comment {
        sent_at: None,
        id: 7,
        target: "plan".into(),
        text: "please fix".into(),
        status: "open".into(),
    });
    app.view.comments.insert(
        7,
        view::Comment {
            id: 7,
            plan_id: None,
            target: "plan".into(),
            text: "please fix".into(),
            status: "open".into(),
        },
    );
    app.scroll = 0;
    draw(&mut app, &mut term, &theme);
    app.view.comment_progress.insert(
        7,
        view::CommentProgress {
            state: "failed".into(),
            error: Some("retry needed".into()),
        },
    );
    let cached = draw(&mut app, &mut term, &theme);
    let text: String = cached.content.iter().map(|cell| cell.symbol()).collect();
    assert!(text.contains("retry needed"));
    app.transcript_layout = TranscriptLayout::default();
    assert_eq!(cached, draw(&mut app, &mut term, &theme));
    app.conversation = "other".into();
    app.view.cards.clear();
    draw(&mut app, &mut term, &theme);
    assert!(app.transcript_layout.entries.is_empty());
}

#[test]
fn transcript_row_cache_stays_bounded_while_history_keeps_growing() {
    use ratatui::{Terminal, backend::TestBackend};
    let mut app = test_app();
    for i in 0..1500 {
        app.view.cards.push(TranscriptCard::Message(MessageEntry {
            id: i.to_string(),
            sent_at: None,
            sender: "You".into(),
            target: "orchestrator".into(),
            text: format!("entry {i} {}", "wrapped text ".repeat(8)),
            state: "sent".into(),
            kind: "user".into(),
        }));
    }
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|frame| app.draw_transcript(frame, frame.area(), &Theme::new(true, false)))
        .unwrap();
    assert_eq!(app.view.cards.len(), 1500);
    assert_eq!(app.transcript_layout.entries.len(), 1500);
    assert!(app.transcript_layout.retained_rows() <= 4096);
    assert!(app.transcript_layout.cached_entries() <= 1024);
    let stored = app.transcript_layout.stores;
    terminal
        .draw(|frame| app.draw_transcript(frame, frame.area(), &Theme::new(true, false)))
        .unwrap();
    // Cards outside the viewport keep their heights and are not wrapped again.
    assert!(app.transcript_layout.stores - stored <= 24);
}

#[test]
fn transcript_zero_area_preserves_viewport_state() {
    use ratatui::{Terminal, backend::TestBackend};
    let mut app = test_app();
    app.scroll = 7;
    app.text_area = Some(Rect::new(1, 2, 3, 4));
    let area = Rect::new(0, 0, 0, 10);
    let mut term = Terminal::new(TestBackend::new(10, 10)).unwrap();
    term.draw(|frame| app.draw_transcript(frame, area, &Theme::new(true, false)))
        .unwrap();
    assert_eq!(app.scroll, 7);
    assert_eq!(app.text_area, Some(Rect::new(1, 2, 3, 4)));
}

#[test]
fn transcript_narrow_area_renders_legacy_rows_safely() {
    use ratatui::{Terminal, backend::TestBackend};
    let mut app = test_app();
    app.view.transcript.push(view::Entry {
        style: Style::default(),
        text: "legacy transcript".into(),
    });
    let area = Rect::new(0, 0, 2, 1);
    let mut term = Terminal::new(TestBackend::new(2, 1)).unwrap();
    term.draw(|frame| app.draw_transcript(frame, area, &Theme::new(false, true)))
        .unwrap();
    assert_eq!(app.text_area, Some(Rect::new(0, 0, 1, 1)));
    assert_eq!(term.backend().buffer().content.len(), 2);
}

#[test]
fn transcript_workflow_fallback_replaces_empty_message() {
    use ratatui::{Terminal, backend::TestBackend};
    let mut app = test_app();
    app.view.workflow_phase = Some("interview".into());
    let mut term = Terminal::new(TestBackend::new(70, 20)).unwrap();
    term.draw(|frame| app.draw_transcript(frame, frame.area(), &Theme::new(true, false)))
        .unwrap();
    let text: String = term
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(text.contains("Phase 1/3: Interview and requirements"));
    assert!(!text.contains("No messages in this conversation"));
}

#[test]
fn scope_confirmation_card_shows_the_complete_requirements_summary() {
    use ratatui::{Terminal, backend::TestBackend};
    let mut app = test_app();
    let summary =
        "Build the import flow. Acceptance: preserve duplicate rows and report rejected records.";
    app.view.apply(&serde_json::json!({
        "seq": 1,
        "kind": "workflow",
        "payload": {
            "phase": "interview",
            "summary": summary
        }
    }));
    app.view.apply(&serde_json::json!({
        "seq": 2,
        "kind": "user_decision_requested",
        "payload": {
            "id": "scope-confirmation-test",
            "context": "Planning must use this scope.",
            "question": "Do you confirm this scope so planning can begin?",
            "options": [
                {"label": "Confirm scope", "description": "Proceed to planning."},
                {"label": "Request changes", "description": "Continue the interview."}
            ]
        }
    }));
    app.view.apply(&serde_json::json!({
        "seq": 3,
        "kind": "scope_confirmation_requested",
        "payload": {"summary": summary}
    }));
    assert_eq!(
        app.view.scope_confirmation_summary.as_deref(),
        Some(summary)
    );
    assert_eq!(
        app.view.interview_question.as_ref().unwrap().1.context,
        "Planning must use this scope."
    );

    let mut terminal = Terminal::new(TestBackend::new(120, 60)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .flat_map(|cell| cell.symbol().chars())
        .filter(|character| !character.is_whitespace())
        .collect();
    assert!(screen.contains("Buildtheimportflow."));
    assert!(screen.contains("preserveduplicaterows"));
    assert!(screen.contains("rejectedrecords."));
    assert!(screen.contains("Doyouconfirmthisscope"));
    assert!(screen.contains("planningcanbegin?"));

    // The scope decision spans both panels with margins and starts at the
    // beginning of a long summary. The wheel must scroll the modal only.
    let summary = (0..80)
        .map(|index| format!("REQ{index:03} preserve rows 👩‍💻e\u{301}\n"))
        .collect::<String>();
    app.input = InputBuffer::with_text("chat draft");
    app.focus = Focus::Graph;
    for (width, height) in [(120, 24), (84, 16)] {
        app.view.scope_confirmation_summary = Some(format!("{summary}Width {width}"));
        app.view.after += 1;
        app.sync_interview();
        app.interview_forms
            .get_mut(&app.conversation)
            .unwrap()
            .input = InputBuffer::with_text("decision draft 👩‍💻e\u{301}");
        terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let screen = |terminal: &Terminal<TestBackend>| -> String {
            terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect()
        };
        let initial = screen(&terminal);
        assert!(initial.contains("REQ000"));
        assert!(initial.contains("PgUp/PgDn / Mouse wheel"));
        assert!(initial.contains("👩‍💻"));
        assert!(initial.contains("e\u{301}"));
        let buffer = terminal.backend().buffer();
        let title_row = (0..height)
            .find(|&y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .contains("Question · choose an answer")
            })
            .unwrap();
        let left = (0..width)
            .find(|&x| buffer[(x, title_row)].symbol() == "┌")
            .unwrap();
        let right = (0..width)
            .rfind(|&x| buffer[(x, title_row)].symbol() == "┐")
            .unwrap();
        let bottom = (title_row..height)
            .find(|&y| buffer[(left, y)].symbol() == "└")
            .unwrap();
        assert!(left > 0 && right < width - 1, "side margins stay visible");
        assert!(
            title_row > 0 && bottom < height - 1,
            "vertical margins stay visible"
        );
        assert!(
            right - left > width * 3 / 4,
            "scope modal spans both panels"
        );
        let offset = app.offset;
        let chat_scroll = app.scroll;
        let wheel = |kind| event::MouseEvent {
            kind,
            column: right - 1,
            row: height / 2,
            modifiers: KeyModifiers::NONE,
        };
        let mut visited = initial.clone();
        for _ in 0..40 {
            app.mouse(wheel(MouseEventKind::ScrollDown));
            terminal.draw(|frame| app.draw(frame)).unwrap();
            visited.push_str(&screen(&terminal));
        }
        for index in 0..80 {
            assert!(visited.contains(&format!("REQ{index:03}")));
        }
        assert!(screen(&terminal).contains("Confirm scope"));
        for _ in 0..40 {
            app.mouse(wheel(MouseEventKind::ScrollUp));
            terminal.draw(|frame| app.draw(frame)).unwrap();
        }
        assert!(screen(&terminal).contains("REQ000"));
        assert!(!screen(&terminal).contains("REQ079"));
        assert_eq!(app.offset, offset);
        assert_eq!(app.scroll, chat_scroll);
        assert_eq!(app.input.text, "chat draft");
        assert_eq!(
            app.interview_forms[&app.conversation].input.text,
            "decision draft 👩‍💻e\u{301}"
        );

        let form = app.interview_forms[&app.conversation].clone();
        app.help = Some(palette::Help::Chat);
        app.mouse(wheel(MouseEventKind::ScrollDown));
        assert_eq!(app.interview_forms[&app.conversation], form);
        app.help = None;
        app.picker = Some(Picker::new(Kind::Commands));
        app.mouse(wheel(MouseEventKind::ScrollDown));
        assert_eq!(app.interview_forms[&app.conversation], form);
        app.picker = None;

        app.key(key_press(KeyCode::PageDown));
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert!(!screen(&terminal).contains("REQ000"));
        app.key(key_press(KeyCode::PageUp));
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert!(screen(&terminal).contains("REQ000"));
        app.key(key_press(KeyCode::Down));
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert!(screen(&terminal).contains("Request changes"));
        assert_eq!(app.interview_forms[&app.conversation].selected, 1);
        app.interview_forms
            .get_mut(&app.conversation)
            .unwrap()
            .selected = 0;
    }
}

#[test]
fn the_chat_shows_that_the_orchestrator_is_sending_then_thinking() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    let mut app = test_app();
    app.view
        .cards
        .push(view::TranscriptCard::Message(view::MessageEntry {
            id: "m1".into(),
            sent_at: None,
            sender: "You".into(),
            target: "orchestrator".into(),
            text: "build it".into(),
            state: "sent".into(),
            kind: "user".into(),
        }));
    let mut term = Terminal::new(TestBackend::new(120, 30)).unwrap();
    let screen = |app: &mut App, term: &mut Terminal<TestBackend>| -> String {
        term.draw(|f| app.draw(f)).unwrap();
        term.backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect()
    };
    assert!(!screen(&mut app, &mut term).contains("orchestrator sending"));

    app.sending = true;
    assert!(screen(&mut app, &mut term).contains("orchestrator sending…"));

    app.sending = false;
    app.view.activity = Some("orchestrator: exec ls".into());
    let thinking = screen(&mut app, &mut term);
    // Agent updates show in the graph's "now" box only, not above the input.
    assert!(thinking.contains(" now "));
    assert_eq!(thinking.matches("orchestrator: exec ls").count(), 1);

    // Between the daemon taking the message and the turn starting.
    app.view.activity = None;
    app.view.idle = false;
    assert!(screen(&mut app, &mut term).contains("waiting for the orchestrator to start"));
    app.view.idle = true;
    assert!(!screen(&mut app, &mut term).contains("orchestrator thinking"));
}

#[test]
fn a_stopped_step_can_be_unblocked_from_a_key_a_command_and_the_header() {
    let mut app = test_app();
    app.view.plan = Some("p1".into());
    let task = |id: &str, status: &str| view::TaskRow {
        id: id.into(),
        title: format!("Task {id}"),
        status: status.into(),
        change_kind: crate::lean::tasks::ChangeKind::Code,
        reason: Some("the reviewer repeated itself".into()),
        ..view::TaskRow::default()
    };
    app.view.tasks = vec![task("t1", "approved")];
    app.selected = "worker:t1".into();
    app.run_workflow(Command::TaskActions);
    assert!(app.unblock.is_none());
    assert!(
        app.notice
            .as_deref()
            .unwrap()
            .contains("nothing to unblock")
    );

    app.view.tasks = vec![
        task("t1", "approved"),
        task("t2", "needs_user"),
        task("t3", "skipped"),
    ];
    // The first stopped step that is not just skipped comes first.
    app.run_workflow(Command::TaskActions);
    assert_eq!(app.unblock.as_ref().unwrap().task, "t2");
    app.key(key_press(KeyCode::Esc));
    assert!(app.unblock.is_none());
    app.open_unblock(Some("t3".into()));
    assert_eq!(app.unblock.as_ref().unwrap().task, "t3");
    app.key(key_press(KeyCode::Esc));

    // The graph key acts on the selected step.
    app.view.tasks.push(task("t4", "failed"));
    app.focus = Focus::Graph;
    app.selected = "reviewer:t4".into();
    // Stage Blocked keeps the graph visible: nothing is running.
    app.key(key_press(KeyCode::Char('u')));
    assert_eq!(app.unblock.as_ref().unwrap().task, "t4");

    // A choice that takes a hint opens the editor for it.
    app.key(key_press(KeyCode::Char('1')));
    assert!(app.unblock.is_none());
    assert_eq!(
        app.editor.as_ref().unwrap().target,
        "unblock:retry_with_guidance:p1:t4"
    );
    app.editor = None;

    // The header offers it whenever a step is stuck.
    assert!(app.header_status().ends_with("[Unblock]"));
    app.view.tasks.retain(|t| t.status == "approved");
    assert!(!app.header_status().contains("[Unblock]"));
}

#[test]
fn a_cancelled_final_audit_is_recoverable_from_u_with_all_tasks_approved() {
    let mut app = test_app();
    app.view.apply(&serde_json::json!({"seq":1,"kind":"plan_proposed","payload":{"plan":"p","tasks":[{"id":"t1","title":"Work","change_kind":"none"}]}}));
    app.view.apply(&serde_json::json!({"seq":2,"kind":"task","payload":{"id":"t1","task":"uuid","status":"approved","round":1}}));
    for event in [
        serde_json::json!({"seq":3,"turn_id":"audit","kind":"turn_started","payload":{"role":"auditor"}}),
        serde_json::json!({"seq":4,"turn_id":"audit","kind":"turn_finished","payload":{"role":"auditor","status":"cancelled","error":"agent turn cancelled"}}),
        serde_json::json!({"seq":5,"kind":"audit","payload":{"plan":"p","approved":false,"error":"agent turn cancelled"}}),
    ] {
        app.view.apply(&event);
    }
    assert!(app.view.plan_resumable());
    assert!(app.view.controls().resume_plan);
    assert_eq!(app.view.stage(), view::Stage::Blocked);
    assert_eq!(app.phase_action().unwrap().1, Command::TaskActions);
    app.focus = Focus::Graph;
    app.selected = ORCHESTRATOR.into();
    app.key(key_press(KeyCode::Char('u')));
    let modal = app.unblock.take().unwrap();
    assert_eq!(modal.task, view::AUDITOR);
    assert_eq!(modal.plan, "p");
    assert_eq!(modal.reason, "agent turn cancelled");
    assert_eq!(modal.choices.len(), 1);
    assert!(!modal.choices[0].asks_text);
    assert!(modal.choices[0].label.contains("Retry the final audit"));
    app.open_unblock(Some(ORCHESTRATOR.into()));
    assert_eq!(app.unblock.take().unwrap().task, ORCHESTRATOR);
    app.view.apply(&serde_json::json!({"seq":6,"turn_id":"audit-2","kind":"turn_started","payload":{"role":"auditor"}}));
    assert!(!app.view.audit_retryable());
    app.open_unblock(Some(view::AUDITOR.into()));
    assert!(app.unblock.is_none());
    assert!(app.notice.as_ref().unwrap().contains("running"));
}

#[test]
fn the_orchestrator_actions_open_without_a_plan_and_with_an_existing_plan() {
    let mut app = test_app();
    app.focus = Focus::Graph;
    app.selected = ORCHESTRATOR.into();
    for plan in [None, Some("p1".into())] {
        app.view.plan = plan;
        app.key(key_press(KeyCode::Char('u')));
        let modal = app.unblock.take().unwrap();
        assert_eq!(modal.task, ORCHESTRATOR);
        assert_eq!(modal.choices.len(), 1);
        assert_eq!(
            modal.choices[0].action,
            crate::lean::RecoveryAction::RestartAgent
        );
        assert!(!modal.choices[0].asks_text);
    }
}

#[test]
fn an_active_step_can_be_restarted_from_the_actions_menu() {
    let mut app = test_app();
    app.view.plan = Some("p1".into());
    app.view.tasks = vec![view::TaskRow {
        id: "t1".into(),
        status: "working".into(),
        ..Default::default()
    }];
    app.selected = "worker:t1".into();
    app.focus = Focus::Graph;
    app.key(key_press(KeyCode::Char('u')));
    assert_eq!(app.unblock.as_ref().unwrap().task, "t1");
    app.key(key_press(KeyCode::Enter));
    assert_eq!(
        app.editor.as_ref().unwrap().target,
        "unblock:restart_agent:p1:t1"
    );
}

fn header_text(app: &App, width: u16) -> String {
    app.header_line(width)
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect()
}

fn click_header(app: &mut App, width: u16, token: &str) {
    let text = header_text(app, width.saturating_sub(4));
    let at = text
        .find(token)
        .unwrap_or_else(|| panic!("{token} in {text}"));
    let col = unicode_width::UnicodeWidthStr::width(&text[..at]) as u16 + 1;
    app.terminal_area = Rect::new(0, 0, width, 24);
    app.mouse(event::MouseEvent {
        kind: MouseEventKind::Down(event::MouseButton::Left),
        column: col,
        row: App::main_areas(app.terminal_area)[2].y,
        modifiers: KeyModifiers::NONE,
    });
}

#[test]
fn the_header_buttons_exist_on_narrow_terminals_and_know_what_they_can_do() {
    let mut app = test_app();
    app.view.idle = true;
    for width in [45, 60, 94, 95, 140, 160] {
        let text = header_text(&app, width);
        let tokens = if width >= 140 {
            [view::PAUSE, view::RESUME, view::CANCEL, view::NEW]
        } else if width >= 95 {
            [
                view::PAUSE_SHORT,
                view::RESUME_SHORT,
                view::CANCEL_SHORT,
                view::NEW_SHORT,
            ]
        } else {
            [
                view::PAUSE_ICON,
                view::RESUME_ICON,
                view::CANCEL_ICON,
                view::NEW_ICON,
            ]
        };
        for token in tokens {
            assert!(text.contains(token), "{width}: {text}");
        }
    }
    // Nothing runs: pause and cancel are dim, and a click does nothing.
    let controls = app.view.controls();
    assert!(!controls.pause && !controls.cancel && !controls.resume_plan);
    // While work runs they light up; paused work offers to continue.
    app.view.idle = false;
    assert!(app.view.controls().pause && app.view.controls().cancel);
    app.view.paused = true;
    assert!(app.view.controls().resume_paused && !app.view.controls().pause);
    assert!(header_text(&app, 60).contains("[▶]"));
    assert!(!header_text(&app, 60).contains("[‖]"));
    // Wide bars show one toggle too: never Pause and Continue side by side.
    for width in [100, 160] {
        let text = header_text(&app, width);
        assert!(
            text.contains("Continue") && !text.contains("Pause"),
            "{text}"
        );
    }
}

#[test]
fn footer_has_one_status_row_and_help_preserves_the_draft() {
    let mut app = test_app();
    app.paste("draft");
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 50)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let row_text = |buffer: &ratatui::buffer::Buffer, y| -> String {
        (0..120).map(|x| buffer[(x, y)].symbol()).collect()
    };
    let buffer = terminal.backend().buffer();
    assert!(row_text(buffer, 47).trim().is_empty());
    assert!(row_text(buffer, 48).ends_with("[?]"));
    assert!(row_text(buffer, 49).trim().is_empty());
    app.mouse(event::MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 118,
        row: 48,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(app.help, Some(Help::Chat));
    terminal.draw(|frame| app.draw(frame)).unwrap();
    app.key(key_press(KeyCode::Esc));
    assert!(app.help.is_none());
    assert_eq!(app.input.text, "draft");
}

#[test]
fn footer_clicks_follow_graphemes_and_the_rendered_row() {
    let mut app = test_app();
    app.project = "Zażółć 👩‍💻 e\u{301}".into();
    for (width, height) in [(80, 24), (180, 50)] {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let bar = App::main_areas(app.terminal_area)[2];
        let row = bar.y + (App::bar_height(app.terminal_area) - 1) / 2;
        let text = header_text(&app, width - 4);
        // Clicking each displayed grapheme must be safe, including wide emoji.
        let project_width = unicode_width::UnicodeWidthStr::width(app.project.as_str());
        for column in 0..project_width as u16 + 1 {
            app.mouse(event::MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column,
                row,
                modifiers: KeyModifiers::NONE,
            });
        }
        if width < SIDE_BY_SIDE_BREAKPOINT {
            let at = text.find("[Team]").unwrap();
            app.mouse(event::MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: unicode_width::UnicodeWidthStr::width(&text[..at]) as u16 + 1,
                row,
                modifiers: KeyModifiers::NONE,
            });
            assert_eq!(app.focus, Focus::Graph);
            app.focus = Focus::Chat;
        }
        app.mouse(event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: width - 2,
            row,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(app.help, Some(Help::Chat));
        app.key(key_press(KeyCode::Esc));
    }
}

#[test]
fn panels_and_buttons_in_the_header_can_be_clicked() {
    let mut app = test_app();
    app.view.idle = false;
    app.view.cards.push(view::TranscriptCard::System {
        sent_at: None,
        title: "x".into(),
        meta: String::new(),
        text: "x".into(),
        tone: Tone::Queued,
    });
    app.view.tasks = vec![view::TaskRow {
        id: "t1".into(),
        status: "working".into(),
        ..view::TaskRow::default()
    }];
    // Every panel is a button, not only the focused one.
    click_header(&mut app, 80, "[Team]");
    assert_eq!(app.focus, Focus::Graph);
    click_header(&mut app, 80, "[Chat]");
    assert_eq!(app.focus, Focus::Chat);
    // With a dialog open the header ignores clicks.
    app.focus = Focus::Chat;
    app.editor = Some(Editor {
        target: "plan:p1".into(),
        input: InputBuffer::new(),
    });
    click_header(&mut app, 80, "[Team]");
    assert_eq!(app.focus, Focus::Chat);
}
#[test]
fn copy_shortcut_queues_text_for_the_rendering_thread() {
    let mut app = test_app();
    let area = Rect::new(0, 0, 8, 1);
    let mut buffer = ratatui::buffer::Buffer::empty(area);
    buffer.set_string(0, 0, "safe", Style::default());
    app.text_selection
        .draw_scrolled(&mut buffer, Some(area), app.selection_context(), 0);
    for (kind, column) in [
        (MouseEventKind::Down(MouseButton::Left), 0),
        (MouseEventKind::Drag(MouseButton::Left), 3),
    ] {
        app.text_selection.mouse(event::MouseEvent {
            kind,
            column,
            row: 0,
            modifiers: KeyModifiers::NONE,
        });
    }
    assert!(app.key(key_ctrl('y')));
    let text = app
        .pending_clipboard
        .take()
        .expect("key queues text, never writes stdout");
    assert_eq!(text, "safe");
    let mut output = Vec::new();
    assert_eq!(
        clipboard::copy_text(&mut output, &text).unwrap(),
        clipboard::CopyMethod::Osc52
    );
    assert_eq!(output, b"\x1b]52;c;c2FmZQ==\x07");
    assert!(app.pending_clipboard.is_none());
}

#[test]
fn the_wheel_scrolls_the_graph_it_is_over_and_the_view_stays_where_the_user_left_it() {
    use ratatui::{Terminal, backend::TestBackend};
    let mut app = test_app();
    app.view.tasks = (0..12)
        .map(|n| view::TaskRow {
            id: format!("t{n}"),
            title: format!("Task {n}"),
            status: "working".into(),
            ..view::TaskRow::default()
        })
        .collect();
    app.view.layout_revision += 1;
    let mut terminal = Terminal::new(TestBackend::new(140, 40)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let area = app.graph_area.expect("graph drawn beside the chat");
    assert_eq!(app.focus, Focus::Chat);
    for _ in 0..3 {
        app.mouse(event::MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: area.x + 1,
            row: area.y + 1,
            modifiers: KeyModifiers::NONE,
        });
    }
    let scrolled = app.offset.1;
    assert!(scrolled > 0, "the wheel over the graph pans it");
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert_eq!(
        app.offset.1, scrolled,
        "the selection does not pull it back"
    );
    assert_eq!(app.focus, Focus::Chat, "scrolling moves no focus");
}

#[test]
fn the_bar_drops_whole_parts_to_fit_rather_than_cutting_a_button() {
    let mut app = test_app();
    app.project = "project · a long project name".into();
    app.conversation = "bebe421c-3365-477b-9bfc-3d5e7d84ef04".into();
    for width in 25..140u16 {
        let line = app.header_line(width);
        let text = header_text(&app, width);
        assert!(line.width() <= usize::from(width), "{width}: {text:?}");
        // Every bracket that opens closes: no button is cut in half.
        assert_eq!(
            text.matches('[').count(),
            text.matches(']').count(),
            "{width}: {text:?}"
        );
    }
    // A phone-wide bar keeps the panel switch and the plan's actions.
    let phone = header_text(&app, 68);
    assert!(
        phone.contains("[Chat]") && phone.contains("[Team]"),
        "{phone:?}"
    );
}

#[test]
fn a_wrapped_list_item_continues_under_its_text() {
    use super::theme::wrap;
    assert_eq!(
        wrap("  - [high] audit and review are missing or stale", 24),
        [
            "  - [high] audit and ",
            "    review are missing ",
            "    or stale"
        ]
    );
    assert_eq!(
        wrap("12. first step done", 12),
        ["12. first ", "    step ", "    done"]
    );
    assert_eq!(wrap("-dash not a list", 8), ["-dash ", "not a ", "list"]);
}

#[test]
fn the_inspector_has_a_shortcut_a_terminal_does_not_send_as_tab() {
    let mut app = App::test_app();
    app.key(event::KeyEvent::new(
        KeyCode::Char('t'),
        event::KeyModifiers::CONTROL,
    ));
    let details = app.details.as_ref().expect("Ctrl+T opens the inspector");
    assert_eq!(details.key, "conversation");
}

#[test]
fn a_hint_drops_its_least_needed_parts_and_keeps_its_way_out() {
    use super::view::fit_hint;
    let parts = ["↑/↓ card", "Enter fold", "e all", "Esc back"];
    assert_eq!(
        fit_hint(&parts, 80, true),
        "↑/↓ card · Enter fold · e all · Esc back"
    );
    assert_eq!(
        fit_hint(&parts, 33, true),
        "↑/↓ card · Enter fold · Esc back"
    );
    assert_eq!(fit_hint(&parts, 5, true), "↑/↓ card · Esc back");
    assert_eq!(fit_hint(&parts[..3], 22, false), "↑/↓ card · Enter fold");
}

#[test]
fn a_window_another_frontend_steers_says_it_only_watches() {
    let mut app = test_app();
    let follow = |app: &mut App, observer| {
        app.apply_update(Update::Events {
            conversation: app.conversation.clone(),
            generation: app.generation,
            events: Vec::new(),
            idle: true,
            observer,
        })
    };
    follow(&mut app, true);
    assert!(app.header_status().contains("observer"));
    follow(&mut app, false);
    assert!(!app.header_status().contains("observer"));
}

#[test]
fn late_document_fetches_cannot_replace_selection_or_discard_a_draft() {
    let mut app = test_app();
    app.input = InputBuffer::with_text("Keep my draft 👩‍💻");
    let mut details = Details::new("orchestrator".into());
    details.load_task(serde_json::json!({"_result_documents":[{"id":"new","path":"new.md"}]}));
    details.switch_tab(details::RESULT);
    details.query = "café".into();
    details.selected_result_document = Some("result_document:new".into());
    details.result_selection_generation = 2;
    app.details = Some(details);
    for (conversation, generation, selection, id) in [
        ("other".to_owned(), app.generation, 2, "result_document:new"),
        (
            app.conversation.clone(),
            app.generation + 1,
            2,
            "result_document:new",
        ),
        (
            app.conversation.clone(),
            app.generation,
            1,
            "result_document:old",
        ),
    ] {
        app.apply_update_inner(Update::ResultDocument {
            conversation,
            generation,
            key: "orchestrator".into(),
            selection,
            id: id.into(),
            result: Err("late error".into()),
        });
        assert!(app.details.as_ref().unwrap().result_error.is_none());
    }
    app.apply_update_inner(Update::ResultDocument {
        conversation: app.conversation.clone(),
        generation: app.generation,
        key: "orchestrator".into(),
        selection: 2,
        id: "result_document:new".into(),
        result: Err("content file was removed".into()),
    });
    let details = app.details.as_ref().unwrap();
    assert_eq!(
        details.result_error.as_ref().unwrap().1,
        "content file was removed"
    );
    assert_eq!(details.query, "café");
    assert_eq!(app.input.text, "Keep my draft 👩‍💻");
}

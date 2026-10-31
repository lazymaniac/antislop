use super::view::render_header;
use super::*;
use ratatui::buffer::Buffer;

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
    app.selected = "verifier:t1".into();
    app.editor = Some(Editor {
        target: app.selected.clone(),
        input: InputBuffer::with_text("draft comment"),
    });
    assert!(app.key(key_press(KeyCode::Esc)));
    assert!(app.editor.is_none());
    assert_eq!(app.selected, "verifier:t1");
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
        "payload": {"plan": "p1", "tasks": [{"id": "t1", "title": "A"}]}
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
    app.details = Some(Details::new("verifier:t1".into()));
    app.key(key_press(KeyCode::Char('c')));
    assert_eq!(app.editor.as_ref().unwrap().target, "verifier:t1");
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

fn requirements_event(app: &mut App, seq: u64, run: &str, resolved: bool) {
    let mut events = vec![
        serde_json::json!({"seq":seq,"kind":"ledger_approval_required","payload":{"run":run}}),
    ];
    if resolved {
        events.push(serde_json::json!({"seq":seq+1,"kind":"ledger_proposal_rejected","payload":{"run":run}}));
    }
    app.apply_update(Update::Events {
        conversation: app.conversation.clone(),
        generation: app.generation,
        events,
        idle: true,
        autonomous: false,
    });
}
fn review_app() -> App {
    let mut app = test_app();
    app.input = InputBuffer::with_text("Unsent chat 👩‍💻");
    let mut ledger = ledger::LedgerView::new();
    ledger.preview = Some(serde_json::json!({"proposal_id":"p1","diff_hash":"hash"}));
    ledger.proposed_change = Some(crate::lean::ledger::LedgerChange {
        conversation: app.conversation.clone(),
        run_id: "r1".into(),
        operation_id: "op1".into(),
        expected_spec_revision: 0,
        reason: "fix".into(),
        entries: vec![],
    });
    ledger.request_id = 12;
    app.ledger_view = Some(ledger);
    app
}
#[test]
fn requirements_decisions_return_to_main_only_after_success() {
    for kind in ["applied", "rejected"] {
        let mut app = review_app();
        app.details = Some(Details::new("worker:t1".into()));
        app.plan_view = Some(PlanView::new("plan".into()));
        app.editor = Some(Editor {
            target: "orchestrator".into(),
            input: InputBuffer::with_text("Comment draft"),
        });
        app.ledger_view.as_mut().unwrap().rejection = InputBuffer::with_text("Correction draft");
        app.apply_update(Update::Ledger {
            conversation: app.conversation.clone(),
            request_id: 12,
            kind: kind.into(),
            reply: Err("raw failure".into()),
        });
        assert!(app.ledger_view.as_ref().unwrap().visible);
        assert!(app.ledger_view.as_ref().unwrap().preview.is_some());
        assert_eq!(
            app.ledger_view.as_ref().unwrap().rejection.text,
            "Correction draft"
        );
        app.apply_update(Update::Ledger {
            conversation: app.conversation.clone(),
            request_id: 12,
            kind: kind.into(),
            reply: Ok(serde_json::json!({"spec_revision":1})),
        });
        assert!(!app.ledger_view.as_ref().unwrap().visible);
        assert!(app.details.is_none() && app.plan_view.is_none() && app.editor.is_none());
        assert_eq!(app.input.text, "Unsent chat 👩‍💻");
        assert!(
            app.comment_drafts
                .values()
                .any(|text| text == "Comment draft")
        );
    }
}
#[test]
fn replay_opens_only_pending_reviews_and_respects_dismissal() {
    let mut app = test_app();
    requirements_event(&mut app, 1, "r1", true);
    assert!(app.ledger_view.is_none());
    requirements_event(&mut app, 3, "r1", false);
    assert!(app.ledger_view.as_ref().unwrap().pending);
    let mut view = ledger::LedgerView::new();
    view.approval_event = 3;
    view.visible = false;
    app.ledger_view = Some(view);
    requirements_event(&mut app, 3, "r1", false);
    assert!(!app.ledger_view.as_ref().unwrap().visible);
    requirements_event(&mut app, 4, "r1", false);
    assert!(app.ledger_view.as_ref().unwrap().visible);
}
#[test]
fn queued_review_waits_for_write_and_late_reply_cannot_replace_it() {
    let mut app = review_app();
    app.ledger_view.as_mut().unwrap().pending = true;
    app.ledger_view.as_mut().unwrap().pending_kind = "applied".into();
    requirements_event(&mut app, 2, "r2", false);
    assert_eq!(
        app.ledger_view.as_ref().unwrap().review_run.as_deref(),
        Some("r2")
    );
    app.apply_update(Update::Ledger {
        conversation: app.conversation.clone(),
        request_id: 12,
        kind: "applied".into(),
        reply: Ok(serde_json::json!({})),
    });
    let view = app.ledger_view.as_ref().unwrap();
    assert!(view.visible && view.pending);
    assert_eq!(view.pending_run.as_deref(), Some("r2"));
    app.apply_update(Update::Ledger {
        conversation: app.conversation.clone(),
        request_id: 12,
        kind: "preview".into(),
        reply: Ok(serde_json::json!({"proposal":{"proposal_id":"old"}})),
    });
    assert!(app.ledger_view.as_ref().unwrap().preview.is_none());
}
#[test]
fn queued_review_survives_waiting_for_conversation_history() {
    let mut app = review_app();
    app.ledger_view.as_mut().unwrap().review_run = Some("r2".into());
    app.apply_update(Update::Ledger {
        conversation: app.conversation.clone(),
        request_id: 11,
        kind: "preview".into(),
        reply: Err("late".into()),
    });
    assert_eq!(
        app.ledger_view.as_ref().unwrap().review_run.as_deref(),
        Some("r2")
    );
    requirements_event(&mut app, 2, "r2", false);
    assert!(app.ledger_view.as_ref().unwrap().pending);
    assert_eq!(
        app.ledger_view.as_ref().unwrap().pending_run.as_deref(),
        Some("r2")
    );
}
#[test]
fn review_escape_closes_details_and_preserves_search_and_chat() {
    let mut app = review_app();
    app.ledger_view.as_mut().unwrap().query = InputBuffer::with_text("filter");
    let mut details = Details::new("worker:t1".into());
    details.native_focus = true;
    app.details = Some(details);
    assert!(!app.native_focused());
    app.paste("must not edit chat");
    assert_eq!(app.input.text, "Unsent chat 👩‍💻");
    app.key(key_press(KeyCode::Esc));
    assert!(app.details.is_none());
    assert!(!app.ledger_view.as_ref().unwrap().visible);
    assert_eq!(app.ledger_view.as_ref().unwrap().query.text, "filter");
    assert_eq!(app.input.text, "Unsent chat 👩‍💻");
}
#[test]
fn requirements_approval_automatically_opens_comparison() {
    let mut app = test_app();
    app.input = InputBuffer::with_text("Draft 👩‍💻");
    app.apply_update(Update::Events {
        conversation: app.conversation.clone(),
        generation: app.generation,
        events: vec![serde_json::json!({
            "seq": 1, "kind": "ledger_approval_required", "payload": {"run": "r1"}
        })],
        idle: true,
        autonomous: false,
    });
    let ledger = app.ledger_view.as_ref().unwrap();
    assert!(ledger.visible);
    assert!(ledger.pending);
    assert_eq!(ledger.pending_kind, "preview");
    assert_eq!(app.input.text, "Draft 👩‍💻");
}

#[test]
fn requirements_button_opens_ledger_and_preserves_chat_draft() {
    use ratatui::{Terminal, backend::TestBackend};
    let mut app = test_app();
    app.view.requirements_approval_run = Some("run1".into());
    app.input = InputBuffer::with_text("My comment 👩‍💻");
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let area = app
        .phase_action_area
        .expect("requirements action above input");
    app.mouse(event::MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: area.x,
        row: area.y,
        modifiers: KeyModifiers::NONE,
    });
    assert!(app.ledger_view.as_ref().is_some_and(|view| view.visible));
    assert_eq!(app.input.text, "My comment 👩‍💻");
}

#[test]
fn interview_shortcut_guides_approval_and_preserves_unsent_answers() {
    let mut app = test_app();
    app.view.workflow_phase = Some("interview".into());
    app.key(event::KeyEvent::new(
        KeyCode::Char('a'),
        KeyModifiers::CONTROL,
    ));
    assert!(
        app.notice
            .as_deref()
            .unwrap()
            .contains("requirements summary")
    );
    app.view.requirements_summary = "Goal and criteria".into();
    app.input = InputBuffer::with_text("Correction 👩‍💻");
    app.key(event::KeyEvent::new(
        KeyCode::Char('a'),
        KeyModifiers::CONTROL,
    ));
    assert_eq!(app.input.text, "Correction 👩‍💻");
    assert_eq!(app.input.cursor, 0);
    app.run_workflow(Command::ApproveRequirements);
    assert!(app.notice.as_deref().unwrap().contains("send your answers"));
    assert_eq!(app.input.text, "Correction 👩‍💻");
}

#[test]
fn chat_ctrl_a_plan_accept_vs_readline_home() {
    let mut app = test_app();
    app.view.apply(&serde_json::json!({
        "seq": 1,
        "kind": "plan_proposed",
        "payload": {"plan": "p1", "tasks": [{"id": "t1", "title": "A"}]}
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
    assert!(text.contains("1/3 interview"));
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
    assert!(items.len() >= 10);
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

    // 1. Initial page (scroll = 0): verify first set of shortcuts and full descriptions
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
    assert!(text1.contains("accept/resume"));
    assert!(text1.contains("delete to start / end of line"));

    // 2. Navigate/scroll help down and verify modal isolation
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

    // 3. Scrolled view: verify subsequent shortcuts and complete descriptions
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

    assert!(text2.contains("cancel running work"));
    assert!(text2.contains("pause new steps / continue"));
    assert!(text2.contains("autonomous / supervised mode"));
    assert!(text2.contains("always sent as chat"));
    assert!(text2.contains("search conversation"));
    assert!(text2.contains("command palette"));
    assert!(text2.contains("back/clear draft"));

    // 4. Verify End followed by Down, PageDown, Up (clamping and no overflow panic)
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

    // 5. Verify resizing clamps stored help scroll
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

    // 6. Graph help at 60x12: verify long filter relation description is fully accessible without clipping
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
    assert!(text_graph.contains("verification"));
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

    // Verify orchestrator node cells remain intact in the buffer
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
        let line = render_header(
            "p",
            "conversation-id",
            Some(false),
            "ready",
            PanelFocus::Chat,
            width,
        );
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("[Plan]"), "{width}: {text}");
    }
    let wide = render_header(
        "p",
        "conversation-id",
        Some(false),
        "ready",
        PanelFocus::Chat,
        120,
    );
    let text: String = wide.spans.iter().map(|s| s.content.as_ref()).collect();
    assert_eq!(text.matches("[Plan]").count(), 1);
}

#[test]
fn transcript_cache_reuses_rows_and_matches_fresh_layout() {
    use ratatui::{Terminal, backend::TestBackend};
    let mut app = test_app();
    for i in 0..40 {
        app.view.cards.push(TranscriptCard::Message(MessageEntry {
            id: i.to_string(),
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
fn the_chat_shows_that_the_orchestrator_is_sending_then_thinking() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    let mut app = test_app();
    app.view
        .cards
        .push(view::TranscriptCard::Message(view::MessageEntry {
            id: "m1".into(),
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
        writes: true,
        reason: Some("the verifier repeated itself".into()),
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
    app.selected = "verifier:t4".into();
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
    // Compact mode token at the narrowest widths.
    app.view.autonomous = Some(true);
    assert!(header_text(&app, 40).contains("[auto]"));
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
    // The mode does not flip until the daemon says so.
    app.view.autonomous = Some(false);
    app.run_workflow(Command::ToggleMode);
    assert_eq!(app.view.autonomous, Some(false));
    app.key(key_ctrl('t'));
    assert_eq!(app.view.autonomous, Some(false));

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

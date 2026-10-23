use antislop::lean::{
    comment_label,
    daemon::{Client, Request, socket},
    diff::ReviewerProfile,
    tui::{
        App, CardProps, ConfirmationAction, ConfirmationCard, ConfirmationStatus, Details,
        DiffView, Editor, Filter, Focus, Help, InputBuffer, Kind, Picker, Theme, TranscriptCard,
        Update, card,
        diffview::diff_header_with_reviewer,
        draw_rows,
        editor::{
            MAX_PASTE_BYTES, compute_visual_lines, sanitize_paste, sanitize_single_line_paste,
            visual_text_lines,
        },
        in_ascii, sanitize,
        view::{
            MIN_TERM_HEIGHT, MIN_TERM_WIDTH, PanelFocus, SIDE_BY_SIDE_BREAKPOINT,
            draw_min_term_guard, main_footer_hint, render_header, target_name,
            transcript_empty_state,
        },
        without_color,
    },
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Terminal,
    backend::TestBackend,
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Span,
};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

#[test]
fn plan_comments_show_queue_current_id_and_raw_failure_inside_the_plan() {
    use antislop::lean::tui::{PlanView, view::View};
    let mut app = App::test_app();
    app.view = View::default();
    let mut seq = 0;
    let mut event = |kind: &str, turn: Option<&str>, payload: Value| {
        seq += 1;
        json!({"seq": seq, "kind": kind, "turn_id": turn, "payload": payload})
    };
    app.view.apply(&event("plan_proposed", None, json!({"plan":"p1", "document":"# Plan\nImplement change", "tasks":[{"id":"t1", "title":"Change", "status":"proposed"}]})));
    for (id, text) in [(1, "First feedback"), (2, "Second feedback")] {
        app.view.apply(&event(
            "comment",
            None,
            json!({"id":id, "plan_id":"p1", "target":"plan:p1", "text":text, "status":"open"}),
        ));
    }
    assert_eq!(app.view.comment_state(1), "queued");
    assert_eq!(app.view.comment_state(2), "queued");
    app.view.apply(&event(
        "turn_started",
        Some("c1"),
        json!({"role":"comment-orchestrator", "provider":"codex", "comments":[1]}),
    ));
    app.view.apply(&event(
        "agent",
        Some("c1"),
        json!({"type":"tool", "name":"exec", "detail":"read plan"}),
    ));
    assert!(
        app.view
            .orchestrator_activity()
            .unwrap()
            .contains("comment #1")
    );
    assert_eq!(app.view.comment_state(2), "queued");
    app.plan_view = Some(PlanView::new("p1".into()));
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Addressing comment #1"));

    app.view.apply(&event("turn_finished", Some("c1"), json!({"role":"comment-orchestrator", "status":"failed", "error":"thread-store conflict: already has an active writer"})));
    assert!(app.view.comment_activity().is_none());
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("#1 [failed]"));
    assert!(text.contains("already has an active writer"));
    assert!(text.contains("o retry/reopen"));
    assert_eq!(
        app.plan_view.as_mut().unwrap().key(
            KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE),
            &app.view
        ),
        antislop::lean::tui::PlanAction::ToggleResolveComment {
            id: 1,
            next_status: "open".into()
        }
    );
    app.view.apply(&event(
        "turn_started",
        Some("c2"),
        json!({"role":"comment-orchestrator", "provider":"codex", "comments":[2]}),
    ));
    assert!(
        app.view
            .orchestrator_activity()
            .unwrap()
            .contains("comment #2")
    );
    assert_eq!(app.view.comment_state(1), "failed");
}

#[test]
fn graph_cards_show_semantic_summaries_and_preserve_full_details() {
    use antislop::lean::tui::{
        graph::{Canvas, Graph, HEIGHT, WIDTH},
        view::{Agent, TaskRow, View},
    };
    let mut view = View::default();
    view.tasks.push(TaskRow {
        id: "t1".into(),
        title: "Full task title".into(),
        status: "working".into(),
        summary: Some("Improve  storage\n behavior 👩‍💻 e\u{301}.".into()),
        verifier_summary: Some("Verify storage correctness.".into()),
        goal: format!("{} TASK_END", "Long task description ".repeat(35)),
        scope: vec!["src/file.rs".into()],
        focus: format!("{} VERIFY_END", "Verify all required behavior ".repeat(12)),
        risks: vec!["RISK_END".into()],
        ..Default::default()
    });
    for active_role in ["worker", "verifier"] {
        view.tasks[0].status = if active_role == "worker" {
            "working"
        } else {
            "verifying"
        }
        .into();
        view.agents.insert(
            format!("{active_role}:t1"),
            Agent {
                running: true,
                activity: "exec cargo check".into(),
                ..Default::default()
            },
        );
        let graph = Graph::new(&view);
        let worker = graph.node("worker:t1").unwrap();
        let verifier = graph.node("verifier:t1").unwrap();
        assert_eq!(verifier.key, worker.key);
        for key in ["worker:t1", "verifier:t1"] {
            let node = graph.node(key).unwrap();
            assert_eq!(
                node.height, HEIGHT,
                "every card is exactly as tall as the budget"
            );
            let mut canvas = Canvas::new_window(node.x, node.y, WIDTH, node.height);
            graph.render(&view, &mut canvas, Filter::All, key);
            let text = canvas.lines().join("\n");
            assert!(
                !text.contains("TASK_END")
                    && !text.contains("VERIFY_END")
                    && !text.contains("RISK_END")
            );
            assert!(!text.contains("src/file.rs"));
            // The long activity phrase is cut at the card edge; the state
            // line alone holds state, and the running role's doing text
            // has a line of its own.
            assert!(!text.contains("exec cargo check"));
            assert!(text.contains("doing: exec cargo"), "{text}");
            if key == "worker:t1" {
                assert!(text.contains("Improve storage"), "{text}");
                assert!(text.contains("👩‍💻") && text.contains("e\u{301}"), "{text}");
            } else {
                assert!(text.contains("Verify storage"), "{text}");
                assert!(text.contains("correctness."), "{text}");
            }
            let mut details = Details::new(key.into());
            details.show_info = true;
            let full = details
                .lines(&view)
                .into_iter()
                .map(|(_, text)| text)
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                full.contains("TASK_END")
                    && full.contains("VERIFY_END")
                    && full.contains("src/file.rs")
            );
            if key.starts_with("verifier") {
                assert!(full.contains("RISK_END"));
            }
        }
        for (_, _, path) in graph.edge_routes(Filter::All) {
            for node in graph.nodes.iter().filter(|node| node.visible) {
                assert!(!Graph::route_intersects_card(&path, node));
            }
        }
    }
}

#[test]
fn graph_scrolls_through_many_short_cards() {
    use antislop::lean::tui::{
        graph::{Graph, HEIGHT},
        view::TaskRow,
    };
    let mut app = App::test_app();
    app.view.tasks = (0..12)
        .map(|i| TaskRow {
            id: format!("t{i}"),
            title: format!("Task {i}"),
            summary: Some(format!("Perform task {i}.")),
            verifier_summary: Some(format!("Check task {i}.")),
            ..Default::default()
        })
        .collect();
    app.focus = Focus::Graph;
    app.selected = "worker:t0".into();
    let mut terminal = Terminal::new(TestBackend::new(80, 21)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let start = app.offset.1;
    assert!(app.key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE)));
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(app.offset.1 > start);
    for _ in 0..30 {
        app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    }
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(app.offset.1 > start);
    assert_eq!(app.selected, "worker:t10");
    let text: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(
        text.contains("Check task 11."),
        "the final short card must be reachable: {text}"
    );
    let graph = Graph::new(&app.view);
    assert!(graph.nodes.iter().all(|node| node.height == HEIGHT));
    assert!(app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
    assert_eq!(app.focus, Focus::Chat);
}

#[test]
fn escape_never_quits_and_q_quits_views_while_editors_keep_text() {
    for focus in [Focus::Chat, Focus::Graph] {
        let mut app = App::test_app();
        app.focus = focus;
        assert!(app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        assert!(!app.key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)));
    }
    let mut app = App::test_app();
    app.plan_view = Some(antislop::lean::tui::PlanView::new("p1".into()));
    assert!(app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
    assert!(app.plan_view.is_none());
    app.open_details("orchestrator".into());
    assert!(!app.key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)));
    app.details = None;
    app.editor = Some(Editor {
        target: "plan:p1".into(),
        input: InputBuffer::new(),
    });
    assert!(app.key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)));
    assert_eq!(app.editor.as_ref().unwrap().input.text, "q");
    assert!(!app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)));
}

#[test]
fn large_multiline_paste_keeps_the_cursor_and_last_line_visible() {
    let mut app = App::test_app();
    app.paste(&"\n".repeat(MAX_PASTE_BYTES));
    app.paste("finish");
    let (col, row) = app.input.visual_cursor(58);
    assert_eq!(col, 6);
    assert_eq!(row, MAX_PASTE_BYTES);

    app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.input.visual_cursor(58).1, MAX_PASTE_BYTES - 1);
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.input.visual_cursor(58).1, MAX_PASTE_BYTES);
    app.input.move_buffer_end();

    let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert_eq!(app.input_shown, MAX_PASTE_BYTES - 5);
    let cursor = terminal.get_cursor_position().unwrap();
    assert_eq!(cursor.y, 9);
    assert_eq!(terminal.backend().buffer()[(1, cursor.y)].symbol(), "f");

    app.editor = Some(Editor {
        target: "worker:t1".into(),
        input: app.input.clone(),
    });
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let cursor = terminal.get_cursor_position().unwrap();
    assert_eq!(cursor.y, 9);
    assert_eq!(terminal.backend().buffer()[(1, cursor.y)].symbol(), "f");
}

#[test]
fn full_width_drafts_show_the_cursor_after_the_last_character() {
    let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
    for text in ["a".repeat(58), "漢".repeat(29)] {
        let mut app = App::test_app();
        app.input = InputBuffer::with_text(&text);
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let cursor = terminal.get_cursor_position().unwrap();
        let (col, _) = app.input.visual_cursor(app.input_width);
        assert_eq!(usize::from(cursor.x), 1 + usize::from(col));
        assert_eq!(
            terminal.backend().buffer()[(cursor.x, cursor.y)].symbol(),
            " "
        );

        app.editor = Some(Editor {
            target: "worker:t1".into(),
            input: InputBuffer::with_text(text),
        });
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let cursor = terminal.get_cursor_position().unwrap();
        let (col, _) = app
            .editor
            .as_ref()
            .unwrap()
            .input
            .visual_cursor(app.editor_width);
        assert_eq!(usize::from(cursor.x), 1 + usize::from(col));
        assert_eq!(
            terminal.backend().buffer()[(cursor.x, cursor.y)].symbol(),
            " "
        );
    }
}

#[test]
fn long_pasted_search_queries_keep_the_cursor_at_the_visible_end() {
    let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
    for len in [MAX_PASTE_BYTES - 2, MAX_PASTE_BYTES] {
        let mut app = App::test_app();
        app.picker = Some(Picker::new(Kind::History));
        app.paste(&"a".repeat(len));
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let cursor = terminal.get_cursor_position().unwrap();
        // At 60 columns, the centered picker spans columns 2..58.
        assert_eq!(cursor.x, 56);
    }
}

#[test]
fn long_activity_labels_fit_the_status_above_chat_input() {
    let mut app = App::test_app();
    app.focus = Focus::Graph;
    app.view.activity = Some("a".repeat(MAX_PASTE_BYTES - 1));
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let rendered: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(!rendered.contains(" now "));
    assert!(rendered.contains(&"a".repeat(30)));
    let header: String = terminal.backend().buffer().content[..120]
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(!header.contains("aaaa"));
    let input_top = (0..30)
        .rev()
        .find(|&y| {
            (0..120)
                .map(|x| terminal.backend().buffer()[(x, y)].symbol())
                .collect::<String>()
                .contains(" message ")
        })
        .unwrap();
    let status: String = (0..120)
        .map(|x| terminal.backend().buffer()[(x, input_top - 1)].symbol())
        .collect();
    assert!(status.contains(&"a".repeat(30)));
}

#[test]
fn tui_input_buffer_unicode_character_boundary_safety() {
    let mut buf = InputBuffer::new();
    assert!(buf.is_empty());
    assert_eq!(buf.len(), 0);

    // Polish diacritics (multi-byte UTF-8: 2 bytes each)
    buf.insert_str("abąćęł");
    assert_eq!(buf.text, "abąćęł");
    assert_eq!(buf.cursor, "abąćęł".len());

    // Backspace on multi-byte char removes exact code point
    assert!(buf.backspace());
    assert_eq!(buf.text, "abąćę");
    assert_eq!(buf.cursor, "abąćę".len());

    // Insert 4-byte emoji
    buf.insert_char('🦀');
    assert_eq!(buf.text, "abąćę🦀");
    assert_eq!(buf.cursor, "abąćę🦀".len());

    // Move left past emoji (4 bytes)
    buf.move_left();
    assert_eq!(buf.cursor, "abąćę".len());

    // Delete forward deletes emoji
    assert!(buf.delete());
    assert_eq!(buf.text, "abąćę");

    // Home / End
    buf.move_home();
    assert_eq!(buf.cursor, 0);
    buf.move_end();
    assert_eq!(buf.cursor, buf.len());

    // Word navigation and deletion
    buf.clear();
    buf.insert_str("first second third");
    buf.move_word_left();
    assert_eq!(buf.cursor, "first second ".len());
    buf.delete_word_back();
    assert_eq!(buf.text, "first third");
    assert_eq!(buf.cursor, "first ".len());

    buf.delete_to_start();
    assert_eq!(buf.text, "third");
    assert_eq!(buf.cursor, 0);

    buf.move_end();
    buf.insert_str(" i czwarte");
    buf.move_home();
    buf.move_right(); // on 'r'
    buf.delete_to_end();
    assert_eq!(buf.text, "t");
}

#[test]
fn tui_visual_lines_and_cjk_double_width() {
    // CJK character "漢" has width 2, ASCII "a" has width 1
    let text = "a漢b";
    let lines = compute_visual_lines(text, 10);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].width, 4); // 1 + 2 + 1 = 4 visual columns

    // Wrapping on width limit
    let long_text = "the first line of text that should wrap onto the next line";
    let wrapped = compute_visual_lines(long_text, 20);
    assert!(wrapped.len() > 1);
    for line in &wrapped {
        assert!(line.width <= 20);
    }
}

#[test]
fn tui_paste_sanitization_preserves_full_unicode_text() {
    // Normalization of CRLF and CR
    let raw = "line 1\r\nline 2\rline 3";
    let sanitized = sanitize_paste(raw);
    assert_eq!(sanitized, "line 1\nline 2\nline 3");

    // Single-line paste flattens newlines to space and trims
    let single = sanitize_single_line_paste("  command\r\n argument  \n");
    assert_eq!(single, "command  argument");

    // Large Unicode pastes remain intact.
    let unit = "abąćęł cńódź efżó ";
    let repeat_count = (MAX_PASTE_BYTES / unit.len()) + 10;
    let huge = unit.repeat(repeat_count);
    assert!(huge.len() > MAX_PASTE_BYTES);

    let capped = sanitize_paste(&huge);
    assert_eq!(capped, huge);
    assert!(capped.is_char_boundary(capped.len()));
}

#[test]
fn tui_responsive_breakpoints_and_terminal_guard() {
    assert_eq!(MIN_TERM_WIDTH, 12);
    assert_eq!(MIN_TERM_HEIGHT, 5);
    assert_eq!(SIDE_BY_SIDE_BREAKPOINT, 85);

    // Below 12x5 guard
    let backend_small = TestBackend::new(11, 4);
    let mut term_small = Terminal::new(backend_small).unwrap();
    let mut triggered = false;
    term_small
        .draw(|f| {
            triggered = draw_min_term_guard(f, f.area());
        })
        .unwrap();
    assert!(triggered);
    let buf = term_small.backend().buffer();
    let rendered: String = buf.content.iter().map(|c| c.symbol()).collect();
    let normalized = rendered.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(normalized.contains("Terminal too small (min. 12x5 required)"));

    // At 12x5 minimum guard passes
    let backend_ok = TestBackend::new(12, 5);
    let mut term_ok = Terminal::new(backend_ok).unwrap();
    let mut triggered_ok = false;
    term_ok
        .draw(|f| {
            triggered_ok = draw_min_term_guard(f, f.area());
        })
        .unwrap();
    assert!(!triggered_ok);

    // Breakpoint at 84 (<85) -> compact banner
    let header_compact = render_header(
        "projekt",
        "conv-test",
        Some(true),
        "ready",
        PanelFocus::Chat,
        84,
    );
    let text_compact: String = header_compact
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect();
    assert!(text_compact.contains("[Chat]"));
    assert!(text_compact.contains("Team"));
    assert!(text_compact.contains("(Tab: switch)"));

    // Breakpoint at 85 (>=85) -> side by side mode
    let header_wide = render_header(
        "projekt",
        "conv-test",
        Some(true),
        "ready",
        PanelFocus::Chat,
        85,
    );
    let text_wide: String = header_wide
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect();
    assert!(!text_wide.contains("[Chat]"));
    assert!(!text_wide.contains("(Tab: switch)"));
    assert!(text_wide.contains("projekt"));
    assert!(text_wide.contains("autonomous"));

    // Empty state
    let empty_lines = transcript_empty_state(80);
    assert_eq!(
        empty_lines[0].spans[0].content.as_ref(),
        "No messages in this conversation."
    );

    // Prioritized footer hints
    let hint_120 = main_footer_hint(PanelFocus::Chat, false, false, None, 120);
    assert!(hint_120.contains("Alt+Enter newline"));
    let hint_75 = main_footer_hint(PanelFocus::Chat, false, false, None, 75);
    assert!(hint_75.contains("Ctrl+P"));
    let hint_50 = main_footer_hint(PanelFocus::Chat, false, false, None, 50);
    assert!(!hint_50.contains("Ctrl+P"));
    assert!(hint_50.contains("Tab team"));
}

#[test]
fn app_interaction_sequences_editing_and_paste() {
    let mut app = App::test_app();
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    // 1. Synthetic character entry with Unicode letter test vectors
    for ch in "abąćęł cńódź efżó".chars() {
        app.key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    assert_eq!(app.input.text, "abąćęł cńódź efżó");
    assert_eq!(app.input.cursor, "abąćęł cńódź efżó".len());

    // 2. Word jump and deletion
    app.key(KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL));
    assert_eq!(app.input.cursor, "abąćęł cńódź ".len());

    app.key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
    assert_eq!(app.input.text, "abąćęł efżó");

    // 3. Clear line to start and end
    app.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    assert_eq!(app.input.text, "efżó");
    assert_eq!(app.input.cursor, 0);

    app.key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL));
    assert_eq!(app.input.cursor, "efżó".len());

    // 4. Multiline editing via Alt+Enter and Ctrl+J
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
    for ch in "second line".chars() {
        app.key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    app.key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
    for ch in "third line".chars() {
        app.key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    assert_eq!(app.input.text, "efżó\nsecond line\nthird line");

    // Multiline vertical cursor navigation
    app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    let (_, row) = app.input.visual_cursor(app.input_width.max(1));
    assert_eq!(row, 1, "Cursor should move up to line 1");

    // 5. Bracketed paste with newline normalization
    app.input.clear();
    app.paste("wklejka 1\r\nwklejka 2\r");
    assert_eq!(app.input.text, "wklejka 1\nwklejka 2\n");

    // 6. Large paste remains intact without a truncation notice
    let big = "a".repeat(MAX_PASTE_BYTES + 100);
    app.paste(&big);
    assert!(app.input.text.ends_with(&big));
    assert_ne!(
        app.notice.as_deref(),
        Some("Pasted text truncated to 64 KB")
    );

    // 7. Verify visual rendering with TestBackend
    terminal
        .draw(|f| {
            app.draw(f);
        })
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(
        text.contains("aaaa"),
        "the full paste remains visible in the input"
    );
    assert!(!text.contains("truncated to"));
}

#[test]
fn f1_help_follows_the_active_view_and_preserves_navigation() {
    for help in [
        Help::Chat,
        Help::Graph,
        Help::Details,
        Help::Diff,
        Help::Comment,
        Help::Picker,
    ] {
        let mut app = App::test_app();
        app.input = InputBuffer::with_text("chat draft?");
        if help != Help::Chat {
            app.focus = Focus::Graph;
            app.selected = "worker:t1".into();
        }
        if matches!(
            help,
            Help::Details | Help::Diff | Help::Comment | Help::Picker
        ) {
            let mut details = Details::new("worker:t1".into());
            details.tab = 2;
            details.scroll = 3;
            app.details = Some(details);
        }
        if matches!(help, Help::Diff | Help::Comment | Help::Picker) {
            app.diff = Some(DiffView::new("t1".into()));
        }
        if matches!(help, Help::Comment | Help::Picker) {
            app.editor = Some(Editor {
                target: "diff:t1:src/main.rs:3".into(),
                input: InputBuffer::with_text("comment draft?"),
            });
        }
        if help == Help::Picker {
            let mut picker = Picker::new(Kind::Commands);
            picker.query = "mode".into();
            picker.selected = 1;
            app.picker = Some(picker);
        }

        let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let details_scroll = app.details.as_ref().map(|details| details.scroll);
        let diff_target = app.diff.as_ref().and_then(DiffView::target);
        assert!(app.key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE)));
        assert_eq!(app.help, Some(help));
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert!(!terminal.backend().cursor_visible());
        assert!(app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        assert!(app.help.is_none());
        assert_eq!(app.input.text, "chat draft?");
        assert_eq!(
            app.diff.is_some(),
            matches!(help, Help::Diff | Help::Comment | Help::Picker)
        );
        if let Some(details) = &app.details {
            assert_eq!(details.tab, 2);
            assert_eq!(Some(details.scroll), details_scroll);
        }
        assert_eq!(app.diff.as_ref().and_then(DiffView::target), diff_target);
        if let Some(editor) = &app.editor {
            assert_eq!(editor.target, "diff:t1:src/main.rs:3");
            assert_eq!(editor.input.text, "comment draft?");
        }
        if let Some(picker) = &app.picker {
            assert_eq!(picker.query, "mode");
            assert_eq!(picker.selected, 1);
        }
    }
}

#[test]
fn help_blocks_typing_and_action_shortcuts_until_explicitly_dismissed() {
    let mut app = App::test_app();
    app.input = InputBuffer::with_text("draft");
    for dismiss in [KeyCode::Esc, KeyCode::F(1), KeyCode::Char('?')] {
        app.key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE));
        for ch in "accidentally typed text".chars() {
            app.key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        for ch in ['a', 'r', 'x', 'n', 'p', 'f', 'u'] {
            app.key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL));
        }
        for code in [KeyCode::Enter, KeyCode::Tab, KeyCode::Backspace] {
            app.key(KeyEvent::new(code, KeyModifiers::NONE));
        }
        app.paste("pasted text");
        assert_eq!(app.help, Some(Help::Chat));
        assert_eq!(app.input.text, "draft");
        assert_eq!(app.focus, Focus::Chat);
        assert!(app.picker.is_none());

        app.key(KeyEvent::new(dismiss, KeyModifiers::NONE));
        assert!(app.help.is_none());
        assert_eq!(app.help_scroll, 0);
        app.key(KeyEvent::new(KeyCode::Char('!'), KeyModifiers::NONE));
        assert_eq!(app.input.text, "draft!");
        app.input.backspace();
    }
}

#[test]
fn question_marks_remain_text_in_chat_comments_and_search() {
    let mut app = App::test_app();
    let question_mark = KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE);
    app.key(question_mark);
    assert_eq!(app.input.text, "?");
    assert!(app.help.is_none());

    app.editor = Some(Editor {
        target: "worker:t1".into(),
        input: InputBuffer::new(),
    });
    app.key(question_mark);
    assert_eq!(app.editor.as_ref().unwrap().input.text, "?");
    assert!(app.help.is_none());
    app.picker = Some(Picker::new(Kind::History));
    app.key(question_mark);
    assert_eq!(app.picker.as_ref().unwrap().query, "?");
    assert!(app.help.is_none());
}

#[test]
fn help_hides_the_picker_cursor_until_dismissed() {
    let mut app = App::test_app();
    let mut picker = Picker::new(Kind::History);
    picker.query = "saved query?".into();
    app.picker = Some(picker);
    let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(terminal.backend().cursor_visible());

    app.key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE));
    assert_eq!(app.help, Some(Help::Picker));
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(!terminal.backend().cursor_visible());
    assert!(app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(terminal.backend().cursor_visible());
    assert_eq!(app.picker.as_ref().unwrap().query, "saved query?");
}

#[test]
fn app_modal_ownership_and_isolation() {
    let mut app = App::test_app();
    app.terminal_area = Rect::new(0, 0, 60, 12);
    app.input = InputBuffer::with_text("draft to preserve");

    // 1. Help modal key ownership and navigation
    app.show_help(Help::Chat);
    assert!(app.help.is_some());

    // Typing inside help does NOT alter chat draft
    for ch in "typing in help".chars() {
        app.key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        assert_eq!(app.help, Some(Help::Chat));
    }
    assert_eq!(app.input.text, "draft to preserve");

    // Help navigation keys at 60x12
    app.show_help(Help::Chat);
    let max_scroll = app.help_max_scroll();
    assert!(max_scroll > 0);

    // End clamps to max_scroll
    app.key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
    assert_eq!(app.help_scroll, max_scroll);

    // Down and PageDown at bottom stay clamped without overflow
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.help_scroll, max_scroll);
    app.key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
    assert_eq!(app.help_scroll, max_scroll);

    // Up immediately moves up
    app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.help_scroll, max_scroll - 1);

    // Paste in help is ignored
    app.paste("ignore paste in help");
    assert_eq!(app.input.text, "draft to preserve");

    // Esc closes help and preserves draft
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.help.is_none());
    assert_eq!(app.input.text, "draft to preserve");

    // 2. Command picker modal ownership
    app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    assert!(app.picker.is_some());

    // Typing routes to picker query, draft untouched
    app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE));
    assert_eq!(app.picker.as_ref().unwrap().query, "pl");
    assert_eq!(app.input.text, "draft to preserve");

    // Paste into picker is sanitized to single line
    app.paste("an\r\n");
    assert_eq!(app.picker.as_ref().unwrap().query, "plan");
    assert_eq!(app.input.text, "draft to preserve");

    // Esc dismisses picker
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.picker.is_none());
    assert_eq!(app.input.text, "draft to preserve");

    // 3. Comment editor modal ownership
    app.editor = Some(Editor {
        target: "worker:t1".into(),
        input: InputBuffer::new(),
    });
    assert!(app.editor.is_some());

    app.key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE));
    assert_eq!(app.editor.as_ref().unwrap().input.text, "ko");
    assert_eq!(app.input.text, "draft to preserve");

    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.editor.is_none());
    assert_eq!(app.input.text, "draft to preserve");
}

#[test]
fn app_fake_request_capture_through_app() {
    let dir = tempfile::tempdir().unwrap();
    let sock_path = socket(dir.path());

    let listener = UnixListener::bind(&sock_path).unwrap();
    let (tx, rx) = mpsc::channel::<Value>();

    let handle = thread::spawn(move || {
        for _ in 0..10 {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let parsed: Value = serde_json::from_str(&line).unwrap();
            tx.send(parsed).unwrap();

            let mut writer = stream;
            let _ = writer.write_all(b"{\"data\":null,\"error\":null}\n");
            let _ = writer.flush();
        }
    });

    let client = Client::at(dir.path());
    let (mut app, rx_updates) = App::test_app_with_client(client);
    let pump_until = |app: &mut App, predicate: fn(&antislop::lean::tui::Update) -> bool| {
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            if let Ok(update) = rx_updates.recv_timeout(Duration::from_millis(50)) {
                let matched = predicate(&update);
                app.apply_update(update);
                if matched {
                    return;
                }
            }
        }
        panic!("pump_until timed out waiting for matching update");
    };

    // 1. Chat Send request via Enter key
    app.input = InputBuffer::with_text("message to the orchestrator");
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(req["op"], "send");
    assert_eq!(req["conversation"], "test-conv");
    assert_eq!(req["text"], "message to the orchestrator");
    assert!(app.input.is_empty(), "Send must clear chat draft");
    pump_until(&mut app, |u| {
        matches!(u, antislop::lean::tui::Update::SendResult { .. })
    });

    // 2. Canonical /mode autonomous and supervised
    app.command("/mode autonomous");
    let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(req["op"], "mode");
    assert_eq!(req["conversation"], "test-conv");
    assert_eq!(req["autonomous"], true);

    app.command("/mode supervised");
    let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(req["op"], "mode");
    assert_eq!(req["conversation"], "test-conv");
    assert_eq!(req["autonomous"], false);

    // 3. Pause and continue
    app.command("/pause");
    let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(req["op"], "pause");
    assert_eq!(req["paused"], true);

    app.command("/continue");
    let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(req["op"], "pause");
    assert_eq!(req["paused"], false);

    // 4. Cancel
    app.command("/cancel");
    let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(req["op"], "cancel");
    assert_eq!(req["conversation"], "test-conv");

    // 5. Plan accept
    app.view.apply(&json!({
        "seq": 1,
        "kind": "plan_proposed",
        "payload": {
            "plan": "plan-xyz",
            "tasks": [
                {"id": "t1", "title": "Kompilacja", "writes": true}
            ]
        }
    }));
    app.command("/accept");
    let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(req["op"], "accept");
    assert_eq!(req["conversation"], "test-conv");
    assert_eq!(req["plan"], "plan-xyz");

    // 6. Plan resume
    app.view.apply(&json!({
        "seq": 2,
        "kind": "task",
        "payload": {
            "id": "t1",
            "status": "failed"
        }
    }));
    app.command("/resume");
    let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(req["op"], "resume");
    assert_eq!(req["conversation"], "test-conv");
    assert_eq!(req["plan"], "plan-xyz");

    // 7. Comment target from Graph
    app.focus = Focus::Graph;
    app.selected = "worker:t1".into();
    app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
    assert!(app.editor.is_some());
    assert_eq!(app.editor.as_ref().unwrap().target, "worker:t1");

    app.editor
        .as_mut()
        .unwrap()
        .input
        .insert_str("Comment on t1");
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(req["op"], "comment");
    assert_eq!(req["conversation"], "test-conv");
    assert_eq!(req["target"], "worker:t1");
    assert_eq!(req["text"], "Comment on t1");
    assert!(app.editor.is_none());
    pump_until(&mut app, |u| {
        matches!(u, antislop::lean::tui::Update::CommentResult { .. })
    });

    // 8. Comment target from Details
    app.details = Some(Details::new("worker:t2".into()));
    app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
    assert!(app.editor.is_some());
    assert_eq!(app.editor.as_ref().unwrap().target, "worker:t2");

    app.editor
        .as_mut()
        .unwrap()
        .input
        .insert_str("Comment in t2 details");
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(req["op"], "comment");
    assert_eq!(req["conversation"], "test-conv");
    assert_eq!(req["target"], "worker:t2");
    assert_eq!(req["text"], "Comment in t2 details");
    assert!(app.editor.is_none());
    pump_until(&mut app, |u| {
        matches!(u, antislop::lean::tui::Update::CommentResult { .. })
    });

    handle.join().unwrap();
}

#[test]
fn app_graph_details_diff_navigation_and_search() {
    let mut app = App::test_app();

    app.view.apply(&json!({
        "seq": 1,
        "kind": "plan",
        "payload": {
            "id": "p1",
            "status": "in_progress",
            "title": "Build",
            "tasks": [
                {"id": "t1", "title": "Backend", "status": "done", "dependencies": []},
                {"id": "t2", "title": "Frontend", "status": "in_progress", "dependencies": ["t1"]}
            ]
        }
    }));

    // Tab toggles between Chat and Graph
    assert_eq!(app.focus, Focus::Chat);
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.focus, Focus::Graph);
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.focus, Focus::Chat);
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.focus, Focus::Graph);

    // Filter cycling in Graph
    assert_eq!(app.filter, Filter::All);
    app.key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
    assert_eq!(app.filter, Filter::Depends);
    app.key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
    assert_eq!(app.filter, Filter::Verifies);
    app.key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
    assert_eq!(app.filter, Filter::Mail);
    app.key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
    assert_eq!(app.filter, Filter::Supervises);
    app.key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
    assert_eq!(app.filter, Filter::All);

    // Arrow navigation in Graph
    app.key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));

    // Open Details on Enter
    app.selected = "worker:t1".into();
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.details.is_some());
    assert_eq!(app.details.as_ref().unwrap().key, "worker:t1");

    // Details tab navigation
    let tab0 = app.details.as_ref().unwrap().tab;
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_ne!(app.details.as_ref().unwrap().tab, tab0);
    app.key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
    assert_eq!(app.details.as_ref().unwrap().tab, tab0);

    // Open Diff from Details
    app.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
    assert!(app.diff.is_some());
    assert_eq!(app.diff.as_ref().unwrap().task, "t1");

    // Diff tab navigation
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));

    // Esc from Diff returns to Details
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.diff.is_none());
    assert!(app.details.is_some());

    // Esc from Details returns to Graph
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.details.is_none());
    assert_eq!(app.focus, Focus::Graph);

    // Search in conversation history
    app.view.apply(&json!({
        "seq": 10,
        "kind": "message",
        "payload": {
            "author": "orchestrator",
            "text": "Wykryto problem ze schematem"
        }
    }));
    app.key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL));
    assert!(app.picker.is_some());
    assert_eq!(app.picker.as_ref().unwrap().kind, Kind::History);

    // Type query and enter
    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    assert!(app.picker.is_none());
    assert_eq!(app.focus, Focus::Chat);
    assert_eq!(app.scroll, usize::MAX);
    assert!(app.found.is_some());
}

#[test]
fn app_incoming_updates_and_rendering() {
    let mut app = App::test_app();
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    // 1. Initial empty render
    terminal
        .draw(|f| {
            app.draw(f);
        })
        .unwrap();
    let text1: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(text1.contains("No messages in this conversation."));

    // 2. Incoming updates: message, task, notice
    app.view.apply(&json!({
        "seq": 1,
        "kind": "message",
        "payload": {
            "author": "orchestrator",
            "text": "Welcome to the test project"
        }
    }));
    app.notice = Some("Step 1 completed successfully".into());

    terminal
        .draw(|f| {
            app.draw(f);
        })
        .unwrap();
    let text2: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(text2.contains("Welcome to the test project"));
    assert!(text2.contains("Step 1 completed successfully"));
}

#[test]
fn app_resizing_cursor_bounds_picker_overlay_ascii_no_color() {
    let mut app = App::test_app();
    app.view.autonomous = Some(true);

    // 1. Resizing across breakpoints
    // 120x30 (wide side-by-side)
    let mut term_wide = Terminal::new(TestBackend::new(120, 30)).unwrap();
    term_wide
        .draw(|f| {
            app.draw(f);
        })
        .unwrap();
    let text_wide: String = term_wide
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(text_wide.contains("autonomous"));
    assert!(!text_wide.contains("[Chat]"));

    // 80x24 (compact stacked)
    let mut term_compact = Terminal::new(TestBackend::new(80, 24)).unwrap();
    term_compact
        .draw(|f| {
            app.draw(f);
        })
        .unwrap();
    let text_compact: String = term_compact
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(text_compact.contains("[Chat]"));

    // 11x4 (below minimum 12x5)
    let mut term_tiny = Terminal::new(TestBackend::new(11, 4)).unwrap();
    term_tiny
        .draw(|f| {
            app.draw(f);
        })
        .unwrap();
    let text_tiny: String = term_tiny
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    let text_tiny_norm = text_tiny.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(text_tiny_norm.contains("Terminal too small (min. 12x5 required)"));

    // 2. Cursor bounds at 60x12
    let mut term_min = Terminal::new(TestBackend::new(60, 12)).unwrap();
    app.input = InputBuffer::with_text("x".repeat(120));
    term_min
        .draw(|f| {
            app.draw(f);
        })
        .unwrap();
    let (cx, cy) = term_min.get_cursor_position().unwrap().into();
    assert!(
        (1..=58).contains(&cx),
        "Chat cursor x ({cx}) must stay inside inner border"
    );
    assert!(
        (1..=10).contains(&cy),
        "Chat cursor y ({cy}) must stay inside inner border"
    );

    // Comment editor cursor bounds
    app.editor = Some(Editor {
        target: "worker:t1".into(),
        input: InputBuffer::with_text("y".repeat(120)),
    });
    term_min
        .draw(|f| {
            app.draw(f);
        })
        .unwrap();
    let (ecx, ecy) = term_min.get_cursor_position().unwrap().into();
    assert!(
        (1..=58).contains(&ecx),
        "Editor cursor x ({ecx}) must stay inside inner border"
    );
    assert!(
        (1..=10).contains(&ecy),
        "Editor cursor y ({ecy}) must stay inside inner border"
    );
    app.editor = None;

    // 3. Picker visibility and reversed selection at 60x12
    let mut picker = Picker::new(Kind::Commands);
    let items_len = picker.items(&app.view).len();
    assert!(items_len >= 10);
    picker.selected = items_len - 1;
    app.picker = Some(picker);

    term_min
        .draw(|f| {
            app.draw(f);
        })
        .unwrap();
    let mut found_reversed = false;
    for y in 0..12 {
        for x in 0..60 {
            if term_min.backend().buffer()[(x, y)]
                .modifier
                .contains(Modifier::REVERSED)
            {
                found_reversed = true;
                break;
            }
        }
    }
    assert!(
        found_reversed,
        "Selected picker item must be visible and reversed at 60x12"
    );
    app.picker = None;

    // 4. The agent canvas and its progress overlay remain visible before tasks run
    app.view.activity = Some("compiling tests".into());
    let mut term_overlay = Terminal::new(TestBackend::new(100, 30)).unwrap();
    term_overlay
        .draw(|f| {
            app.draw(f);
        })
        .unwrap();
    let text_overlay: String = term_overlay
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(text_overlay.contains("team · filter:"));
    assert!(text_overlay.contains(" now "));
    assert!(text_overlay.contains("compiling tests"));

    // 5. ASCII conversion
    let mut buffer = Buffer::empty(Rect::new(0, 0, 15, 1));
    buffer.set_string(0, 0, "╭─✓ ąębć 漢", Style::default());
    in_ascii(&mut buffer);
    let text_ascii: String = buffer.content.iter().map(|c| c.symbol()).collect();
    assert_eq!(text_ascii, "+-+ aebc ?     ");

    // 6. NO_COLOR preservation of DIM and REVERSED
    let mut buffer_nc = Buffer::empty(Rect::new(0, 0, 3, 1));
    buffer_nc.set_string(0, 0, "a", Style::default().fg(Color::DarkGray));
    buffer_nc.set_string(1, 0, "b", Style::default().fg(Color::Red).bg(Color::Blue));
    buffer_nc.set_string(2, 0, "c", Style::default().add_modifier(Modifier::REVERSED));
    without_color(&mut buffer_nc);
    assert!(
        buffer_nc
            .content
            .iter()
            .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset)
    );
    assert!(buffer_nc.content[0].modifier.contains(Modifier::DIM));
    assert!(buffer_nc.content[2].modifier.contains(Modifier::REVERSED));
}

#[test]
fn tui_isolated_fake_request_capture() {
    let dir = tempfile::tempdir().unwrap();
    let sock_path = socket(dir.path());

    let listener = UnixListener::bind(&sock_path).unwrap();
    let (tx, rx) = mpsc::channel::<Value>();

    let handle = thread::spawn(move || {
        for _ in 0..7 {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let parsed: Value = serde_json::from_str(&line).unwrap();
            tx.send(parsed).unwrap();

            // Reply with dummy JSON line
            let mut writer = stream;
            let _ = writer.write_all(b"{\"ok\":true}\n");
            let _ = writer.flush();
        }
    });

    let client = Client::at(dir.path());

    // 1. Send
    let res = client.call(&Request::Send {
        conversation: "conv-1".into(),
        text: "hello orchestrator".into(),
    });
    assert!(res.is_ok());
    let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(req["op"], "send");
    assert_eq!(req["conversation"], "conv-1");
    assert_eq!(req["text"], "hello orchestrator");

    // 2. Comment
    let res = client.call(&Request::Comment {
        conversation: "conv-1".into(),
        target: "worker:t1".into(),
        text: "code feedback".into(),
    });
    assert!(res.is_ok());
    let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(req["op"], "comment");
    assert_eq!(req["target"], "worker:t1");
    assert_eq!(req["text"], "code feedback");

    // 3. Accept plan
    let res = client.call(&Request::Accept {
        conversation: "conv-1".into(),
        plan: "p-100".into(),
    });
    assert!(res.is_ok());
    let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(req["op"], "accept");
    assert_eq!(req["plan"], "p-100");

    // 4. Resume plan
    let res = client.call(&Request::Resume {
        conversation: "conv-1".into(),
        plan: "p-100".into(),
    });
    assert!(res.is_ok());
    let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(req["op"], "resume");
    assert_eq!(req["plan"], "p-100");

    // 5. Mode change (canonical autonomous)
    let res = client.call(&Request::Mode {
        conversation: "conv-1".into(),
        autonomous: true,
    });
    assert!(res.is_ok());
    let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(req["op"], "mode");
    assert_eq!(req["autonomous"], true);

    // 6. Pause
    let res = client.call(&Request::Pause {
        conversation: "conv-1".into(),
        paused: true,
    });
    assert!(res.is_ok());
    let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(req["op"], "pause");
    assert_eq!(req["paused"], true);

    // 7. Cancel
    let res = client.call(&Request::Cancel {
        conversation: "conv-1".into(),
    });
    assert!(res.is_ok());
    let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(req["op"], "cancel");
    assert_eq!(req["conversation"], "conv-1");

    handle.join().unwrap();
}

#[test]
fn tui_grapheme_cluster_editing_and_wrapping_parity() {
    let mut buf = InputBuffer::new();

    // 1. ZWJ emoji sequences: woman technologist (10 bytes), family (25 bytes)
    let tech_woman = "👩‍💻"; // 10 bytes: \u{1F469}\u{200D}\u{1F4BB}
    let family = "👨‍👩‍👧‍👦"; // 25 bytes

    buf.insert_str("Hello ");
    buf.insert_str(tech_woman);
    buf.insert_str(" and ");
    buf.insert_str(family);
    buf.insert_str("!");

    assert_eq!(buf.text, format!("Hello {} and {}!", tech_woman, family));
    let end_pos = buf.cursor;

    // Moving left past '!'
    buf.move_left();
    assert_eq!(buf.cursor, end_pos - 1);

    // Moving left past family emoji moves back entire 25 bytes at once!
    buf.move_left();
    assert_eq!(buf.cursor, end_pos - 1 - family.len());

    // Moving right past family emoji moves forward entire 25 bytes at once!
    buf.move_right();
    assert_eq!(buf.cursor, end_pos - 1);

    // Backspace on family emoji deletes entire 25-byte cluster atomically
    assert!(buf.backspace());
    assert!(!buf.text.contains(family));
    assert_eq!(buf.text, format!("Hello {} and !", tech_woman));

    // Delete tech_woman atomically
    buf.cursor = "Hello ".len();
    assert!(buf.delete());
    assert!(!buf.text.contains(tech_woman));
    assert_eq!(buf.text, "Hello  and !");

    // 2. Combining character sequence: e + acute accent
    let mut buf2 = InputBuffer::with_text("e\u{0301}clair");
    assert_eq!(buf2.cursor, 8); // 3 bytes for e\u{0301} + 5 bytes for clair
    buf2.cursor = 3;
    buf2.move_left();
    assert_eq!(buf2.cursor, 0); // moves before combining cluster
    buf2.move_right();
    assert_eq!(buf2.cursor, 3); // moves past combining cluster
    assert!(buf2.backspace());
    assert_eq!(buf2.text, "clair");
    assert_eq!(buf2.cursor, 0);

    // 3. Wrapping of grapheme clusters:
    // With width 7: "abc " (4 cols) + "👩‍💻 " (3 cols) = 7 cols (fits on line 1)
    let lines = compute_visual_lines("abc 👩‍💻 def", 7);
    assert_eq!(lines.len(), 2);
    let text_lines = visual_text_lines("abc 👩‍💻 def", 7);
    assert_eq!(text_lines[0], "abc 👩‍💻 ");
    assert_eq!(text_lines[1], "def");

    // Now width 6: "abc " (4 cols) + "👩‍💻 " (3 cols) = 7 cols > 6, so "👩‍💻 " wraps to line 2 atomically, and "def" fits on line 2 (3 + 3 = 6 <= 6)
    let lines2 = compute_visual_lines("abc 👩‍💻 def", 6);
    assert_eq!(lines2.len(), 2);
    let text_lines2 = visual_text_lines("abc 👩‍💻 def", 6);
    assert_eq!(text_lines2[0], "abc ");
    assert_eq!(text_lines2[1], "👩‍💻 def");
}

#[test]
fn tui_sent_message_history_ring_and_draft_stashing() {
    let mut app = App::test_app();
    assert!(app.history.is_empty());

    // Record 3 distinct messages into history
    app.record_history("first message");
    app.record_history("second message");
    app.record_history("second message"); // duplicate ignored
    app.record_history("third message");

    assert_eq!(app.history.len(), 3);
    assert_eq!(app.history[0], "first message");
    assert_eq!(app.history[1], "second message");
    assert_eq!(app.history[2], "third message");

    // Start editing a fresh draft
    app.input = InputBuffer::with_text("my draft in progress");
    assert_eq!(app.history_index, None);

    // Press Up on first row -> stashes draft and recalls newest message (index 2)
    app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.history_index, Some(2));
    assert_eq!(app.input.text, "third message");
    assert_eq!(app.history_draft, "my draft in progress");

    // Press Up -> recalls index 1
    app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.history_index, Some(1));
    assert_eq!(app.input.text, "second message");

    // Press Up -> recalls index 0
    app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.history_index, Some(0));
    assert_eq!(app.input.text, "first message");

    // Press Up at oldest -> stays at index 0 without overflow
    app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.history_index, Some(0));
    assert_eq!(app.input.text, "first message");

    // Press Down -> recalls index 1
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.history_index, Some(1));
    assert_eq!(app.input.text, "second message");

    // Press Down -> recalls index 2
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.history_index, Some(2));
    assert_eq!(app.input.text, "third message");

    // Press Down past newest entry -> restores stashed draft!
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.history_index, None);
    assert_eq!(app.input.text, "my draft in progress");

    // Now test Esc while browsing history
    app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.history_index, Some(2));
    assert_eq!(app.input.text, "third message");

    // Esc immediately cancels history recall and restores draft
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.history_index, None);
    assert_eq!(app.input.text, "my draft in progress");
}

#[test]
fn test_esc_behavior_and_cleared_draft_counterexample() {
    let mut app = App::test_app();
    app.history = vec!["previous message".into()];

    // 1. Esc during history recall cancels history and restores draft
    app.input = InputBuffer::with_text("draft in progress");
    app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.history_index, Some(0));
    assert_eq!(app.input.text, "previous message");
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.history_index, None);
    assert_eq!(app.input.text, "draft in progress");

    // 2. Esc on non-empty draft clears input
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.input.is_empty());

    // 3. Counterexample proof: pressing Up on the empty buffer unconditionally
    // overwrites history_draft with "" (mod.rs:811), so Down does NOT retrieve the draft.
    app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.history_index, Some(0));
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.history_index, None);
    assert!(
        app.input.is_empty(),
        "Draft cleared by Esc is not restored by Up/Down"
    );

    // Esc on an empty draft stays in the app. Explicit quit keys detach.
    assert!(app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
    assert!(!app.key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)));
    assert!(!app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)));
}

#[test]
fn tui_send_reliability_draft_retention_on_error_and_in_flight_typing() {
    let mut app = App::test_app();
    app.conversation = "conv-abc".into();

    // 1. Send chat message
    app.input = InputBuffer::with_text("original message content");
    app.submit();

    assert!(app.sending);
    assert_eq!(
        app.pending_send.as_deref(),
        Some("original message content")
    );
    assert!(
        app.input.is_empty(),
        "Draft is cleared so user can keep typing while in flight"
    );

    // 2. User adds new typing while waiting for the daemon response
    app.input.insert_str("new addition while waiting");

    // 3. Daemon fails with network error
    let req_id = app.send_request_id;
    app.apply_update(Update::SendResult {
        conversation: app.conversation.clone(),
        generation: app.generation,
        request_id: req_id,
        sent_text: "original message content".into(),
        result: Err("connection lost".into()),
    });

    assert!(!app.sending);
    assert_eq!(app.pending_send, None);
    // Crucial requirement: DO NOT automatically glue old message with new draft!
    // Active typing remains pristine:
    assert_eq!(app.input.text, "new addition while waiting");
    // Failed message is preserved in failed_send without concatenating:
    assert_eq!(app.failed_send.as_deref(), Some("original message content"));
    assert_eq!(
        app.notice.as_deref(),
        Some("✗ connection lost (unsent message preserved)")
    );

    // 4. When input was empty upon error, draft is restored directly to input
    app.input.clear();
    app.failed_send = None;
    app.input = InputBuffer::with_text("another message");
    app.submit();
    let req_id_2 = app.send_request_id;
    assert!(app.input.is_empty());
    app.apply_update(Update::SendResult {
        conversation: app.conversation.clone(),
        generation: app.generation,
        request_id: req_id_2,
        sent_text: "another message".into(),
        result: Err("network error".into()),
    });
    assert!(!app.sending);
    assert_eq!(app.input.text, "another message");
    assert_eq!(
        app.notice.as_deref(),
        Some("✗ network error (draft restored)")
    );
}

#[test]
fn tui_comment_send_retention_and_protection_on_error() {
    let mut app = App::test_app();
    app.conversation = "conv-xyz".into();

    // Open comment editor for task t1
    app.editor = Some(Editor {
        target: "worker:t1".into(),
        input: InputBuffer::with_text("important comment draft"),
    });

    // Submit comment
    app.submit_comment();
    assert!(app.sending_comment);
    assert!(app.editor.is_none());
    assert_eq!(
        app.pending_comment.as_ref().map(|e| e.target.as_str()),
        Some("worker:t1")
    );

    // Daemon returns an error
    let editor = app.pending_comment.clone().unwrap();
    let req_id = app.comment_request_id;
    app.apply_update(Update::CommentResult {
        conversation: app.conversation.clone(),
        plan: app.view.plan.clone(),
        generation: app.generation,
        request_id: req_id,
        editor,
        result: Err("database locked".into()),
    });

    assert!(!app.sending_comment);
    assert!(app.pending_comment.is_none());
    // Comment editor is restored with draft intact!
    assert!(app.editor.is_some());
    let restored = app.editor.as_ref().unwrap();
    assert_eq!(restored.target, "worker:t1");
    assert_eq!(restored.input.text, "important comment draft");
    assert_eq!(
        app.notice.as_deref(),
        Some("✗ database locked (comment draft preserved)")
    );

    // User submits comment, but then opens a new editor while send is in flight
    app.submit_comment();
    let req_id_2 = app.comment_request_id;
    let editor_2 = app.pending_comment.clone().unwrap();
    app.editor = Some(Editor {
        target: "worker:t2".into(),
        input: InputBuffer::with_text("inny edytor otwarty"),
    });
    app.apply_update(Update::CommentResult {
        conversation: app.conversation.clone(),
        plan: app.view.plan.clone(),
        generation: app.generation,
        request_id: req_id_2,
        editor: editor_2,
        result: Err("write error".into()),
    });
    assert!(!app.sending_comment);
    // Active editor is NOT replaced or glued
    assert_eq!(app.editor.as_ref().unwrap().target, "worker:t2");
    assert_eq!(
        app.editor.as_ref().unwrap().input.text,
        "inny edytor otwarty"
    );
    // Failed comment was preserved in drafts
    let saved = app.get_comment_draft("worker:t1");
    assert_eq!(saved.text, "important comment draft");
    assert_eq!(
        app.failed_comment.as_ref().map(|e| e.target.as_str()),
        Some("worker:t1")
    );
}

#[test]
fn tui_duplicate_send_blocking_for_chat_and_comments() {
    let mut app = App::test_app();
    app.conversation = "conv-dup".into();

    // 1. Chat duplicate send
    app.input = InputBuffer::with_text("first send");
    app.submit();
    assert!(app.sending);

    // User types more and presses Enter again while sending is in flight
    app.input.insert_str("second attempt");
    app.submit(); // Should be blocked!
    assert_eq!(
        app.notice.as_deref(),
        Some("Sending in progress; draft preserved.")
    );
    assert_eq!(app.input.text, "second attempt");

    // Also verify Ctrl+S is blocked
    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert_eq!(
        app.notice.as_deref(),
        Some("Sending in progress; draft preserved.")
    );

    // 2. Comment duplicate send
    app.sending = false;
    app.editor = Some(Editor {
        target: "task:t1".into(),
        input: InputBuffer::with_text("comment one"),
    });
    app.submit_comment();
    assert!(app.sending_comment);

    // Reopen editor and attempt duplicate submit
    app.editor = Some(Editor {
        target: "task:t1".into(),
        input: InputBuffer::with_text("comment two"),
    });
    app.submit_comment();
    assert_eq!(
        app.notice.as_deref(),
        Some("Sending comment in progress; draft preserved.")
    );
    assert!(
        app.editor.is_some(),
        "Editor draft remains open when blocked"
    );
}

#[test]
fn tui_conversation_switching_preserves_isolated_drafts() {
    let mut app = App::test_app();
    app.open("conv-1".into());

    // Type a draft in conv-1
    app.input = InputBuffer::with_text("draft in conversation 1");

    // Switch to conv-2
    app.open("conv-2".into());
    assert_eq!(app.conversation, "conv-2");
    assert!(app.input.is_empty(), "Fresh conversation has empty draft");

    // Type a draft in conv-2
    app.input = InputBuffer::with_text("draft in conversation 2");

    // Switch back to conv-1
    app.open("conv-1".into());
    assert_eq!(app.conversation, "conv-1");
    assert_eq!(
        app.input.text, "draft in conversation 1",
        "Draft in conv-1 was restored!"
    );

    // Switch back to conv-2
    app.open("conv-2".into());
    assert_eq!(app.conversation, "conv-2");
    assert_eq!(
        app.input.text, "draft in conversation 2",
        "Draft in conv-2 was restored!"
    );
}

#[test]
fn tui_established_keymap_ctrl_s_and_newlines() {
    let mut app = App::test_app();

    // 1. Chat: Alt+Enter and Ctrl+J insert newlines
    app.input = InputBuffer::with_text("line1");
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
    app.input.insert_str("line2");
    app.key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
    app.input.insert_str("line3");
    assert_eq!(app.input.text, "line1\nline2\nline3");

    // 2. Chat: Ctrl+S triggers submit
    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert!(app.sending);
    assert_eq!(app.pending_send.as_deref(), Some("line1\nline2\nline3"));
    assert!(app.input.is_empty());

    // 3. Comment: Alt+Enter and Ctrl+J insert newlines
    app.sending = false;
    app.editor = Some(Editor {
        target: "worker:t1".into(),
        input: InputBuffer::with_text("comment line1"),
    });
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
    app.editor
        .as_mut()
        .unwrap()
        .input
        .insert_str("comment line2");
    app.key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
    app.editor
        .as_mut()
        .unwrap()
        .input
        .insert_str("comment line3");
    assert_eq!(
        app.editor.as_ref().unwrap().input.text,
        "comment line1\ncomment line2\ncomment line3"
    );

    // 4. Comment: Ctrl+S triggers submit_comment
    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert!(app.sending_comment);
    assert_eq!(
        app.pending_comment.as_ref().unwrap().input.text,
        "comment line1\ncomment line2\ncomment line3"
    );
}

#[test]
fn tui_delayed_responses_from_old_conversations_and_older_requests_do_not_alter_active_state() {
    let mut app = App::test_app();

    // 1. In Conv-A: send message and comment, open details and diff
    app.conversation = "conv-A".into();
    app.input = InputBuffer::with_text("message from A");
    app.submit();
    let conv_a_send_id = app.send_request_id;
    assert!(app.sending);

    app.editor = Some(Editor {
        target: "worker:t1".into(),
        input: InputBuffer::with_text("comment from A"),
    });
    app.submit_comment();
    let conv_a_comment_id = app.comment_request_id;
    assert!(app.sending_comment);

    app.details = Some(Details::new("worker:t1".into()));
    let conv_a_task_id = app.task_request_id;
    app.diff = Some(DiffView::new("t1".into()));
    let conv_a_diff_id = app.diff_request_id;

    // 2. Switch to Conv-B
    app.open("conv-B".into());
    assert_eq!(app.conversation, "conv-B");
    assert!(!app.sending);
    assert!(!app.sending_comment);

    // Active state in Conv-B
    app.input = InputBuffer::with_text("active draft in B");
    app.editor = Some(Editor {
        target: "worker:t9".into(),
        input: InputBuffer::with_text("active comment in B"),
    });
    app.details = Some(Details::new("worker:t9".into()));
    app.diff = Some(DiffView::new("t9".into()));

    // 3. Delayed responses from Conv-A arrive while in Conv-B:
    // 3a. SendResult for Conv-A
    app.apply_update(Update::SendResult {
        conversation: "conv-A".into(),
        generation: 1,
        request_id: conv_a_send_id,
        sent_text: "message from A".into(),
        result: Err("send error in the old conversation".into()),
    });
    // Active Conv-B state remains pristine:
    assert_eq!(app.input.text, "active draft in B");
    assert!(!app.sending);
    assert_eq!(app.pending_send, None);
    assert_eq!(app.failed_send, None);
    // Preserved Conv-A state was updated:
    let state_a = app.conversation_states.get("conv-A").unwrap();
    assert!(!state_a.sending);
    assert_eq!(state_a.pending_send, None);
    assert_eq!(state_a.input.text, "message from A");
    assert_eq!(state_a.failed_send, None);

    // 3b. CommentResult for Conv-A
    app.apply_update(Update::CommentResult {
        conversation: "conv-A".into(),
        plan: None,
        generation: 1,
        request_id: conv_a_comment_id,
        editor: Editor {
            target: "worker:t1".into(),
            input: InputBuffer::with_text("comment from A"),
        },
        result: Err("comment error in the old conversation".into()),
    });
    // Active Conv-B comment editor remains pristine:
    assert_eq!(app.editor.as_ref().unwrap().target, "worker:t9");
    assert_eq!(
        app.editor.as_ref().unwrap().input.text,
        "active comment in B"
    );
    assert!(!app.sending_comment);

    // 3c. Task update for Conv-A
    app.apply_update(Update::Task {
        documents: Default::default(),
        conversation: "conv-A".into(),
        plan: None,
        generation: 1,
        request_id: conv_a_task_id,
        key: "worker:t1".into(),
        round: 0,
        task: json!({"id": "t1", "title": "old task"}),
    });
    // Active Conv-B details remains pristine (for worker:t9):
    assert_eq!(app.details.as_ref().unwrap().key, "worker:t9");
    assert!(app.details.as_ref().unwrap().task.is_none());

    // 3d. Diff update for Conv-A
    app.apply_update(Update::Diff {
        conversation: "conv-A".into(),
        plan: None,
        generation: 1,
        request_id: conv_a_diff_id,
        task: "t1".into(),
        reply: Ok(json!([])),
    });
    // Active Conv-B diff remains pristine (for t9):
    assert_eq!(app.diff.as_ref().unwrap().task, "t9");

    // 4. Within the same conversation (Conv-B), older request IDs are ignored:
    app.input.clear();
    app.input = InputBuffer::with_text("request 1 in B");
    app.submit();
    let b_req1 = app.send_request_id;
    // Request 1 succeeds
    app.apply_update(Update::SendResult {
        conversation: "conv-B".into(),
        generation: app.generation,
        request_id: b_req1,
        sent_text: "request 1 in B".into(),
        result: Ok(()),
    });
    assert!(!app.sending);

    // Start request 2
    app.input = InputBuffer::with_text("request 2 in B");
    app.submit();
    let b_req2 = app.send_request_id;
    assert!(b_req2 > b_req1);
    assert!(app.sending);
    assert_eq!(app.pending_send.as_deref(), Some("request 2 in B"));

    // User types new draft while waiting for request 2
    app.input.insert_str("new addition during request 2");

    // Stale/delayed response for request 1 arrives late:
    app.apply_update(Update::SendResult {
        conversation: "conv-B".into(),
        generation: app.generation,
        request_id: b_req1,
        sent_text: "request 1 in B".into(),
        result: Err("delayed error from an old request".into()),
    });
    // Must NOT alter request 2 in flight or overwrite draft:
    assert!(app.sending);
    assert_eq!(app.pending_send.as_deref(), Some("request 2 in B"));
    assert_eq!(app.input.text, "new addition during request 2");
}

#[test]
fn tui_identical_sends_correlation() {
    let mut app = App::test_app();
    app.conversation = "conv-idem".into();

    // 1. Send identical text first time
    app.input = InputBuffer::with_text("identical content");
    app.submit();
    let req1 = app.send_request_id;
    assert!(app.sending);
    assert_eq!(app.pending_send.as_deref(), Some("identical content"));

    // In-flight typing during send 1
    app.input.insert_str("entry while waiting 1");

    // Out-of-order older response with wrong ID arrives:
    app.apply_update(Update::SendResult {
        conversation: "conv-idem".into(),
        generation: app.generation,
        request_id: req1 - 1,
        sent_text: "identical content".into(),
        result: Err("old error".into()),
    });
    assert!(app.sending);
    assert_eq!(app.pending_send.as_deref(), Some("identical content"));
    assert_eq!(app.input.text, "entry while waiting 1");

    // Send 1 fails with network error
    app.apply_update(Update::SendResult {
        conversation: "conv-idem".into(),
        generation: app.generation,
        request_id: req1,
        sent_text: "identical content".into(),
        result: Err("przekroczono limit czasu".into()),
    });
    assert!(!app.sending);
    assert_eq!(app.pending_send, None);
    // Crucial: not glued, active draft kept intact, failed preserved in failed_send
    assert_eq!(app.input.text, "entry while waiting 1");
    assert_eq!(app.failed_send.as_deref(), Some("identical content"));

    // 2. User submits identical text second time
    app.input = InputBuffer::with_text("identical content");
    app.submit();
    let req2 = app.send_request_id;
    assert!(req2 > req1);
    assert!(app.sending);
    assert_eq!(app.pending_send.as_deref(), Some("identical content"));

    // A delayed duplicate response for req1 arrives:
    app.apply_update(Update::SendResult {
        conversation: "conv-idem".into(),
        generation: app.generation,
        request_id: req1,
        sent_text: "identical content".into(),
        result: Ok(()),
    });
    // Request 2 must still be in flight!
    assert!(app.sending);
    assert_eq!(app.pending_send.as_deref(), Some("identical content"));

    // When the real response for req2 arrives:
    app.apply_update(Update::SendResult {
        conversation: "conv-idem".into(),
        generation: app.generation,
        request_id: req2,
        sent_text: "identical content".into(),
        result: Ok(()),
    });
    assert!(!app.sending);
    assert_eq!(app.pending_send, None);
}

#[test]
fn tui_conversation_switching_preserves_full_editor_and_flight_state() {
    let mut app = App::test_app();

    // 1. Set up rich state in Conv-A
    app.open("conv-a".into());
    app.input = InputBuffer::with_text("line 1\nline 2 with a draft");
    app.input.cursor = 12;
    app.input.preferred_col = Some(15);

    app.editor = Some(Editor {
        target: "worker:t1".into(),
        input: InputBuffer::with_text("working comment"),
    });
    app.editor.as_mut().unwrap().input.cursor = 5;
    app.editor.as_mut().unwrap().input.preferred_col = Some(5);

    app.save_comment_draft("worker:t2", InputBuffer::with_text("another draft t2"));

    app.sending = true;
    app.pending_send = Some("content A being sent".into());
    app.send_request_id = 101;

    app.sending_comment = true;
    app.pending_comment = Some(Editor {
        target: "worker:t3".into(),
        input: InputBuffer::with_text("comment t3 being sent"),
    });
    app.comment_request_id = 102;

    app.details = Some(Details::new("worker:t1".into()));
    app.diff = Some(DiffView::new("t1".into()));

    // 2. Switch to Conv-B (A -> B)
    app.open("conv-b".into());
    assert_eq!(app.conversation, "conv-b");
    assert!(app.input.is_empty());
    assert!(app.editor.is_none());
    assert!(!app.sending);
    assert!(!app.sending_comment);
    assert!(app.details.is_none());
    assert!(app.diff.is_none());

    // In Conv-B: user starts a different draft
    app.input = InputBuffer::with_text("conversation B draft");

    // 3. Switch back to Conv-A (B -> A)
    app.open("conv-a".into());
    assert_eq!(app.conversation, "conv-a");
    // Full input buffer state preserved:
    assert_eq!(app.input.text, "line 1\nline 2 with a draft");
    assert_eq!(app.input.cursor, 12);
    assert_eq!(app.input.preferred_col, Some(15));

    // Full comment editor state preserved:
    assert!(app.editor.is_some());
    let ed = app.editor.as_ref().unwrap();
    assert_eq!(ed.target, "worker:t1");
    assert_eq!(ed.input.text, "working comment");
    assert_eq!(ed.input.cursor, 5);
    assert_eq!(ed.input.preferred_col, Some(5));

    // Comment drafts preserved:
    assert_eq!(app.get_comment_draft("worker:t2").text, "another draft t2");

    // In-flight sending state preserved:
    assert!(app.sending);
    assert_eq!(app.pending_send.as_deref(), Some("content A being sent"));
    assert_eq!(app.send_request_id, 101);

    // In-flight comment state preserved:
    assert!(app.sending_comment);
    assert_eq!(
        app.pending_comment.as_ref().map(|e| e.target.as_str()),
        Some("worker:t3")
    );
    assert_eq!(app.comment_request_id, 102);

    // Details and Diff preserved:
    assert_eq!(app.details.as_ref().unwrap().key, "worker:t1");
    assert_eq!(app.diff.as_ref().unwrap().task, "t1");

    // 4. In-flight operations resolve AFTER switching A -> B -> A
    // (Note: generation has incremented twice, but request_id correlates them!)
    assert_ne!(app.generation, 1);

    // 4a. SendResult arrives for request 101 with error
    app.apply_update(Update::SendResult {
        conversation: "conv-a".into(),
        generation: 1, // original generation before switch
        request_id: 101,
        sent_text: "content A being sent".into(),
        result: Err("network error after A->B->A".into()),
    });
    // Flight flag cleared - no permanent lock!
    assert!(!app.sending);
    assert_eq!(app.pending_send, None);
    // Draft retained without automatic gluing:
    assert_eq!(app.input.text, "line 1\nline 2 with a draft");
    assert_eq!(app.failed_send.as_deref(), Some("content A being sent"));
    assert_eq!(
        app.notice.as_deref(),
        Some("✗ network error after A->B->A (unsent message preserved)")
    );

    // 4b. CommentResult arrives for request 102 with error
    app.apply_update(Update::CommentResult {
        conversation: "conv-a".into(),
        plan: None,
        generation: 1,
        request_id: 102,
        editor: Editor {
            target: "worker:t3".into(),
            input: InputBuffer::with_text("comment t3 being sent"),
        },
        result: Err("database error after A->B->A".into()),
    });
    // Comment flight flag cleared:
    assert!(!app.sending_comment);
    assert_eq!(app.pending_comment, None);
    // Active editor for worker:t1 not overwritten or glued:
    assert_eq!(app.editor.as_ref().unwrap().target, "worker:t1");
    assert_eq!(app.editor.as_ref().unwrap().input.text, "working comment");
    // Failed comment for worker:t3 preserved in drafts and failed_comment:
    assert_eq!(
        app.get_comment_draft("worker:t3").text,
        "comment t3 being sent"
    );
    assert_eq!(
        app.failed_comment.as_ref().map(|e| e.target.as_str()),
        Some("worker:t3")
    );

    // 4c. Details and Diff arrive after A -> B -> A
    app.apply_update(Update::Task {
        documents: Default::default(),
        conversation: "conv-a".into(),
        plan: None,
        generation: 1,
        request_id: app.task_request_id,
        key: "worker:t1".into(),
        round: 0,
        task: json!({"id": "t1", "title": "task after A->B->A"}),
    });
    assert_eq!(
        app.details.as_ref().unwrap().task.as_ref().unwrap()["title"],
        "task after A->B->A"
    );

    app.apply_update(Update::Diff {
        conversation: "conv-a".into(),
        plan: None,
        generation: 1,
        request_id: app.diff_request_id,
        task: "t1".into(),
        reply: Ok(json!([])),
    });
    assert_eq!(app.diff.as_ref().unwrap().task, "t1");

    // 5. Switch back to Conv-B: its draft is preserved:
    app.open("conv-b".into());
    assert_eq!(app.conversation, "conv-b");
    assert_eq!(app.input.text, "conversation B draft");
}

#[test]
fn tui_in_flight_operations_complete_and_restore_drafts_after_a_b_a_switch() {
    let mut app = App::test_app();
    app.open("conv-flight-a".into());

    // 1. Send message: input is cleared and message is in flight
    app.input = InputBuffer::with_text("initial message content");
    app.submit();
    let send_req = app.send_request_id;
    assert!(app.sending);
    assert!(app.input.is_empty());
    let gen_before_switch = app.generation;

    // 2. Cycle A -> B -> A
    app.open("conv-flight-b".into());
    assert_eq!(app.conversation, "conv-flight-b");
    app.open("conv-flight-a".into());
    assert_eq!(app.conversation, "conv-flight-a");
    assert_ne!(app.generation, gen_before_switch);
    assert!(app.sending);
    assert!(app.input.is_empty());

    // 3. Delayed network error arrives after A -> B -> A cycle
    app.apply_update(Update::SendResult {
        conversation: "conv-flight-a".into(),
        generation: gen_before_switch,
        request_id: send_req,
        sent_text: "initial message content".into(),
        result: Err("awaria sieci".into()),
    });

    // Verification: sending flag is cleared, draft is restored to input!
    assert!(!app.sending, "sending must be cleared after A->B->A error");
    assert_eq!(app.pending_send, None);
    assert_eq!(app.input.text, "initial message content");
    assert_eq!(
        app.notice.as_deref(),
        Some("✗ awaria sieci (draft restored)")
    );

    // 4. Send comment: editor is cleared and comment is in flight
    app.editor = Some(Editor {
        target: "task:t5".into(),
        input: InputBuffer::with_text("important task feedback"),
    });
    app.submit_comment();
    let comment_req = app.comment_request_id;
    assert!(app.sending_comment);
    assert!(app.editor.is_none());
    let gen_comment_start = app.generation;

    // 5. Cycle A -> B -> A
    app.open("conv-flight-b".into());
    app.open("conv-flight-a".into());
    assert_ne!(app.generation, gen_comment_start);
    assert!(app.sending_comment);
    assert!(app.editor.is_none());

    // 6. Comment error arrives after A -> B -> A cycle
    let pending_editor = app.pending_comment.clone().unwrap();
    app.apply_update(Update::CommentResult {
        conversation: "conv-flight-a".into(),
        plan: None,
        generation: gen_comment_start,
        request_id: comment_req,
        editor: pending_editor,
        result: Err("comment rejected".into()),
    });

    // Verification: sending_comment flag is cleared, comment editor is restored!
    assert!(
        !app.sending_comment,
        "sending_comment must be cleared after A->B->A error"
    );
    assert_eq!(app.pending_comment, None);
    assert!(app.editor.is_some());
    let ed = app.editor.as_ref().unwrap();
    assert_eq!(ed.target, "task:t5");
    assert_eq!(ed.input.text, "important task feedback");
    assert_eq!(
        app.notice.as_deref(),
        Some("✗ comment rejected (comment draft preserved)")
    );

    // 7. Verify response arriving while user is in B (A -> B), then returns to A (B -> A)
    app.input = InputBuffer::with_text("message processed in the background");
    app.submit();
    let send_req_bg = app.send_request_id;
    assert!(app.sending);

    app.open("conv-flight-b".into());
    assert_eq!(app.conversation, "conv-flight-b");
    assert!(!app.sending);

    // While in B, background failure arrives for A
    app.apply_update(Update::SendResult {
        conversation: "conv-flight-a".into(),
        generation: 999,
        request_id: send_req_bg,
        sent_text: "message processed in the background".into(),
        result: Err("timeout serwera".into()),
    });

    // Switch back to A: draft was restored into state.input and sending is false!
    app.open("conv-flight-a".into());
    assert_eq!(app.conversation, "conv-flight-a");
    assert!(!app.sending);
    assert_eq!(app.input.text, "message processed in the background");
}

#[test]
fn tui_comments_with_same_target_in_different_conversations_and_plans() {
    let mut app = App::test_app();

    // 1. (conv1, None, "worker:t1")
    app.open("conv1".into());
    app.view.plan = None;
    app.save_comment_draft(
        "worker:t1",
        InputBuffer::with_text("conv1 draft without a plan"),
    );

    // 2. (conv1, Some("plan-alpha"), "worker:t1")
    app.view.plan = Some("plan-alpha".into());
    app.save_comment_draft(
        "worker:t1",
        InputBuffer::with_text("conv1 plan alpha draft"),
    );

    // 3. (conv1, Some("plan-beta"), "worker:t1")
    app.view.plan = Some("plan-beta".into());
    app.save_comment_draft("worker:t1", InputBuffer::with_text("conv1 plan beta draft"));

    // 4. (conv2, Some("plan-alpha"), "worker:t1")
    app.open("conv2".into());
    app.view.plan = Some("plan-alpha".into());
    app.save_comment_draft(
        "worker:t1",
        InputBuffer::with_text("conv2 plan alpha draft"),
    );

    // Verify all 4 are isolated and accessible without collision:
    assert_eq!(
        app.get_comment_draft("worker:t1").text,
        "conv2 plan alpha draft"
    );

    // Switch back to conv1
    app.open("conv1".into());
    // Under plan-beta:
    app.view.plan = Some("plan-beta".into());
    assert_eq!(
        app.get_comment_draft("worker:t1").text,
        "conv1 plan beta draft"
    );

    // Under plan-alpha:
    app.view.plan = Some("plan-alpha".into());
    assert_eq!(
        app.get_comment_draft("worker:t1").text,
        "conv1 plan alpha draft"
    );

    // Under None:
    app.view.plan = None;
    assert_eq!(
        app.get_comment_draft("worker:t1").text,
        "conv1 draft without a plan"
    );
}

#[test]
fn tui_history_recall_with_cursor_restoration() {
    let mut app = App::test_app();

    app.record_history("first command");
    app.record_history("second command");

    // Draft in progress with specific cursor position and preferred_col
    app.input = InputBuffer::with_text("draft with the cursor in the middle");
    app.input.cursor = 7;
    app.input.preferred_col = Some(7);

    // Up arrow navigates into history
    app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.history_index, Some(1));
    assert_eq!(app.input.text, "second command");

    // Up arrow again to older history
    app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.history_index, Some(0));
    assert_eq!(app.input.text, "first command");

    // Down arrow navigates back down
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.history_index, Some(1));
    assert_eq!(app.input.text, "second command");

    // Down arrow returns to draft with exact cursor and preferred_col restored!
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.history_index, None);
    assert_eq!(app.input.text, "draft with the cursor in the middle");
    assert_eq!(app.input.cursor, 7);
    assert_eq!(app.input.preferred_col, Some(7));

    // Also test Esc cancels history browsing and restores the draft:
    app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.history_index, Some(1));
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.history_index, None);
    assert_eq!(app.input.text, "draft with the cursor in the middle");
    assert_eq!(app.input.cursor, 7);
    assert_eq!(app.input.preferred_col, Some(7));
}

#[test]
fn tui_preferred_column_memory_and_grapheme_editing() {
    // 1. Preferred column memory across lines of differing lengths
    let text = "the first line has twenty characters\nshort!\nthe third line is very long again";
    let mut buf = InputBuffer::with_text(text);
    // Position cursor at col 20 on line 0
    buf.cursor = 20;
    let (c, r) = buf.visual_cursor(80);
    assert_eq!(r, 0);
    assert_eq!(c, 20);

    // Move down to short line (width = 6)
    buf.move_down(80);
    let (c2, r2) = buf.visual_cursor(80);
    assert_eq!(r2, 1);
    assert_eq!(c2, 6); // Clamped to line end
    assert_eq!(buf.preferred_col, Some(20)); // Memory maintained!

    // Move down to long line (line 2)
    buf.move_down(80);
    let (c3, r3) = buf.visual_cursor(80);
    assert_eq!(r3, 2);
    assert_eq!(c3, 20); // Restored back to col 20!

    // Moving up back to short line:
    buf.move_up(80);
    let (c4, r4) = buf.visual_cursor(80);
    assert_eq!(r4, 1);
    assert_eq!(c4, 6);
    assert_eq!(buf.preferred_col, Some(20));

    // Moving up back to line 0:
    buf.move_up(80);
    let (c5, r5) = buf.visual_cursor(80);
    assert_eq!(r5, 0);
    assert_eq!(c5, 20);

    // Horizontal movement resets preferred_col
    buf.move_left();
    assert_eq!(buf.preferred_col, None);

    // 2. Grapheme cluster editing: Unicode letters and composite emojis
    let mut gbuf = InputBuffer::with_text("Abąćęł 🚀 👨‍👩‍👧‍👦 end");
    // Position cursor right after the family emoji
    let family_idx = gbuf.text.find(" 👨‍👩‍👧‍👦").unwrap() + " 👨‍👩‍👧‍👦".len();
    gbuf.cursor = family_idx;

    // Backspace must remove the entire 18-byte ZWJ sequence atomically:
    assert!(gbuf.backspace());
    assert_eq!(gbuf.text, "Abąćęł 🚀  end");

    // Position cursor right before 🚀
    let rocket_idx = gbuf.text.find('🚀').unwrap();
    gbuf.cursor = rocket_idx;
    // Delete must remove the 4-byte rocket emoji atomically:
    assert!(gbuf.delete());
    assert_eq!(gbuf.text, "Abąćęł   end");

    // Stepping left/right across multi-byte Unicode test characters:
    let mut pol = InputBuffer::with_text("abąćęł");
    pol.move_buffer_start();
    assert_eq!(pol.cursor, 0);
    pol.move_right(); // 'a'
    assert_eq!(pol.cursor, 1);
    pol.move_right(); // 'b'
    assert_eq!(pol.cursor, 2);
    pol.move_right(); // 'ą' (2 bytes)
    assert_eq!(pol.cursor, 4);
    pol.move_right(); // 'ć' (2 bytes)
    assert_eq!(pol.cursor, 6);
    pol.move_right(); // 'ę' (2 bytes)
    assert_eq!(pol.cursor, 8);
    pol.move_right(); // 'ł' (2 bytes)
    assert_eq!(pol.cursor, 10);
    pol.move_left(); // back over 'ł'
    assert_eq!(pol.cursor, 8);
    assert!(pol.backspace()); // backspace 'ę'
    assert_eq!(pol.text, "abąćł");
    assert_eq!(pol.cursor, 6);
}

#[test]
fn tui_cards_structural_entries_and_actual_delivery_states() {
    let mut app = App::test_app();
    // 1. Initial mail event
    app.view.apply(&serde_json::json!({
        "kind": "mail",
        "payload": {
            "sender": "user",
            "recipient": "orchestrator",
            "text": "First user request",
            "kind": "request",
            "id": 101,
        }
    }));
    assert_eq!(app.view.messages.len(), 1);
    assert_eq!(app.view.messages[0].sender, "You");
    assert_eq!(app.view.messages[0].state, "waiting for turn");
    assert_eq!(app.view.cards.len(), 1);

    // 2. Delivery confirmation alters actual state
    app.view.apply(&serde_json::json!({
        "kind": "mail_delivered",
        "payload": {
            "ids": [101],
        }
    }));
    assert_eq!(app.view.messages[0].state, "delivered");
    if let TranscriptCard::Message(msg) = &app.view.cards[0] {
        assert_eq!(msg.state, "delivered");
    } else {
        panic!("expected TranscriptCard::Message");
    }

    // 3. Rejected mail event
    app.view.apply(&serde_json::json!({
        "kind": "mail",
        "payload": {
            "sender": "orchestrator",
            "recipient": "worker:t9",
            "text": "Task t9 to perform",
            "kind": "task",
            "id": 102,
        }
    }));
    assert_eq!(app.view.messages[1].state, "waiting for turn");
    app.view.apply(&serde_json::json!({
        "kind": "mail_rejected",
        "payload": {
            "sender": "orchestrator",
            "recipient": "worker:t9",
            "reason": "agent unavailable",
        }
    }));
    assert_eq!(app.view.messages[1].state, "rejected");

    // 4. Render on a terminal and verify buffer symbols
    let mut term = Terminal::new(TestBackend::new(140, 30)).unwrap();
    term.draw(|f| app.draw(f)).unwrap();
    let text: String = term
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(text.contains("You"));
    assert!(text.contains("orchestrator"));
    assert!(text.contains("delivered"));
    assert!(text.contains("rejected"));
    assert!(text.contains("First user request"));
}

#[test]
fn tui_compact_minimal_terminals_from_12x5_to_84_and_split_at_85() {
    let mut app = App::test_app();
    app.input = InputBuffer::with_text("test command");

    // 11x4: Below floor triggers guard
    let mut term_11x4 = Terminal::new(TestBackend::new(11, 4)).unwrap();
    term_11x4.draw(|f| app.draw(f)).unwrap();
    let text_11x4: String = term_11x4
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    let norm_11x4 = text_11x4.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(norm_11x4.contains("Terminal too small (min. 12x5 required)"));

    // 12x5: Minimal working terminal renders without crashing
    let mut term_12x5 = Terminal::new(TestBackend::new(12, 5)).unwrap();
    term_12x5.draw(|f| app.draw(f)).unwrap();
    let (cx, cy) = term_12x5.get_cursor_position().unwrap().into();
    assert!(
        (1..=10).contains(&cx),
        "Cursor x ({cx}) must be inside 12-col terminal bounds"
    );
    assert!(
        (1..=4).contains(&cy),
        "Cursor y ({cy}) must be inside 5-row terminal bounds"
    );

    // Can toggle panel focus via Tab even in compact mode
    assert_eq!(app.focus, Focus::Chat);
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.focus, Focus::Graph);
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.focus, Focus::Chat);

    // Can open Help via F1 and dismiss with Esc on 12x5
    assert!(app.key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE)));
    assert!(app.help.is_some());
    term_12x5.draw(|f| app.draw(f)).unwrap();
    assert!(app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
    assert!(app.help.is_none());

    // 84 cols (< 85): Compact header badge
    let mut term_84 = Terminal::new(TestBackend::new(84, 20)).unwrap();
    term_84.draw(|f| app.draw(f)).unwrap();
    let text_84: String = term_84
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(text_84.contains("[Chat]"));
    assert!(text_84.contains("Team"));

    // 85 cols (>= 85): Split side-by-side with vertical separator
    let mut term_85 = Terminal::new(TestBackend::new(85, 20)).unwrap();
    term_85.draw(|f| app.draw(f)).unwrap();
    let text_85: String = term_85
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(!text_85.contains("[Czat]"));
    // The separator column contains vertical glyph
    let sep_col = 35;
    assert_eq!(term_85.backend().buffer()[(sep_col, 5)].symbol(), "│");
}

#[test]
fn tui_ascii_mode_and_theme_glyphs() {
    let t_ascii = Theme::new(false, true);
    assert_eq!(t_ascii.glyphs().vertical, "|");
    assert_eq!(t_ascii.glyphs().bar, "|");
    assert_eq!(t_ascii.glyphs().to, ">");
    assert_eq!(t_ascii.glyphs().done, "x");

    // Unicode letter transliteration in ASCII mode
    assert_eq!(t_ascii.clean("Abąćęł cńódź efżó"), "Abacel cnodz efzo");
    assert_eq!(
        t_ascii.clean("Step 1: ✓ approved → done…"),
        "Step 1: + approved > done."
    );

    // Rendering in ASCII mode
    let mut buffer = Buffer::empty(Rect::new(0, 0, 40, 5));
    let rows = card(
        &t_ascii,
        CardProps {
            fill: t_ascii.card(),
            accent: t_ascii.accent(),
            head: vec![
                Span::raw("You"),
                Span::raw(" > "),
                Span::raw("orchestrator"),
            ],
            right: "delivered".into(),
            body: "Abąćęł cńódź efżó",
            max_body: usize::MAX,
            width: 40,
        },
    );
    draw_rows(&mut buffer, Rect::new(0, 0, 40, 5), &rows);
    let rendered: String = buffer.content.iter().map(|c| c.symbol()).collect();
    assert!(rendered.contains("|"));
    assert!(rendered.contains("Abacel cnodz efzo"));
    assert!(!rendered.contains("▌"));
}

#[test]
fn tui_no_color_mode_preserves_modifiers_and_clears_colors() {
    let t_nc = Theme::new(false, false);
    assert_eq!(t_nc.text().fg, None);
    assert_eq!(t_nc.text().bg, None);
    assert_eq!(t_nc.card().bg, None);
    assert_eq!(t_nc.raised().bg, None);
    assert!(t_nc.title().add_modifier.contains(Modifier::BOLD));
    assert!(t_nc.dim().add_modifier.contains(Modifier::DIM));

    let mut buf = Buffer::empty(Rect::new(0, 0, 30, 3));
    let rows = card(
        &t_nc,
        CardProps {
            fill: t_nc.card(),
            accent: t_nc.accent(),
            head: vec![Span::raw("Heading")],
            right: "meta".into(),
            body: "Card content",
            max_body: usize::MAX,
            width: 30,
        },
    );
    draw_rows(&mut buf, Rect::new(0, 0, 30, 3), &rows);
    assert!(
        buf.content
            .iter()
            .all(|c| c.fg == Color::Reset && c.bg == Color::Reset)
    );
}

#[test]
fn tui_malicious_output_sanitization_on_buffer() {
    let malicious = "Normal \x1b]52;c;c3RlYWw=\x07text \x1bPpayload\x1b\\with \u{202e}bidi\u{202c} and \u{9b}31mcolor\u{9c}";
    let cleaned = sanitize(malicious);
    assert!(!cleaned.contains('\x1b'));
    assert!(!cleaned.contains("c3RlYWw="));
    assert!(!cleaned.contains('\u{202e}'));
    assert_eq!(cleaned, "Normal text with bidi and color");

    let mut app = App::test_app();
    app.view.apply(&serde_json::json!({
        "kind": "mail",
        "payload": {
            "sender": "user",
            "recipient": "orchestrator",
            "text": malicious,
            "kind": "request",
            "id": 999,
        }
    }));
    app.view.activity = Some(malicious.into());

    let mut term = Terminal::new(TestBackend::new(100, 30)).unwrap();
    term.draw(|f| app.draw(f)).unwrap();
    let text: String = term
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(!text.contains('\x1b'));
    assert!(!text.contains("c3RlYWw="));
    assert!(!text.contains('\u{202e}'));
    assert!(text.contains("Normal text"));
}

#[test]
fn test_spatial_graph_stable_positions_large_and_cyclic_plans() {
    use antislop::lean::tui::graph::{Graph, GroupingMode};
    use antislop::lean::tui::view::{AUDITOR, ORCHESTRATOR, PLAN_VERIFIER, TaskRow, View};

    // 1. Stable, deterministic positions based on dependency topology
    let mut view = View::default();
    view.tasks = vec![
        TaskRow {
            id: "t1".into(),
            title: "Task 1".into(),
            depends_on: vec![],
            status: "proposed".into(),
            ..TaskRow::default()
        },
        TaskRow {
            id: "t2".into(),
            title: "Task 2".into(),
            depends_on: vec!["t1".into()],
            status: "proposed".into(),
            ..TaskRow::default()
        },
        TaskRow {
            id: "t3".into(),
            title: "Task 3".into(),
            depends_on: vec!["t1".into()],
            status: "proposed".into(),
            ..TaskRow::default()
        },
        TaskRow {
            id: "t4".into(),
            title: "Task 4".into(),
            depends_on: vec!["t2".into(), "t3".into()],
            status: "proposed".into(),
            ..TaskRow::default()
        },
    ];

    let g1 = Graph::new(&view);
    let g2 = Graph::new(&view);
    for (n1, n2) in g1.nodes.iter().zip(g2.nodes.iter()) {
        assert_eq!(n1.key, n2.key);
        assert_eq!(n1.x, n2.x);
        assert_eq!(n1.y, n2.y);
    }

    let node_pos = |g: &Graph, key: &str| {
        let n = g.node(key).unwrap();
        (n.x, n.y)
    };
    let w1 = node_pos(&g1, "worker:t1");
    let v1 = node_pos(&g1, "verifier:t1");
    let w2 = node_pos(&g1, "worker:t2");
    let w3 = node_pos(&g1, "worker:t3");
    let w4 = node_pos(&g1, "worker:t4");

    // Worker and verifier alignment: verifier sits directly below worker
    assert_eq!(w1.0, v1.0);
    assert_eq!(v1, w1);

    // Columns advance with dependency depth: w1.0 < w2.0 <= w4.0
    assert!(w1.0 < w2.0);
    assert_eq!(w2.0, w3.0);
    assert!(w2.0 < w4.0);

    // Fixed supervision positions
    assert_eq!(node_pos(&g1, PLAN_VERIFIER).1, 0);
    assert_eq!(node_pos(&g1, ORCHESTRATOR).1, 0);
    assert_eq!(node_pos(&g1, AUDITOR).1, 0);

    // 2. Large graph (500 tasks): tests non-recursive Kahn's algorithm
    let mut large_tasks = Vec::new();
    for i in 0..500 {
        let parent = if i > 0 {
            vec![format!("task_{}", i - 1)]
        } else {
            vec![]
        };
        large_tasks.push(TaskRow {
            id: format!("task_{i}"),
            title: format!("Task {i}"),
            depends_on: parent,
            status: "proposed".into(),
            ..TaskRow::default()
        });
    }
    let mut large_view = View::default();
    large_view.tasks = large_tasks;
    let large_graph = Graph::new(&large_view);
    assert_eq!(large_graph.nodes.len(), 3 + 500);
    assert!(large_graph.width > 0);
    assert!(large_graph.height > 0);

    // 3. Pathological cyclic dependencies: c1 -> c2 -> c3 -> c1
    let cyclic_tasks = vec![
        TaskRow {
            id: "c1".into(),
            title: "Cyclic 1".into(),
            depends_on: vec!["c2".into()],
            status: "proposed".into(),
            ..TaskRow::default()
        },
        TaskRow {
            id: "c2".into(),
            title: "Cyclic 2".into(),
            depends_on: vec!["c3".into()],
            status: "proposed".into(),
            ..TaskRow::default()
        },
        TaskRow {
            id: "c3".into(),
            title: "Cyclic 3".into(),
            depends_on: vec!["c1".into()],
            status: "proposed".into(),
            ..TaskRow::default()
        },
        TaskRow {
            id: "leaf".into(),
            title: "Leaf Task".into(),
            depends_on: vec!["c3".into()],
            status: "proposed".into(),
            ..TaskRow::default()
        },
    ];
    let mut cyclic_view = View::default();
    cyclic_view.tasks = cyclic_tasks;
    let cyclic_graph = Graph::new(&cyclic_view);
    assert_eq!(cyclic_graph.nodes.len(), 3 + 4);
    for node in &cyclic_graph.nodes {
        assert!(node.visible);
    }

    // 4. Grouping modes (Dependency vs Status)
    let mut status_view = View::default();
    status_view.tasks = vec![
        TaskRow {
            id: "s1".into(),
            title: "Task S1".into(),
            depends_on: vec![],
            status: "proposed".into(),
            ..TaskRow::default()
        },
        TaskRow {
            id: "s2".into(),
            title: "Task S2".into(),
            depends_on: vec![],
            status: "working".into(),
            ..TaskRow::default()
        },
        TaskRow {
            id: "s3".into(),
            title: "Task S3".into(),
            depends_on: vec![],
            status: "approved".into(),
            ..TaskRow::default()
        },
    ];
    let dep_graph = Graph::new_with_options(
        &status_view,
        None,
        &std::collections::HashSet::new(),
        GroupingMode::Dependency,
    );
    let stat_graph = Graph::new_with_options(
        &status_view,
        None,
        &std::collections::HashSet::new(),
        GroupingMode::Status,
    );

    assert_eq!(
        dep_graph.node("worker:s1").unwrap().x,
        dep_graph.node("worker:s2").unwrap().x
    );
    assert!(stat_graph.node("worker:s1").unwrap().x < stat_graph.node("worker:s2").unwrap().x);
    assert!(stat_graph.node("worker:s2").unwrap().x < stat_graph.node("worker:s3").unwrap().x);
}

#[test]
fn test_spatial_graph_vdm_relation_markers_and_visible_only_navigation() {
    use antislop::lean::tui::graph::{Canvas, Graph, GroupingMode};
    use antislop::lean::tui::view::{Link, TaskRow, View};

    let mut view = View::default();
    view.tasks = vec![
        TaskRow {
            id: "t1".into(),
            title: "First task".into(),
            depends_on: vec![],
            status: "working".into(),
            ..TaskRow::default()
        },
        TaskRow {
            id: "t2".into(),
            title: "Second task".into(),
            depends_on: vec!["t1".into()],
            status: "proposed".into(),
            ..TaskRow::default()
        },
    ];
    view.links.insert(
        ("worker:t1".into(), "worker:t2".into()),
        Link {
            count: 3,
            last: std::time::Instant::now(),
        },
    );

    // 1. Edge and node relation markers (V, D, M)
    let graph = Graph::new(&view);
    let mut canvas = Canvas::new(graph.width, graph.height);
    graph.render(&view, &mut canvas, Filter::All, "worker:t1");
    let lines = canvas.lines();
    let text = lines.join("\n");

    // Edges and node badges contain markers
    assert!(text.contains('V') || text.contains('D') || text.contains("✉3"));

    // 2. Visible-only navigation and Collapsing
    let next_below = graph.neighbour("worker:t1", 0, 1);
    assert_ne!(next_below.as_deref(), Some("verifier:t1"));

    let mut collapsed = std::collections::HashSet::new();
    collapsed.insert("t1".into());
    let collapsed_graph =
        Graph::new_with_options(&view, None, &collapsed, GroupingMode::Dependency);

    let w1 = collapsed_graph.node("worker:t1").unwrap();
    let v1 = collapsed_graph.node("verifier:t1").unwrap();
    assert!(w1.visible);
    assert!(w1.collapsed);
    assert!(v1.visible);

    let next_below_collapsed = collapsed_graph.neighbour("worker:t1", 0, 1);
    assert_ne!(next_below_collapsed.as_deref(), Some("verifier:t1"));

    // 3. Text query filtering
    let mut app = App::test_app();
    app.view.tasks = vec![
        TaskRow {
            id: "t1".into(),
            title: "First task".into(),
            depends_on: vec![],
            status: "working".into(),
            ..TaskRow::default()
        },
        TaskRow {
            id: "t2".into(),
            title: "Second task".into(),
            depends_on: vec!["t1".into()],
            status: "proposed".into(),
            ..TaskRow::default()
        },
    ];
    app.focus = Focus::Graph;
    app.selected = "worker:t1".into();

    app.set_graph_filter(Some("second".into()));
    assert_ne!(app.selected, "worker:t1");
    assert!(app.selected.contains("t2"));

    let filtered_graph = Graph::new_with_options(
        &view,
        Some("second"),
        &std::collections::HashSet::new(),
        GroupingMode::Dependency,
    );
    assert!(!filtered_graph.node("worker:t1").unwrap().visible);
    assert!(filtered_graph.node("worker:t2").unwrap().visible);

    let nav = filtered_graph.neighbour("worker:t2", -1, 0);
    if let Some(target) = nav {
        assert!(filtered_graph.node(&target).unwrap().visible);
        assert_ne!(target, "worker:t1");
    }

    // Toggle collapse via key
    app.set_graph_filter(None);
    app.selected = "worker:t1".into();
    assert!(!app.graph_collapsed.contains("t1"));
    app.key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
    assert!(app.graph_collapsed.contains("t1"));
    app.key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
    assert!(!app.graph_collapsed.contains("t1"));
}

#[test]
fn test_searchable_conversation_switcher_with_project_filtering_and_draft_retention() {
    let dir = tempfile::tempdir().unwrap();
    let sock = socket(dir.path());
    let listener = UnixListener::bind(&sock).unwrap();
    let (tx, rx) = mpsc::channel();

    let _server = thread::spawn(move || {
        while let Ok((stream, _)) = listener.accept() {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            if reader.read_line(&mut line).is_ok()
                && !line.is_empty()
                && let Ok(parsed) = serde_json::from_str::<Value>(&line)
            {
                let _ = tx.send(parsed.clone());
                let mut writer = stream;
                if parsed["op"] == "conversations" {
                    let resp = json!({
                        "data": [
                            {
                                "id": "c-alpha",
                                "project": "my-project",
                                "status": "working",
                                "tasks_total": 4,
                                "tasks_done": 2
                            },
                            {
                                "id": "c-foreign",
                                "project": "other-project",
                                "status": "idle"
                            },
                            {
                                "id": "c-beta",
                                "project": "my-project",
                                "status": "idle"
                            }
                        ],
                        "error": null
                    });
                    let _ = writer.write_all(format!("{resp}\n").as_bytes());
                } else {
                    let _ = writer.write_all(b"{\"data\":null,\"error\":null}\n");
                }
                let _ = writer.flush();
            }
        }
    });

    let client = Client::at(dir.path());
    let (mut app, _rx_updates) = App::test_app_with_client(client);
    app.project = "my-project".into();
    app.conversation = "c-initial".into();

    // 1. Enter chat draft in current conversation
    app.input.insert_str("Draft in c-initial");
    app.input.cursor = 5;

    // 2. Open switcher via Ctrl+O
    app.key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
    let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(req["op"], "conversations");

    assert!(app.picker.is_some());
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(picker.kind, Kind::Conversations);

    // Foreign project conversation was filtered out
    assert!(!picker.conversations.iter().any(|c| c.id == "c-foreign"));
    // Current project conversations are present
    assert!(picker.conversations.iter().any(|c| c.id == "c-alpha"));
    assert!(picker.conversations.iter().any(|c| c.id == "c-beta"));
    assert!(picker.conversations.iter().any(|c| c.id == "c-initial"));

    // 3. Search filtering
    app.key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));

    let items = app.picker.as_ref().unwrap().items(&app.view);
    assert_eq!(items.len(), 1);
    assert!(items[0].0.contains("c-beta"));

    // 4. Press Enter to switch to c-beta
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.picker.is_none());
    assert_eq!(app.conversation, "c-beta");

    // In c-beta, input is fresh
    assert!(app.input.is_empty());
    app.input.insert_str("Draft in c-beta");
    app.input.cursor = 10;

    // 5. Open switcher again and switch back to c-initial
    app.key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
    let _ = rx.recv_timeout(Duration::from_secs(5)).unwrap();

    for ch in "initial".chars() {
        app.key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.conversation, "c-initial");

    // Draft and cursor in c-initial are fully preserved
    assert_eq!(app.input.text, "Draft in c-initial");
    assert_eq!(app.input.cursor, 5);

    // Switch again to c-beta: draft in c-beta preserved as well
    app.open("c-beta".into());
    assert_eq!(app.conversation, "c-beta");
    assert_eq!(app.input.text, "Draft in c-beta");
    assert_eq!(app.input.cursor, 10);
}

#[test]
fn test_refresh_updates_and_overlay_collision_avoidance() {
    use antislop::lean::tui::graph::{Graph, Node, compute_progress_overlay};
    use ratatui::layout::Rect;

    // 1. Overlay collision avoidance
    let inner = Rect::new(0, 0, 80, 25);
    let progress_width = 30;
    let progress_height = 6;
    let default_overlay = Rect::new(50, 19, 30, 6);

    // Case A: No node selected
    let pos_none = compute_progress_overlay(inner, None, (0, 0), progress_width, progress_height);
    assert_eq!(pos_none, Some(default_overlay));

    // Case B: Node selected at top-left (no collision)
    let node_top_left = Node {
        key: "worker:t1".into(),
        x: 0,
        y: 0,
        height: antislop::lean::tui::graph::HEIGHT,
        visible: true,
        group: None,
        collapsed: false,
    };
    let pos_top_left = compute_progress_overlay(
        inner,
        Some(&node_top_left),
        (0, 0),
        progress_width,
        progress_height,
    );
    assert_eq!(pos_top_left, Some(default_overlay));

    // Case C: Node selected at bottom-right (collision!) -> flips to top-right
    let node_bottom_right = Node {
        key: "worker:t2".into(),
        x: 55,
        y: 20,
        height: antislop::lean::tui::graph::HEIGHT,
        visible: true,
        group: None,
        collapsed: false,
    };
    let pos_flipped = compute_progress_overlay(
        inner,
        Some(&node_bottom_right),
        (0, 0),
        progress_width,
        progress_height,
    );
    assert_eq!(pos_flipped, Some(Rect::new(50, 0, 30, 6)));

    // Case D: Node selected at bottom-right but HIDDEN -> stays at default bottom-right
    let node_hidden = Node {
        key: "verifier:t2".into(),
        x: 55,
        y: 20,
        height: antislop::lean::tui::graph::HEIGHT,
        visible: false,
        group: None,
        collapsed: false,
    };
    let pos_hidden = compute_progress_overlay(
        inner,
        Some(&node_hidden),
        (0, 0),
        progress_width,
        progress_height,
    );
    assert_eq!(pos_hidden, Some(default_overlay));

    // 2. Refresh updates preserving graph selection and state
    let mut app = App::test_app();
    app.view.apply(&json!({
        "seq": 1,
        "kind": "plan_proposed",
        "payload": {
            "plan": "plan-1",
            "tasks": [
                {"id": "t1", "title": "Step 1", "round": 1, "status": "proposed"},
                {"id": "t2", "title": "Step 2", "round": 1, "status": "proposed"},
                {"id": "t3", "title": "Step 3", "round": 1, "status": "proposed"}
            ]
        }
    }));

    app.focus = Focus::Graph;
    app.selected = "worker:t2".into();

    app.view.apply(&json!({
        "seq": 2,
        "kind": "task",
        "payload": {
            "id": "t2",
            "status": "working",
            "round": 2
        }
    }));

    assert_eq!(app.selected, "worker:t2");
    let cur_task = app.view.tasks.iter().find(|task| task.id == "t2").unwrap();
    assert_eq!(cur_task.id, "t2");
    assert_eq!(cur_task.status, "working");
    assert_eq!(cur_task.round, 2);

    let g = Graph::new(&app.view);
    let n = g.node("worker:t2").unwrap();
    assert!(n.visible);
}

#[test]
fn test_inspector_scroll_and_query_memory_per_agent_and_tab() {
    let mut details = Details::new("worker:t1".into());
    // Agent worker:t1, Tab 0
    assert_eq!(details.tab, 0);
    details.scroll = 15;
    details.query = "analysis".into();

    // Switch to Tab 1
    details.switch_tab(1);
    assert_eq!(details.tab, 1);
    assert_eq!(details.scroll, 0);
    assert_eq!(details.query, "");
    details.scroll = 8;
    details.query = "logi".into();

    // Switch to agent verifier:t1
    details.switch_key("verifier:t1".into());
    assert_eq!(details.key, "verifier:t1");
    assert_eq!(details.scroll, 0);
    assert_eq!(details.query, "");
    details.scroll = 25;
    details.query = "dowody".into();

    // Switch back to worker:t1: restores Tab 1 scroll 8 and query "logi"
    details.switch_key("worker:t1".into());
    assert_eq!(details.key, "worker:t1");
    assert_eq!(details.scroll, 8);
    assert_eq!(details.query, "logi");

    // Switch back to Tab 0: restores scroll 15 and query "analysis"
    details.switch_tab(0);
    assert_eq!(details.tab, 0);
    assert_eq!(details.scroll, 15);
    assert!(
        details.follow,
        "returning to Overview resumes the live session"
    );
    assert_eq!(details.query, "analysis");

    // Switch back to verifier:t1: active tab is 0 (never scrolled for verifier:t1)
    details.switch_key("verifier:t1".into());
    assert_eq!(details.key, "verifier:t1");
    assert_eq!(details.tab, 0);
    assert_eq!(details.scroll, 0);
    // Switch to tab 1 on verifier:t1: restores scroll 25 and query "dowody"
    details.switch_tab(1);
    assert_eq!(details.tab, 1);
    assert_eq!(details.scroll, 25);
    assert_eq!(details.query, "dowody");
}

#[test]
fn test_inspector_conversation_switch_preserves_details_memory() {
    let mut app = App::test_app();
    app.open("conv-A".into());
    app.open_details("worker:t1".into());
    if let Some(d) = &mut app.details {
        d.scroll = 42;
        d.query = "szukana_fraza".into();
        d.save_current_tab();
    }
    // Switch to conv-B
    app.open("conv-B".into());
    assert!(app.details.is_none());

    // Switch back to conv-A: details memory is preserved!
    app.open("conv-A".into());
    app.open_details("worker:t1".into());
    let d = app.details.as_ref().unwrap();
    assert_eq!(d.scroll, 42);
    assert_eq!(d.query, "szukana_fraza");
}

#[test]
fn test_inspector_whole_record_search_and_raw_view() {
    let mut details = Details::new("worker:t1".into());
    details.task = Some(json!({
        "id": "t1",
        "spec": {
            "title": "Task with a report and findings",
            "writes": true
        },
        "report": "Detailed final report with unique_key_12345",
        "findings": ["Finding A: analysis_key_xyz"],
        "artifacts": ["src/main.rs", "target/report.json"]
    }));
    details.tab = antislop::lean::tui::details::RESULT; // RESULT tab

    let mut view = antislop::lean::tui::view::View::default();
    view.tasks.push(antislop::lean::tui::view::TaskRow {
        id: "t1".into(),
        ..Default::default()
    });

    // 1. Without query: all records are shown
    let all_lines = details.lines(&view);
    assert!(
        all_lines
            .iter()
            .any(|(_, text)| text.contains("Detailed final report"))
    );

    // 2. Querying for "unique_key_12345" returns the ENTIRE record (header + content)
    details.query = "unique_key_12345".into();
    let filtered_lines = details.lines(&view);
    assert!(
        filtered_lines
            .iter()
            .any(|(_, text)| text.contains("Report")),
        "record header must be included"
    );
    assert!(
        filtered_lines
            .iter()
            .any(|(_, text)| text.contains("unique_key_12345")),
        "content must be included"
    );

    // 3. Raw view toggle
    details.raw = true;
    details.query.clear();
    let raw_lines = details.lines(&view);
    assert!(
        raw_lines
            .iter()
            .any(|(_, text)| text.contains('{') || text.contains('"')),
        "raw view must render JSON structure"
    );
}

#[test]
fn test_inspector_explicit_missing_data_notices() {
    let mut details = Details::new("worker:t1".into());
    details.task = Some(json!({
        "id": "t1",
        "spec": {
            "id": "t1",
            "title": "Empty task",
            "goal": "Testing checks",
            "kind": "implementation",
            "verifier": {
                "focus": "correctness",
                "risks": ["regression risk"]
            }
        },
        "verdict": null
    }));
    let mut view = antislop::lean::tui::view::View::default();
    view.tasks.push(antislop::lean::tui::view::TaskRow {
        id: "t1".into(),
        ..Default::default()
    });

    // Result -> explicit notices for missing report, artifacts, evidence
    details.tab = antislop::lean::tui::details::RESULT;
    let lines = details.lines(&view);
    let all_text = lines
        .iter()
        .map(|(_, t)| t.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        all_text.contains("[no historical report data]"),
        "must contain explicit missing report notice: {all_text}"
    );
    assert!(
        all_text.contains("[historical data unavailable]"),
        "must contain explicit missing artifacts notice: {all_text}"
    );
    assert!(
        all_text.contains("[no attached evidence"),
        "must contain explicit missing evidence notice: {all_text}"
    );

    // Checks -> explicit notices for missing controls data
    details.tab = antislop::lean::tui::details::CHECKS;
    let lines_checks = details.lines(&view);
    let all_checks = lines_checks
        .iter()
        .map(|(_, t)| t.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        all_checks.contains("no historical data"),
        "must contain controls text: {all_checks}"
    );

    // Comments -> explicit notice for missing comments
    details.tab = antislop::lean::tui::details::COMMENTS;
    let lines_comments = details.lines(&view);
    let all_comments = lines_comments
        .iter()
        .map(|(_, t)| t.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        all_comments.contains("[no comments"),
        "must contain explicit missing comments notice: {all_comments}"
    );

    // Overview: session history -> explicit notice when no sessions are retained
    details.tab = 0;
    let lines_sessions = details.lines(&view);
    let all_sessions = lines_sessions
        .iter()
        .map(|(_, t)| t.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        all_sessions.contains("historical"),
        "must contain explicit missing historical sessions notice: {all_sessions}"
    );
}

#[test]
fn overview_includes_the_terminal_transcript_without_database_mirrors() {
    use antislop::lean::tui::session::Block;
    use antislop::lean::tui::view::{Agent, View};
    let mut details = Details::new("worker:t1".into());
    details.tab = 0;
    details.history_unavailable = true;
    details.sessions.push(antislop::lean::store::SessionInfo {
        backend: None,
        turn_id: "database-turn".into(),
        plan_id: None,
        role: "worker:t1".into(),
        provider: "codex".into(),
        session_id: Some("database-session".into()),
        status: "done".into(),
        error: None,
        upto_message: 1,
        started_ms: 1000,
        finished_ms: Some(1050),
        pid: None,
    });
    let mut view = View::default();
    view.agents.insert(
        "worker:t1".into(),
        Agent {
            provider: "codex".into(),
            feed: vec![
                Block::Turn("codex · default model".into()),
                Block::Tool {
                    name: "shell".into(),
                    detail: "exec cargo check".into(),
                },
                Block::Text("answer".into()),
            ],
            session: vec![
                "── turn".into(),
                "exec cargo check".into(),
                "session mirror".into(),
            ],
            ..Default::default()
        },
    );
    view.tasks.push(antislop::lean::tui::view::TaskRow {
        id: "t1".into(),
        goal: "Shared task overview".into(),
        ..Default::default()
    });
    view.agents.insert(
        "verifier:t1".into(),
        Agent {
            provider: "claude".into(),
            feed: vec![Block::Text("Verifier evidence".into())],
            ..Default::default()
        },
    );
    let text = details
        .lines(&view)
        .into_iter()
        .map(|(_, line)| line)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !text.contains("Shared task overview"),
        "metadata stays outside the transcript"
    );
    assert!(text.contains("Session · worker"));
    assert!(text.contains("Session · verifier"));
    assert!(text.contains("Verifier evidence"));
    details.switch_key("verifier:t1".into());
    let verifier_text = details
        .lines(&view)
        .into_iter()
        .map(|(_, line)| line)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!verifier_text.contains("Shared task overview"));
    assert!(verifier_text.contains("• Ran exec cargo check"));
    assert!(verifier_text.contains("Verifier evidence"));
    details.query = "Verifier evidence".into();
    details.raw = true;
    let filtered = details
        .lines(&view)
        .into_iter()
        .map(|(_, line)| line)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(filtered.contains("\"session\""));
    assert!(filtered.contains("Verifier evidence"));
    assert!(!filtered.contains("exec cargo check"));
    details.query.clear();
    details.raw = false;
    details.switch_key("worker:t1".into());
    assert!(text.contains("codex · idle"));
    assert!(text.contains("• Ran exec cargo check"), "{text}");
    assert_eq!(text.matches("exec cargo check").count(), 1);
    assert!(!text.contains("session mirror"));
    assert!(!text.contains("database-turn"));
    assert!(!text.contains("database-session"));
    assert!(!text.contains("Retained sessions"));
    assert!(!text.contains("archive"));

    // Older event logs without terminal data still have one transcript.
    view.agents.get_mut("worker:t1").unwrap().feed.clear();
    let text = details
        .lines(&view)
        .into_iter()
        .map(|(_, line)| line)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("codex · idle"));
    assert_eq!(text.matches("exec cargo check").count(), 1);
    assert!(text.contains("session mirror"));
}

#[test]
fn test_conversation_details_inspector() {
    let mut details = Details::new("conversation".into());
    let mut view = antislop::lean::tui::view::View::default();
    view.tasks.push(antislop::lean::tui::view::TaskRow {
        id: "t1".into(),
        uuid: Some("u1".into()),
        title: "Kompilacja i testy".into(),
        status: "approved".into(),
        writes: true,
        ..Default::default()
    });
    view.plan = Some("P42".into());
    view.plan_scope_hash = Some("hash-p42".into());

    let lines = details.lines(&view);
    let text = lines
        .iter()
        .map(|(_, t)| t.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        text.contains("Conversation details"),
        "must show conversation title"
    );
    assert!(text.contains("Active plan: P42"), "must show plan");
    assert!(
        text.contains("Tasks: total 1, completed 1"),
        "must show task summary"
    );
}

#[test]
fn test_recipient_selection_and_broadcast() {
    let mut view = antislop::lean::tui::view::View::default();
    view.tasks.push(antislop::lean::tui::view::TaskRow {
        id: "t1".into(),
        title: "Building the kernel".into(),
        writes: true,
        ..Default::default()
    });

    let picker = Picker::new(Kind::Recipients);
    let items = picker.items(&view);

    // Verify broadcast and specific targets
    assert!(items.iter().any(|(label, act)| label.contains("all")
        && matches!(act, antislop::lean::tui::palette::Action::Address(a) if a == "all")));
    assert!(items.iter().any(|(label, act)| label.contains("orchestrator") && matches!(act, antislop::lean::tui::palette::Action::Address(a) if a == "orchestrator")));
    assert!(items.iter().any(|(label, act)| label.contains("worker:t1")
        && matches!(act, antislop::lean::tui::palette::Action::Address(a) if a == "worker:t1")));
    assert_eq!(items.len(), 6, "both protocol roles remain selectable");
    for status in ["working", "verifying", "approved"] {
        view.tasks[0].status = status.into();
        for role in ["worker", "verifier"] {
            let address = format!("{role}:t1");
            assert!(picker.items(&view).iter().any(|(_, act)| matches!(act, antislop::lean::tui::palette::Action::Address(a) if a == &address)));
        }
    }

    // Test app recipient action execution
    let mut app = App::test_app();
    app.view = view;
    // Selecting @all on empty draft
    app.picker = Some(picker);
    let _ = app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)); // select 'all'
    let _ = app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.input.text, "@all ");

    // Typing @ on empty draft opens recipient picker
    app.input.clear();
    app.key(KeyEvent::new(KeyCode::Char('@'), KeyModifiers::NONE));
    assert!(app.picker.is_some());
    assert_eq!(app.picker.as_ref().unwrap().kind, Kind::Recipients);
}

#[test]
fn test_precise_comment_targets_and_labels() {
    assert_eq!(target_name("plan:P1"), "Plan P1");
    assert_eq!(target_name("task:T1"), "Task T1");
    assert_eq!(target_name("check:C1"), "Check C1");
    assert_eq!(target_name("finding:F1"), "Finding F1");
    assert_eq!(target_name("report:R1"), "Task R1 report");
    assert_eq!(target_name("artifact:A1"), "Artifact A1");
    assert_eq!(target_name("diff:D1"), "D1 diff");
    assert_eq!(target_name("conversation"), "conversation");

    assert_eq!(comment_label("plan:P1", &[]), "the plan P1");
    assert_eq!(comment_label("task:T1", &[]), "specification of task T1");
    assert_eq!(comment_label("check:C1", &[]), "check C1");
    assert_eq!(comment_label("finding:F1", &[]), "finding F1");
    assert_eq!(comment_label("report:R1", &[]), "report of task R1");
    assert_eq!(comment_label("artifact:A1", &[]), "artifact A1");
}

#[test]
fn test_diff_reviewer_profile_rendered() {
    let mut diff = DiffView::new("t1".into());
    diff.reviewer = Some(ReviewerProfile {
        provider: "openai".into(),
        model: Some("o3-mini".into()),
        effort: Some("high".into()),
    });

    let header = diff_header_with_reviewer(
        "t1",
        "HEAD~1..HEAD",
        1,
        10,
        2,
        false,
        diff.reviewer.as_ref(),
        80,
    );
    let text = header
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect::<Vec<_>>()
        .join("");
    assert!(
        text.contains("reviewer:"),
        "diff header must include reviewer: {text}"
    );
}

#[test]
fn test_confirmation_card_action_lifecycle() {
    let (mut app, _rx) = App::test_app_with_client(Client::at(std::path::Path::new("/dev/null")));
    app.view.plan = Some("P100".into());
    app.view.plan_scope_hash = Some("hash-p100".into());

    let card = ConfirmationCard {
        id: "conf-1".into(),
        title: "Uruchomienie planu P100".into(),
        action: ConfirmationAction::AcceptPlan,
        plan_id: Some("P100".into()),
        task_id: None,
        candidate: None,
        scope_hash: "hash-p100".into(),
        status: ConfirmationStatus::Pending,
        detail: "5 tasks in the plan".into(),
    };

    app.request_confirmation(card.clone());
    assert!(app.confirmation.is_some());
    assert_eq!(app.view.cards.len(), 1);
    assert!(
        matches!(&app.view.cards[0], TranscriptCard::Confirmation(c) if c.id == "conf-1" && c.status == ConfirmationStatus::Pending)
    );

    // Confirm action via Enter key
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.confirmation.is_none());
    assert!(
        matches!(&app.view.cards[0], TranscriptCard::Confirmation(c) if c.id == "conf-1" && c.status == ConfirmationStatus::Confirmed)
    );

    // Test rejection
    let card2 = ConfirmationCard {
        id: "conf-2".into(),
        title: "Wznowienie planu P100".into(),
        action: ConfirmationAction::ResumePlan,
        plan_id: Some("P100".into()),
        task_id: None,
        candidate: None,
        scope_hash: "hash-p100".into(),
        status: ConfirmationStatus::Pending,
        detail: "Wznowienie".into(),
    };
    app.request_confirmation(card2);
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.confirmation.is_none());
    assert!(
        matches!(&app.view.cards[1], TranscriptCard::Confirmation(c) if c.id == "conf-2" && c.status == ConfirmationStatus::Rejected)
    );
}

#[test]
fn test_stale_confirmation_handling_and_rendering() {
    let (mut app, _rx) = App::test_app_with_client(Client::at(std::path::Path::new("/dev/null")));
    app.view.plan = Some("P100".into());
    app.view.plan_scope_hash = Some("scope-current".into());

    // 1. Confirm attempt with stale scope hash
    let stale_card = ConfirmationCard {
        id: "conf-stale".into(),
        title: "Uruchomienie planu P100".into(),
        action: ConfirmationAction::AcceptPlan,
        plan_id: Some("P100".into()),
        task_id: None,
        candidate: None,
        scope_hash: "scope-old".into(),
        status: ConfirmationStatus::Pending,
        detail: "5 tasks".into(),
    };
    app.request_confirmation(stale_card);
    assert!(app.confirmation.is_some());

    // User presses Enter to confirm -> rejected as stale
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.confirmation.is_none());
    assert!(
        matches!(&app.view.cards[0], TranscriptCard::Confirmation(c) if c.id == "conf-stale" && c.status == ConfirmationStatus::Stale)
    );
    assert!(app.notice.as_ref().unwrap().contains("stale_scope"));

    // 2. Background event changes scope while confirmation is pending
    let pending_card = ConfirmationCard {
        id: "conf-live".into(),
        title: "Uruchomienie planu P100".into(),
        action: ConfirmationAction::AcceptPlan,
        plan_id: Some("P100".into()),
        task_id: None,
        candidate: None,
        scope_hash: "scope-current".into(),
        status: ConfirmationStatus::Pending,
        detail: "5 tasks".into(),
    };
    app.request_confirmation(pending_card);
    assert!(app.confirmation.is_some());

    // Live update arrives changing the plan scope hash
    app.apply_update(Update::Events {
        conversation: "test-conv".into(),
        generation: app.generation,
        events: vec![json!({
            "seq": 99,
            "kind": "plan_proposed",
            "payload": {
                "plan": "P100",
                "scope_hash": "scope-newer",
                "tasks": []
            }
        })],
        idle: true,
        autonomous: false,
    });
    // Confirmation modal should be dismissed and card marked stale
    assert!(app.confirmation.is_none());
    assert!(
        matches!(&app.view.cards[1], TranscriptCard::Confirmation(c) if c.id == "conf-live" && c.status == ConfirmationStatus::Stale)
    );

    // 3. Daemon rejects with stale_scope asynchronously
    let confirmed_card = ConfirmationCard {
        id: "conf-async".into(),
        title: "Diff candidate".into(),
        action: ConfirmationAction::ReviewDiff,
        plan_id: Some("P100".into()),
        task_id: Some("t1".into()),
        candidate: Some("patch1".into()),
        scope_hash: "scope-newer".into(),
        status: ConfirmationStatus::Confirmed,
        detail: "Recenzja".into(),
    };
    app.view
        .cards
        .push(TranscriptCard::Confirmation(confirmed_card));
    app.apply_update(Update::Notice("✗ stale_scope: mismatch".into()));
    let async_card = app
        .view
        .cards
        .iter()
        .find_map(|c| match c {
            TranscriptCard::Confirmation(conf) if conf.id == "conf-async" => Some(conf),
            _ => None,
        })
        .expect("conf-async must exist");
    assert_eq!(async_card.status, ConfirmationStatus::Stale);

    // 4. Rendering test: verify stale cards render "[stale: stale_scope]"
    let mut terminal = Terminal::new(TestBackend::new(180, 40)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let rendered: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(
        rendered.contains("[stale: stale_scope]"),
        "rendered transcript must include stale notice: {rendered}"
    );
}

fn count_whole_cards(
    graph: &antislop::lean::tui::graph::Graph,
    inner_w: u16,
    inner_h: u16,
) -> usize {
    use antislop::lean::tui::graph::{HEIGHT, WIDTH};
    graph
        .nodes
        .iter()
        .filter(|n| n.visible && n.x + WIDTH <= inner_w && n.y + HEIGHT <= inner_h)
        .count()
}

#[test]
fn test_adaptive_graph_screen_utilization_and_compaction_matrix() {
    use antislop::lean::tui::graph::{Graph, GroupingMode};
    use antislop::lean::tui::view::{TaskRow, View};

    // Terminal dimensions and inner graph panel interiors
    // 80x24 (compact): inner 78x20
    // 85x24 (split): inner 48x20
    // 120x30 (split): inner 77x26
    // 160x50 (split): inner 104x46
    let terminals = [
        ("80x24", 78u16, 20u16),
        ("85x24", 48u16, 20u16),
        ("120x30", 77u16, 26u16),
        ("160x50", 104u16, 46u16),
    ];

    // 1. Parallel tasks (4 independent tasks)
    let parallel_tasks = vec![
        TaskRow {
            id: "p1".into(),
            title: "P1".into(),
            ..TaskRow::default()
        },
        TaskRow {
            id: "p2".into(),
            title: "P2".into(),
            ..TaskRow::default()
        },
        TaskRow {
            id: "p3".into(),
            title: "P3".into(),
            ..TaskRow::default()
        },
        TaskRow {
            id: "p4".into(),
            title: "P4".into(),
            ..TaskRow::default()
        },
    ];
    let mut par_view = View::default();
    par_view.tasks = parallel_tasks;

    let legacy_par = Graph::new_with_viewport(
        &par_view,
        None,
        &Default::default(),
        GroupingMode::Dependency,
        None,
    );

    for &(name, w, h) in &terminals {
        let adaptive_par = Graph::new_with_viewport(
            &par_view,
            None,
            &Default::default(),
            GroupingMode::Dependency,
            Some((w, h)),
        );
        let legacy_whole = count_whole_cards(&legacy_par, w, h);
        let adaptive_whole = count_whole_cards(&adaptive_par, w, h);
        assert!(
            adaptive_whole >= legacy_whole,
            "Terminal {name}: adaptive ({adaptive_whole}) should have >= whole cards than legacy ({legacy_whole})"
        );
    }

    // 2. Long chain (4 tasks sequential)
    let chain_tasks = vec![
        TaskRow {
            id: "c1".into(),
            title: "C1".into(),
            depends_on: vec![],
            ..TaskRow::default()
        },
        TaskRow {
            id: "c2".into(),
            title: "C2".into(),
            depends_on: vec!["c1".into()],
            ..TaskRow::default()
        },
        TaskRow {
            id: "c3".into(),
            title: "C3".into(),
            depends_on: vec!["c2".into()],
            ..TaskRow::default()
        },
        TaskRow {
            id: "c4".into(),
            title: "C4".into(),
            depends_on: vec!["c3".into()],
            ..TaskRow::default()
        },
    ];
    let mut chain_view = View::default();
    chain_view.tasks = chain_tasks;
    for &(name, w, _) in &terminals {
        let adaptive_chain = Graph::new_with_viewport(
            &chain_view,
            None,
            &Default::default(),
            GroupingMode::Dependency,
            Some((w, 20)),
        );
        assert!(
            adaptive_chain.nodes.iter().all(|n| n.visible),
            "Terminal {name} long chain: every card is placed"
        );
    }

    // 3. Branching (1 root branching to 3 parallel tasks)
    let branch_tasks = vec![
        TaskRow {
            id: "b1".into(),
            title: "B1".into(),
            depends_on: vec![],
            ..TaskRow::default()
        },
        TaskRow {
            id: "b2".into(),
            title: "B2".into(),
            depends_on: vec!["b1".into()],
            ..TaskRow::default()
        },
        TaskRow {
            id: "b3".into(),
            title: "B3".into(),
            depends_on: vec!["b1".into()],
            ..TaskRow::default()
        },
        TaskRow {
            id: "b4".into(),
            title: "B4".into(),
            depends_on: vec!["b1".into()],
            ..TaskRow::default()
        },
    ];
    let mut branch_view = View::default();
    branch_view.tasks = branch_tasks;
    let adaptive_branch = Graph::new_with_viewport(
        &branch_view,
        None,
        &Default::default(),
        GroupingMode::Dependency,
        Some((78, 20)),
    );
    assert!(adaptive_branch.nodes.iter().all(|n| n.visible));

    // 4. Cycles
    let cycle_tasks = vec![
        TaskRow {
            id: "cy1".into(),
            title: "CY1".into(),
            depends_on: vec!["cy2".into()],
            ..TaskRow::default()
        },
        TaskRow {
            id: "cy2".into(),
            title: "CY2".into(),
            depends_on: vec!["cy1".into()],
            ..TaskRow::default()
        },
    ];
    let mut cycle_view = View::default();
    cycle_view.tasks = cycle_tasks;
    let adaptive_cycle = Graph::new_with_viewport(
        &cycle_view,
        None,
        &Default::default(),
        GroupingMode::Dependency,
        Some((78, 20)),
    );
    assert_eq!(adaptive_cycle.nodes.len(), 3 + 2);

    // 5. Compaction after collapsing and filtering
    let mut collapsed = std::collections::HashSet::new();
    collapsed.insert("p1".into());
    let collapsed_graph = Graph::new_with_viewport(
        &par_view,
        None,
        &collapsed,
        GroupingMode::Dependency,
        Some((78, 20)),
    );
    let p1_w = collapsed_graph.node("worker:p1").unwrap();
    let p1_v = collapsed_graph.node("verifier:p1").unwrap();
    assert!(p1_w.visible && p1_w.collapsed);
    assert!(p1_v.visible);

    // Filter compaction
    let filtered_graph = Graph::new_with_viewport(
        &par_view,
        Some("p1"),
        &Default::default(),
        GroupingMode::Dependency,
        Some((78, 20)),
    );
    assert!(filtered_graph.node("worker:p1").unwrap().visible);
    assert!(!filtered_graph.node("worker:p2").unwrap().visible);
}

#[test]
fn test_adaptive_graph_collision_and_routing_safety() {
    use antislop::lean::tui::graph::{
        Canvas, Graph, GroupingMode, HEIGHT, WIDTH, compute_progress_overlay_with_nodes,
    };
    use antislop::lean::tui::view::{Link, TaskRow, View};

    let tasks = vec![
        TaskRow {
            id: "t1".into(),
            title: "T1".into(),
            depends_on: vec![],
            ..TaskRow::default()
        },
        TaskRow {
            id: "t2".into(),
            title: "T2".into(),
            depends_on: vec!["t1".into()],
            ..TaskRow::default()
        },
        TaskRow {
            id: "t3".into(),
            title: "T3".into(),
            depends_on: vec!["t1".into()],
            ..TaskRow::default()
        },
        TaskRow {
            id: "t4".into(),
            title: "T4".into(),
            depends_on: vec!["t2".into(), "t3".into()],
            ..TaskRow::default()
        },
    ];
    let mut view = View::default();
    view.tasks = tasks;
    view.links.insert(
        ("worker:t1".into(), "worker:t4".into()),
        Link {
            count: 1,
            last: std::time::Instant::now(),
        },
    );

    for &(w, h) in &[(78, 20), (48, 20), (77, 26), (104, 46)] {
        let graph = Graph::new_with_viewport(
            &view,
            None,
            &Default::default(),
            GroupingMode::Dependency,
            Some((w, h)),
        );

        // 1. Collision check: No two visible nodes may overlap
        let visible_nodes: Vec<_> = graph.nodes.iter().filter(|n| n.visible).collect();
        for i in 0..visible_nodes.len() {
            for j in (i + 1)..visible_nodes.len() {
                let a = visible_nodes[i];
                let b = visible_nodes[j];
                let separate = a.x + WIDTH <= b.x
                    || b.x + WIDTH <= a.x
                    || a.y + HEIGHT <= b.y
                    || b.y + HEIGHT <= a.y;
                assert!(
                    separate,
                    "Nodes {} at ({},{}) and {} at ({},{}) collide!",
                    a.key, a.x, a.y, b.key, b.x, b.y
                );
            }
        }

        // 2. Routing safety: Every visible edge route under every filter must never intersect any visible card
        for filter in [
            Filter::All,
            Filter::Supervises,
            Filter::Depends,
            Filter::Verifies,
            Filter::Mail,
        ] {
            let routes = graph.edge_routes(filter);
            for (from, to, path) in routes {
                for node in &visible_nodes {
                    assert!(
                        !Graph::route_intersects_card(&path, node),
                        "Edge {from} -> {to} (filter {filter:?}) intersects card {} at ({}, {}) in terminal {w}x{h}!\nPath: {path:?}",
                        node.key,
                        node.x,
                        node.y
                    );
                }
            }
        }

        // 3. Rendering into Canvas and verifying bounded window
        let mut canvas = Canvas::new_window(0, 0, w, h);
        graph.render(&view, &mut canvas, Filter::All, "worker:t1");
    }

    // 3. Progress overlay yields when cards occupy all candidate corners
    let inner = Rect::new(0, 0, 78, 20);
    let full_nodes = [
        antislop::lean::tui::graph::Node::new("br".into(), 50, 14),
        antislop::lean::tui::graph::Node::new("tr".into(), 50, 0),
        antislop::lean::tui::graph::Node::new("tl".into(), 0, 0),
        antislop::lean::tui::graph::Node::new("bl".into(), 0, 14),
    ];
    let refs: Vec<&_> = full_nodes.iter().collect();
    let overlay = compute_progress_overlay_with_nodes(inner, None, &refs, (0, 0), 24, 6);
    assert_eq!(
        overlay, None,
        "Overlay must yield (return None) when all corners collide"
    );
}

#[test]
fn task_card_mouse_selection_uses_one_identity_for_both_phases() {
    use antislop::lean::tui::{graph::Graph, view::TaskRow};
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let mut app = App::test_app();
    app.view.tasks = vec![
        TaskRow {
            id: "t1".into(),
            status: "working".into(),
            ..Default::default()
        },
        TaskRow {
            id: "t2".into(),
            status: "verifying".into(),
            ..Default::default()
        },
    ];
    app.focus = Focus::Graph;
    app.selected = "worker:t1".into();
    let mut terminal = Terminal::new(TestBackend::new(160, 70)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let inner = app.graph_area.unwrap();
    let graph = Graph::new_with_viewport(
        &app.view,
        None,
        &app.graph_collapsed,
        app.graph_grouping,
        app.graph_viewport,
    );
    let node = graph.node("verifier:t2").unwrap();
    app.mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: inner.x + node.x + 3 - app.offset.0,
        row: inner.y + node.y + 2 - app.offset.1,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(
        app.selected, "worker:t2",
        "clicking verifier state selects its task"
    );
    app.toggle_selected_collapse();
    app.select_node("verifier:t2".into());
    assert_eq!(app.selected, "worker:t2");
    assert!(
        app.graph_collapsed.contains("t2"),
        "role alias preserves collapse"
    );
    terminal.draw(|frame| app.draw(frame)).unwrap();
    assert!(app.graph_collapsed.contains("t2"));
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let details = app.details.as_ref().unwrap();
    assert_eq!(details.key, "worker:t2");
    assert!(
        antislop::lean::tui::details::tabs_for(&details.key)
            .contains(&antislop::lean::tui::details::CHECKS)
    );
    app.key(KeyEvent::new(KeyCode::Char('4'), KeyModifiers::NONE));
    assert_eq!(
        app.details.as_ref().unwrap().tab,
        antislop::lean::tui::details::CHECKS
    );
    app.details = None;
    let mut tiny = Terminal::new(TestBackend::new(11, 4)).unwrap();
    tiny.draw(|frame| app.draw(frame)).unwrap();
    assert!(
        app.graph_area.is_none(),
        "small-terminal guard clears stale mouse bounds"
    );
}

#[test]
fn test_adaptive_graph_navigation_and_resize_stability() {
    use antislop::lean::tui::graph::Graph;
    use antislop::lean::tui::view::TaskRow;

    let (mut app, _rx) = App::test_app_with_client(Client::at(std::path::Path::new("/dev/null")));
    app.view.tasks = vec![
        TaskRow {
            id: "t1".into(),
            title: "T1".into(),
            ..TaskRow::default()
        },
        TaskRow {
            id: "t2".into(),
            title: "T2".into(),
            ..TaskRow::default()
        },
        TaskRow {
            id: "t3".into(),
            title: "T3".into(),
            ..TaskRow::default()
        },
    ];
    for task in &mut app.view.tasks {
        task.status = "working".into();
    }
    app.focus = Focus::Graph;
    app.selected = "worker:t1".into();

    // 1. Initial draw on 80x24 terminal
    let mut term_small = Terminal::new(TestBackend::new(80, 24)).unwrap();
    term_small.draw(|f| app.draw(f)).unwrap();

    let initial_viewport = app.graph_viewport.expect("viewport set after draw");
    assert_eq!(initial_viewport, (76, 18));

    // Capture node coordinates
    let g1 = Graph::new_with_viewport(
        &app.view,
        None,
        &app.graph_collapsed,
        app.graph_grouping,
        app.graph_viewport,
    );
    let coords_initial: Vec<(String, u16, u16)> =
        g1.nodes.iter().map(|n| (n.key.clone(), n.x, n.y)).collect();

    // 2. Selection changes and activity updates do NOT shift nodes
    app.selected = "worker:t2".into();
    app.view.activity = Some("Processing data...".into());
    term_small.draw(|f| app.draw(f)).unwrap();

    let g2 = Graph::new_with_viewport(
        &app.view,
        None,
        &app.graph_collapsed,
        app.graph_grouping,
        app.graph_viewport,
    );
    let coords_after: Vec<(String, u16, u16)> =
        g2.nodes.iter().map(|n| (n.key.clone(), n.x, n.y)).collect();
    assert_eq!(
        coords_initial, coords_after,
        "Selection or activity must not displace graph nodes"
    );

    // 3. Arrow navigation
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.selected, "worker:t3");

    // 4. Resize terminal to 160x50: viewport dynamically adapts
    let mut term_large = Terminal::new(TestBackend::new(160, 50)).unwrap();
    term_large.draw(|f| app.draw(f)).unwrap();
    let resized_viewport = app.graph_viewport.expect("viewport updated after resize");
    assert_eq!(resized_viewport, (102, 44));
}

#[test]
fn test_canvas_memory_bounded_for_large_graphs() {
    use antislop::lean::tui::graph::{Canvas, Graph, GroupingMode};
    use antislop::lean::tui::view::TaskRow;

    // Generate a 500-task graph
    let mut tasks = Vec::with_capacity(500);
    for i in 0..500 {
        tasks.push(TaskRow {
            id: format!("task_{i}"),
            title: format!("Task {i}"),
            depends_on: if i > 0 {
                vec![format!("task_{}", i - 1)]
            } else {
                vec![]
            },
            ..TaskRow::default()
        });
    }
    let mut view = antislop::lean::tui::view::View::default();
    view.tasks = tasks;

    // Viewport-based layout for 120x30 terminal (inner: 77x26)
    let graph = Graph::new_with_viewport(
        &view,
        None,
        &Default::default(),
        GroupingMode::Dependency,
        Some((77, 26)),
    );

    // Windowed canvas allocates strictly O(inner.width * inner.height) = 77 * 26 = 2002 cells
    let mut window_canvas = Canvas::new_window(0, 0, 77, 26);
    graph.render(&view, &mut window_canvas, Filter::All, "worker:task_0");

    let lines = window_canvas.lines();
    assert_eq!(lines.len(), 26);

    // Canvas::new cap check: 60,000 x 60,000 does not OOM
    let capped_canvas = Canvas::new(60_000, 60_000);
    assert_eq!(capped_canvas.lines().len(), 1024);
}

#[test]
fn test_adaptive_graph_non_adjacent_same_column_bypass_routing() {
    use antislop::lean::tui::graph::{Filter, Graph, GroupingMode, HEIGHT, WIDTH};
    use antislop::lean::tui::view::{ORCHESTRATOR, TaskRow, View};

    // At 78 columns two cards fit side by side. Row 0 holds the plan verifier
    // and the orchestrator, row 1 the auditor, then the workers wrap two per row:
    //    worker:t1 worker:t2 / worker:t3 worker:t4.
    // The orchestrator and worker:t4 share a column with worker:t2 between them.
    let mut view = View::default();
    view.tasks = (1..=4)
        .map(|i| TaskRow {
            id: format!("t{i}"),
            title: format!("T{i}"),
            ..TaskRow::default()
        })
        .collect();

    let graph = Graph::new_with_viewport(
        &view,
        None,
        &Default::default(),
        GroupingMode::Dependency,
        Some((78, 20)),
    );

    let orch = graph.node(ORCHESTRATOR).expect("orchestrator present");
    let w2 = graph.node("worker:t2").expect("worker:t2 present");
    let w4 = graph.node("worker:t4").expect("worker:t4 present");

    assert_eq!(orch.x, w2.x, "Orchestrator and worker:t2 in same column");
    assert_eq!(orch.x, w4.x, "Orchestrator and worker:t4 in same column");
    assert_eq!(orch.y, 0);
    assert!(orch.y < w2.y && w2.y < w4.y);

    // Supervision edge route: ORCHESTRATOR -> worker:t4
    let sup_routes = graph.edge_routes(Filter::Supervises);
    let orch_to_w4 = sup_routes
        .iter()
        .find(|(from, to, _)| *from == ORCHESTRATOR && *to == "worker:t4")
        .expect("supervision edge ORCHESTRATOR -> worker:t4 exists");

    let path = &orch_to_w4.2;
    // Must bypass worker:t2 via side corridor instead of passing straight through it
    assert!(
        !Graph::route_intersects_card(path, w2),
        "Supervision edge must NOT intersect worker:t2!"
    );
    assert!(
        !Graph::route_intersects_card(path, orch),
        "Supervision edge must NOT intersect Orchestrator interior!"
    );
    assert!(
        !Graph::route_intersects_card(path, w4),
        "Supervision edge must NOT intersect worker:t4 interior!"
    );

    // Check specific path points: corridor must run at from.right() + GAP_X / 2
    let half = 6 / 2;
    let expected_lane = orch.x + WIDTH + half;
    assert_eq!(path[0], (orch.x + WIDTH, orch.y + HEIGHT / 2));
    assert_eq!(path[1], (expected_lane, orch.y + HEIGHT / 2));
    assert_eq!(path[2], (expected_lane, w4.y + HEIGHT / 2));
    assert_eq!(path[3], (w4.x + WIDTH, w4.y + HEIGHT / 2));
}

#[test]
fn test_adaptive_graph_admin_and_task_compaction_and_column_selection() {
    use antislop::lean::tui::graph::{Graph, GroupingMode};
    use antislop::lean::tui::view::{AUDITOR, ORCHESTRATOR, PLAN_VERIFIER, TaskRow, View};

    let tasks = vec![
        TaskRow {
            id: "t1".into(),
            title: "Task 1".into(),
            depends_on: vec![],
            status: "proposed".into(),
            ..TaskRow::default()
        },
        TaskRow {
            id: "t2".into(),
            title: "Task 2".into(),
            depends_on: vec!["t1".into()],
            status: "approved".into(),
            ..TaskRow::default()
        },
        TaskRow {
            id: "t3".into(),
            title: "Task 3".into(),
            depends_on: vec!["t2".into()],
            status: "approved".into(),
            ..TaskRow::default()
        },
    ];
    let mut view = View::default();
    view.tasks = tasks;

    // 1. Filter out all administrative nodes (filter: "task 1")
    // When admin nodes are hidden, row 0 must be compacted: worker:t1 sits at row 0
    let filtered_graph = Graph::new_with_viewport(
        &view,
        Some("task 1"),
        &Default::default(),
        GroupingMode::Dependency,
        Some((78, 20)),
    );

    assert!(!filtered_graph.node(PLAN_VERIFIER).unwrap().visible);
    assert!(!filtered_graph.node(ORCHESTRATOR).unwrap().visible);
    assert!(!filtered_graph.node(AUDITOR).unwrap().visible);

    let w1 = filtered_graph.node("worker:t1").unwrap();
    assert!(w1.visible);
    assert_eq!(w1.x, 3, "Column 0 used");
    assert_eq!(w1.y, 0, "Compacted to row 0 when admin nodes hidden!");

    let v1 = filtered_graph.node("verifier:t1").unwrap();
    assert!(v1.visible);
    assert_eq!(v1.y, w1.y, "Verifier address resolves to the task card");

    // 2. Column selection according to visible tasks:
    // Only "task 3" visible (raw topological depth was 2).
    // Visible task selection compacts active column depth to column 0!
    let t3_graph = Graph::new_with_viewport(
        &view,
        Some("task 3"),
        &Default::default(),
        GroupingMode::Dependency,
        Some((78, 20)),
    );
    let w3 = t3_graph.node("worker:t3").unwrap();
    assert!(w3.visible);
    assert_eq!(
        w3.x, 3,
        "Task 3 compacted to column 0 (no empty preceding columns)!"
    );
    assert_eq!(
        w3.y, 0,
        "Task 3 compacted to row 0 (no empty preceding rows)!"
    );

    // 3. Status grouping column compaction:
    // Filter "approved": only t2 and t3 match. Raw status column was 2.
    // Must be compacted to column 0!
    let status_graph = Graph::new_with_viewport(
        &view,
        Some("approved"),
        &Default::default(),
        GroupingMode::Status,
        Some((78, 20)),
    );
    let w2_status = status_graph.node("worker:t2").unwrap();
    assert!(w2_status.visible);
    assert_eq!(
        w2_status.x, 3,
        "Approved tasks compacted to column 0 in Status mode!"
    );
}

#[test]
fn test_adaptive_graph_height_utilization_and_scroll_limitation() {
    use antislop::lean::tui::graph::{Graph, GroupingMode};
    use antislop::lean::tui::view::{TaskRow, View};

    // 6 tasks in two stages
    let tasks = vec![
        TaskRow {
            id: "a1".into(),
            title: "A1".into(),
            ..TaskRow::default()
        },
        TaskRow {
            id: "a2".into(),
            title: "A2".into(),
            ..TaskRow::default()
        },
        TaskRow {
            id: "a3".into(),
            title: "A3".into(),
            ..TaskRow::default()
        },
        TaskRow {
            id: "b1".into(),
            title: "B1".into(),
            depends_on: vec!["a1".into()],
            ..TaskRow::default()
        },
        TaskRow {
            id: "b2".into(),
            title: "B2".into(),
            depends_on: vec!["a2".into()],
            ..TaskRow::default()
        },
        TaskRow {
            id: "b3".into(),
            title: "B3".into(),
            depends_on: vec!["a3".into()],
            ..TaskRow::default()
        },
    ];
    let mut view = View::default();
    view.tasks = tasks;

    // Terminals: 80x24 (h=20), 120x30 (h=26), 160x62 (h=58 — one screen
    // tall enough for the fixed-size cards, five rows at their full height)
    let g_short = Graph::new_with_viewport(
        &view,
        None,
        &Default::default(),
        GroupingMode::Dependency,
        Some((78, 20)),
    );
    let g_mid = Graph::new_with_viewport(
        &view,
        None,
        &Default::default(),
        GroupingMode::Dependency,
        Some((77, 26)),
    );
    let g_tall = Graph::new_with_viewport(
        &view,
        None,
        &Default::default(),
        GroupingMode::Dependency,
        Some((104, 58)),
    );

    let whole_short = count_whole_cards(&g_short, 78, 20);
    let whole_mid = count_whole_cards(&g_mid, 77, 26);
    let whole_tall = count_whole_cards(&g_tall, 104, 58);

    // Height utilization: larger panel height fits strictly more whole readable cards
    assert!(
        whole_mid > whole_short,
        "Height 26 ({whole_mid}) must fit more whole cards than height 20 ({whole_short})"
    );
    assert!(
        whole_tall > whole_mid,
        "Height 58 ({whole_tall}) must fit more whole cards than height 26 ({whole_mid})"
    );

    // Taller height allows all tasks to be visible without vertical scrolling
    assert_eq!(
        whole_tall,
        g_tall.nodes.iter().filter(|n| n.visible).count(),
        "On 160x62 all nodes fit completely without scroll"
    );
}

#[test]
fn test_adaptive_graph_app_comprehensive_matrix_across_all_terminals() {
    use antislop::lean::tui::graph::{Filter, Graph};
    use antislop::lean::tui::view::TaskRow;

    // Terminal sizes to test:
    // 80x24 (compact: 78x20)
    // 85x24 (split: 48x20)
    // 120x30 (split: 77x26)
    // 160x50 (split: 104x46)
    let terminal_configs = [
        ("80x24", 80u16, 24u16, 76u16, 18u16),
        ("85x24", 85u16, 24u16, 46u16, 18u16),
        ("120x30", 120u16, 30u16, 75u16, 24u16),
        ("160x50", 160u16, 50u16, 102u16, 44u16),
    ];

    for &(term_name, term_w, term_h, inner_w, inner_h) in &terminal_configs {
        let (mut app, _rx) =
            App::test_app_with_client(Client::at(std::path::Path::new("/dev/null")));
        app.focus = Focus::Graph;

        // 1. Long chain: c1 -> c2 -> c3 -> c4
        app.view.tasks = vec![
            TaskRow {
                id: "c1".into(),
                title: "Chain 1".into(),
                depends_on: vec![],
                ..TaskRow::default()
            },
            TaskRow {
                id: "c2".into(),
                title: "Chain 2".into(),
                depends_on: vec!["c1".into()],
                ..TaskRow::default()
            },
            TaskRow {
                id: "c3".into(),
                title: "Chain 3".into(),
                depends_on: vec!["c2".into()],
                ..TaskRow::default()
            },
            TaskRow {
                id: "c4".into(),
                title: "Chain 4".into(),
                depends_on: vec!["c3".into()],
                ..TaskRow::default()
            },
        ];
        app.selected = "worker:c1".into();

        let mut term = Terminal::new(TestBackend::new(term_w, term_h)).unwrap();
        term.draw(|f| app.draw(f)).unwrap();

        assert_eq!(
            app.graph_viewport,
            Some((inner_w, inner_h)),
            "Viewport matches terminal {term_name}"
        );

        let g_chain = Graph::new_with_viewport(
            &app.view,
            app.graph_filter.as_deref(),
            &app.graph_collapsed,
            app.graph_grouping,
            app.graph_viewport,
        );
        let visible_chain: Vec<_> = g_chain.nodes.iter().filter(|n| n.visible).collect();

        // Verify routing safety on chains
        for filter in [
            Filter::All,
            Filter::Supervises,
            Filter::Depends,
            Filter::Verifies,
        ] {
            for (from, to, path) in g_chain.edge_routes(filter) {
                for node in &visible_chain {
                    assert!(
                        !Graph::route_intersects_card(&path, node),
                        "Chain in {term_name}: Edge {from}->{to} ({filter:?}) intersects card {}",
                        node.key
                    );
                }
            }
        }

        // 2. Branching: b1 root branching to b2, b3, b4
        app.view.tasks = vec![
            TaskRow {
                id: "b1".into(),
                title: "Branch root".into(),
                depends_on: vec![],
                ..TaskRow::default()
            },
            TaskRow {
                id: "b2".into(),
                title: "Branch child 1".into(),
                depends_on: vec!["b1".into()],
                ..TaskRow::default()
            },
            TaskRow {
                id: "b3".into(),
                title: "Branch child 2".into(),
                depends_on: vec!["b1".into()],
                ..TaskRow::default()
            },
            TaskRow {
                id: "b4".into(),
                title: "Branch child 3".into(),
                depends_on: vec!["b1".into()],
                ..TaskRow::default()
            },
        ];
        term.draw(|f| app.draw(f)).unwrap();
        let g_branch = Graph::new_with_viewport(
            &app.view,
            app.graph_filter.as_deref(),
            &app.graph_collapsed,
            app.graph_grouping,
            app.graph_viewport,
        );
        let visible_branch: Vec<_> = g_branch.nodes.iter().filter(|n| n.visible).collect();

        for filter in [
            Filter::All,
            Filter::Supervises,
            Filter::Depends,
            Filter::Verifies,
        ] {
            for (from, to, path) in g_branch.edge_routes(filter) {
                for node in &visible_branch {
                    assert!(
                        !Graph::route_intersects_card(&path, node),
                        "Branch in {term_name}: Edge {from}->{to} ({filter:?}) intersects card {}",
                        node.key
                    );
                }
            }
        }

        // 3. Cycles: cy1 <-> cy2
        app.view.tasks = vec![
            TaskRow {
                id: "cy1".into(),
                title: "Cycle 1".into(),
                depends_on: vec!["cy2".into()],
                ..TaskRow::default()
            },
            TaskRow {
                id: "cy2".into(),
                title: "Cycle 2".into(),
                depends_on: vec!["cy1".into()],
                ..TaskRow::default()
            },
        ];
        term.draw(|f| app.draw(f)).unwrap();
        let g_cycle = Graph::new_with_viewport(
            &app.view,
            app.graph_filter.as_deref(),
            &app.graph_collapsed,
            app.graph_grouping,
            app.graph_viewport,
        );
        assert_eq!(g_cycle.nodes.len(), 3 + 2);

        // 4. Filtering and compaction through App:
        app.set_graph_filter(Some("cy1".into()));
        term.draw(|f| app.draw(f)).unwrap();
        let g_filtered = Graph::new_with_viewport(
            &app.view,
            app.graph_filter.as_deref(),
            &app.graph_collapsed,
            app.graph_grouping,
            app.graph_viewport,
        );
        let cy1_w = g_filtered.node("worker:cy1").unwrap();
        assert!(cy1_w.visible);
        assert_eq!(cy1_w.x, 3, "Filtered cy1 compacted to column 0");
        assert_eq!(cy1_w.y, 0, "Filtered cy1 compacted to row 0");

        // 5. Collapsing:
        app.set_graph_filter(None);
        app.graph_collapsed.insert("cy1".into());
        term.draw(|f| app.draw(f)).unwrap();
        let g_collapsed = Graph::new_with_viewport(
            &app.view,
            app.graph_filter.as_deref(),
            &app.graph_collapsed,
            app.graph_grouping,
            app.graph_viewport,
        );
        assert!(g_collapsed.node("worker:cy1").unwrap().collapsed);
        assert!(
            g_collapsed.node("verifier:cy1").unwrap().visible,
            "Collapsed task remains addressable by either role"
        );

        // 6. Stability on selection and activity updates:
        let initial_coords: Vec<_> = g_collapsed
            .nodes
            .iter()
            .map(|n| (n.key.clone(), n.x, n.y))
            .collect();
        app.selected = "worker:cy2".into();
        app.view.activity = Some("Work in progress".into());
        term.draw(|f| app.draw(f)).unwrap();
        let after_coords: Vec<_> = Graph::new_with_viewport(
            &app.view,
            app.graph_filter.as_deref(),
            &app.graph_collapsed,
            app.graph_grouping,
            app.graph_viewport,
        )
        .nodes
        .iter()
        .map(|n| (n.key.clone(), n.x, n.y))
        .collect();
        assert_eq!(
            initial_coords, after_coords,
            "Selection/activity must not displace nodes in {term_name}"
        );
    }
}

#[test]
fn test_workspace_trust_modal_lifecycle() {
    let mut app = App::test_app();
    app.input = InputBuffer::with_text("initial draft");
    app.require_trust("/Users/test/untrusted-repo");

    let mut term = Terminal::new(TestBackend::new(90, 24)).unwrap();
    term.draw(|f| app.draw(f)).unwrap();
    assert!(!term.backend().cursor_visible());
    let rendered: String = term
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(rendered.contains("Workspace trust required"));
    assert!(rendered.contains("Do you trust the contents of this directory?"));
    assert!(rendered.contains("/Users/test/untrusted-repo"));

    // Typing ordinary keys or pasting is blocked while trust prompt is active
    assert!(app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)));
    app.paste("injected text");
    assert_eq!(app.input.text, "initial draft");
    assert!(app.trust_prompt.is_some());

    // Pressing 'n' or Esc rejects and exits (returns false)
    let mut reject_app = App::test_app();
    reject_app.require_trust("/Users/test/untrusted-repo");
    assert!(!reject_app.key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE)));

    // Pressing 'y' or Enter trusts the workspace and dismisses the modal
    assert!(app.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE)));
    assert!(app.trust_prompt.is_none());
    assert_eq!(app.notice.as_deref(), Some("workspace directory trusted"));

    // Normal typing works after trusting
    assert!(app.key(KeyEvent::new(KeyCode::Char('!'), KeyModifiers::NONE)));
    assert_eq!(app.input.text, "initial draft!");
}

#[test]
fn task_events_attach_feedback_only_for_their_round_and_recovery_clears_it() {
    use antislop::lean::tui::view::View;
    let mut view = View::default();
    let ev = |seq: u64, kind: &str, payload: Value| json!({"seq": seq, "kind": kind, "payload": payload});
    view.apply(&ev(
        1,
        "plan_proposed",
        json!({"plan": "p1", "tasks": [{"id": "t1", "title": "Store"}]}),
    ));
    // A round announced without feedback shows no effect and no counters.
    view.apply(&ev(
        2,
        "task",
        json!({"id": "t1", "task": "u1", "status": "working", "round": 1}),
    ));
    assert!(view.tasks[0].effect.is_none());
    assert!(view.tasks[0].obligations.is_none());
    assert!(view.tasks[0].integration.is_none());
    // Feedback stamped for another round is stale and must not attach.
    view.apply(&ev(
        3,
        "task",
        json!({"id": "t1", "task": "u1", "status": "verifying", "round": 1,
               "feedback": {"round": 0, "effect": "x", "coverage": {"total": 1},
                            "integration": {"outcome": "committed"}}}),
    ));
    assert!(view.tasks[0].effect.is_none());
    assert!(view.tasks[0].integration.is_none());
    // Feedback for the announced round attaches.
    view.apply(&ev(
        4,
        "task",
        json!({"id": "t1", "task": "u1", "status": "approved", "round": 1,
               "feedback": {"round": 1, "effect": "2 files changed",
                            "coverage": {"total": 3, "ok": 3, "missing": 0},
                            "integration": {"outcome": "applied",
                                            "detail": "applied to the working tree"}}}),
    ));
    assert_eq!(view.tasks[0].effect.as_deref(), Some("2 files changed"));
    assert_eq!(view.tasks[0].obligations.as_ref().unwrap()["total"], 3);
    assert_eq!(
        view.tasks[0].integration.as_ref().unwrap()["outcome"],
        "applied"
    );
    // A new round without feedback clears what the old one showed.
    view.apply(&ev(
        5,
        "task",
        json!({"id": "t1", "task": "u1", "status": "working", "round": 2, "feedback": null}),
    ));
    assert!(view.tasks[0].effect.is_none());
    assert!(view.tasks[0].obligations.is_none());
    assert!(view.tasks[0].integration.is_none());
    // The integration event writes the raw reason and detail onto the row.
    view.apply(&ev(
        6,
        "integration",
        json!({"id": "t1", "title": "Store", "status": "needs_user",
               "detail": "conflicts with the current branch in src/a.rs"}),
    ));
    assert_eq!(
        view.tasks[0].reason.as_deref(),
        Some("conflicts with the current branch in src/a.rs")
    );
    assert_eq!(
        view.tasks[0].integration.as_ref().unwrap()["source"],
        "integration"
    );
    // task_recovery is keyed by the task's database id.
    view.apply(&ev(
        7,
        "task_recovery",
        json!({"task": "u1", "action": "retry_with_guidance", "status": "queued",
               "detail": "retrying task with guidance"}),
    ));
    assert_eq!(view.tasks[0].status, "queued");
    assert!(view.tasks[0].integration.is_none());
    view.apply(&ev(
        8,
        "task_recovery",
        json!({"task": "u1", "action": "keep_on_branch", "status": "approved",
               "branch": "antislop/u1", "detail": "change preserved on branch antislop/u1"}),
    ));
    assert_eq!(
        view.tasks[0].integration.as_ref().unwrap()["outcome"],
        "left_on_branch"
    );
    assert_eq!(
        view.tasks[0].integration.as_ref().unwrap()["branch"],
        "antislop/u1"
    );
}

#[test]
fn rich_cards_show_round_cause_effect_coverage_and_integration_within_budget() {
    use antislop::lean::tui::{
        Details,
        graph::{Canvas, Filter, Graph, HEIGHT, WIDTH},
        view::{Agent, View},
    };
    let mut view = View::default();
    let ev = |seq: u64, kind: &str, payload: Value| json!({"seq": seq, "kind": kind, "payload": payload});
    view.apply(&ev(
        1,
        "plan_proposed",
        json!({"plan": "p1", "tasks": [
            {"id": "t1", "title": "Store", "summary": "Fix the storage path.",
             "verifier": {"summary": "Check the storage path."}},
            {"id": "t2", "title": "API", "depends_on": ["t1"]}
        ]}),
    ));
    let card = |view: &View, key: &str| -> (String, u16) {
        let graph = Graph::new(view);
        let node = graph.node(key).expect("node");
        let height = node.height;
        let mut canvas = Canvas::new_window(node.x, node.y, WIDTH, height);
        graph.render(view, &mut canvas, Filter::All, key);
        (canvas.lines().join("\n"), height)
    };
    // Before any round starts, no card claims one.
    let (text, _) = card(&view, "worker:t1");
    assert!(text.contains("proposed"), "{text}");
    assert!(
        !text.contains(" r1"),
        "no quiet r1 before the first round: {text}"
    );
    assert!(
        !text.contains("effect"),
        "no effect line before a round: {text}"
    );
    // Worker working round 1: both roles show the round.
    view.apply(&ev(
        2,
        "task",
        json!({"id": "t1", "task": "u1", "status": "working", "round": 1, "feedback": null}),
    ));
    view.agents.insert(
        "worker:t1".into(),
        Agent {
            running: true,
            provider: "codex".into(),
            activity: "cargo check".into(),
            ..Agent::default()
        },
    );
    let (text, _) = card(&view, "worker:t1");
    assert!(text.contains("working r1"), "{text}");
    let (text, _) = card(&view, "verifier:t1");
    assert!(text.contains("waits for worker r1"), "{text}");
    // Verifier checking: separate activity lines per role.
    view.apply(&ev(
        3,
        "task",
        json!({"id": "t1", "task": "u1", "status": "verifying", "round": 1, "feedback": null}),
    ));
    view.agents.insert(
        "verifier:t1".into(),
        Agent {
            running: true,
            provider: "agy".into(),
            activity: "cargo test".into(),
            ..Agent::default()
        },
    );
    let (worker_now, _) = card(&view, "worker:t1");
    assert!(worker_now.contains("verifier r1"), "{worker_now}");
    assert!(
        worker_now.contains("cargo test"),
        "task shows its active verifier"
    );
    let (verifier_now, _) = card(&view, "verifier:t1");
    assert!(verifier_now.contains("checking r1"), "{verifier_now}");
    assert!(
        verifier_now.contains("· agy"),
        "verifier shows its own agent: {verifier_now}"
    );
    assert!(
        !verifier_now.contains("codex"),
        "no worker agent on the verifier card"
    );
    // Approved with feedback: effect on the worker card, coverage and the
    // one landing label on the bottom line; details keep the same words.
    view.apply(&ev(
        4,
        "task",
        json!({"id": "t1", "task": "u1", "status": "approved", "round": 1,
               "feedback": {"round": 1,
                            "effect": "2 files changed — src/a.rs",
                            "coverage": {"total": 3, "ok": 2, "issue": 0, "blocked": 0,
                                         "na": 0, "missing": 1, "unknown": []},
                            "integration": {"outcome": "committed",
                                            "detail": "committed as 1234abc"}}}),
    ));
    let (worker, worker_h) = card(&view, "worker:t1");
    assert!(worker.contains("effect: 2 files"), "{worker}");
    assert!(worker.contains("committed"), "{worker}");
    assert_eq!(worker_h, HEIGHT, "the card keeps its fixed height budget");
    let (verifier, verifier_h) = card(&view, "verifier:t1");
    assert!(verifier.contains("2/3"), "coverage counters: {verifier}");
    assert!(verifier.contains("committed"), "{verifier}");
    assert_eq!(verifier_h, HEIGHT);
    for (_, _, path) in Graph::new(&view).edge_routes(Filter::All) {
        for node in Graph::new(&view).nodes.iter().filter(|n| n.visible) {
            assert!(!antislop::lean::tui::graph::Graph::route_intersects_card(
                &path, node
            ));
        }
    }
    let mut details = Details::new("worker:t1".into());
    details.show_info = true;
    let full: String = details
        .lines(&view)
        .into_iter()
        .map(|(_, text)| text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(full.contains("round r1"), "{full}");
    assert!(
        full.contains("Integration: committed · committed as 1234abc"),
        "details repeat the card's reading: {full}"
    );
    assert!(full.contains("Obligations: 2 of 3 accounted"), "{full}");
    assert!(
        full.contains("done: committed"),
        "details state the next step honestly: {full}"
    );
    // A queued task whose dependency failed says which one it waits for,
    // and never guesses a limit or a write conflict.
    view.apply(&ev(
        5,
        "task",
        json!({"id": "t1", "task": "u1", "status": "failed", "round": 1, "feedback": null,
               "reason": "the verifier repeated itself"}),
    ));
    view.apply(&ev(
        6,
        "task",
        json!({"id": "t2", "task": "u2", "status": "queued", "round": 0, "feedback": null}),
    ));
    let (api, _) = card(&view, "worker:t2");
    assert!(api.contains("D:t1"), "dependency id on the card: {api}");
    assert!(!api.contains("limit") && !api.contains("conflict"), "{api}");
    let mut queued_details = Details::new("worker:t2".into());
    queued_details.show_info = true;
    let queued_full: String = queued_details
        .lines(&view)
        .into_iter()
        .map(|(_, text)| text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        queued_full.contains("next: waits for t1"),
        "details name the concrete dependency: {queued_full}"
    );
    let (failed, _) = card(&view, "worker:t1");
    assert!(
        failed.contains("failed r1") && failed.contains("the verifier repeated"),
        "state alone, cause on its own line: {failed}"
    );
    let mut failed_details = Details::new("worker:t1".into());
    failed_details.show_info = true;
    let cause: String = failed_details
        .lines(&view)
        .into_iter()
        .map(|(_, text)| text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        cause.contains("the verifier repeated itself"),
        "details keep the whole cause: {cause}"
    );
    // A failed task with no reported cause says so instead of inventing one.
    view.apply(&ev(
        7,
        "task",
        json!({"id": "t2", "task": "u2", "status": "failed", "round": 1, "feedback": null}),
    ));
    let (quiet, _) = card(&view, "worker:t2");
    assert!(
        quiet.contains("failed r1") && quiet.contains("cause not reported"),
        "an unexplained stop says so on its own line: {quiet}"
    );
    let mut quiet_details = Details::new("worker:t2".into());
    quiet_details.show_info = true;
    let quiet_full: String = quiet_details
        .lines(&view)
        .into_iter()
        .map(|(_, text)| text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        quiet_full.contains("Reason: cause not reported"),
        "an unexplained stop says so: {quiet_full}"
    );
}

#[test]
fn card_geometry_is_identical_across_activity_effect_verdict_retry_and_late_events() {
    use antislop::lean::tui::{
        graph::{Canvas, Filter, Graph, GroupingMode, HEIGHT, WIDTH},
        view::{Agent, View},
    };
    let ev = |seq: u64, kind: &str, payload: Value| json!({"seq": seq, "kind": kind, "payload": payload});
    let mut view = View::default();
    view.apply(&ev(
        1,
        "plan_proposed",
        json!({"plan": "p1", "tasks": [
            {"id": "t1", "title": "Store", "summary": "Fix storage.",
             "verifier": {"summary": "Check storage."}},
            {"id": "t2", "title": "API", "depends_on": ["t1"],
             "summary": "Expose the corrected storage behind a small HTTP surface a verifier can read in one pass, with careful unicode 👩‍💻 everywhere.",
             "verifier": {"summary": "Verify the 👩‍💻 surface wrapped past every card edge twice over turtles e\u{301} until only an ellipsis ends the last visible line."}},
        ]}),
    ));
    let geometry = |view: &View, grouping: GroupingMode| -> Vec<(String, u16, u16, u16, bool)> {
        Graph::new_with_options(view, None, &Default::default(), grouping)
            .nodes
            .iter()
            .map(|node| (node.key.clone(), node.x, node.y, node.height, node.visible))
            .collect()
    };
    let before = geometry(&view, GroupingMode::Dependency);
    // Every node — admin, worker and verifier alike — starts at the fixed
    // height, and stays there through every event below.
    assert!(before.iter().all(|(_, _, _, h, _)| *h == HEIGHT));

    // Activity: the round starts and the agent runs.
    view.apply(&ev(
        2,
        "task",
        json!({"id": "t1", "task": "u1", "status": "working", "round": 1, "feedback": null}),
    ));
    view.agents.insert(
        "worker:t1".into(),
        Agent {
            running: true,
            provider: "codex".into(),
            activity: "cargo check --workspace --all-targets".into(),
            ..Default::default()
        },
    );
    assert_eq!(
        geometry(&view, GroupingMode::Dependency),
        before,
        "activity moves no card"
    );
    // Working on round 2 under Status grouping keeps its column too, and
    // no card ever changes height there either.
    let status_before = geometry(&view, GroupingMode::Status);
    view.apply(&ev(
        3,
        "task",
        json!({"id": "t1", "task": "u1", "status": "verifying", "round": 2, "feedback": null}),
    ));
    let status_now = geometry(&view, GroupingMode::Status);
    assert_eq!(
        status_now
            .iter()
            .map(|(key, _, _, height, _)| (key, height))
            .collect::<Vec<_>>(),
        status_before
            .iter()
            .map(|(key, _, _, height, _)| (key, height))
            .collect::<Vec<_>>(),
        "heights never move"
    );
    assert!(status_now.iter().all(|(_, _, _, h, _)| *h == HEIGHT));

    // Effect + verdict: a cut (fragment) effect of the bounded kind.
    let long_effect = format!("{}…", "fixed 👩‍💻 — src/store.rs ".repeat(12));
    view.apply(&ev(
        4,
        "task",
        json!({"id": "t1", "task": "u1", "status": "approved", "round": 2,
               "feedback": {"round": 2, "effect": long_effect.clone(),
                            "coverage": {"total": 4, "ok": 4, "missing": 0},
                            "integration": {"outcome": "committed", "detail": "as 1234abc"}}}),
    ));
    assert_eq!(
        geometry(&view, GroupingMode::Dependency),
        before,
        "an effect and a verdict move no card"
    );
    let graph = Graph::new(&view);
    let node = graph.node("worker:t1").unwrap();
    let mut canvas = Canvas::new_window(node.x, node.y, WIDTH, node.height);
    graph.render(&view, &mut canvas, Filter::All, "worker:t1");
    let text = canvas.lines().join(" ");
    assert!(
        text.contains("effect: [fragment]"),
        "a cut effect says it is a fragment, front-first: {text}"
    );
    assert!(text.contains("fixed"));

    // Retry: the recovery clears the round's card data; geometry holds.
    view.apply(&ev(
        5,
        "task_recovery",
        json!({"task": "u1", "action": "unblock", "status": "queued",
               "detail": "unblocked and queued again"}),
    ));
    assert!(view.tasks[0].effect.is_none() && view.tasks[0].integration.is_none());
    assert_eq!(
        geometry(&view, GroupingMode::Dependency),
        before,
        "a retry moves no card"
    );

    // Late events: feedback stamped for the finished round must not revive
    // on the new round, and again no card moves.
    view.apply(&ev(
        6,
        "task",
        json!({"id": "t1", "task": "u1", "status": "working", "round": 3,
               "feedback": {"round": 2, "effect": "stale effect"}}),
    ));
    assert_eq!(view.tasks[0].effect, None, "stale feedback is not revived");
    assert_eq!(
        geometry(&view, GroupingMode::Dependency),
        before,
        "a late event moves no card"
    );

    // Short and long descriptions share the one fixed frame: the short
    // card keeps empty slots, the long card cuts with the ellipsis, and
    // both stand at the same height in the same places.
    let (short_text, short_h) = {
        let graph = Graph::new(&view);
        let node = graph.node("verifier:t1").unwrap();
        let mut canvas = Canvas::new_window(node.x, node.y, WIDTH, node.height);
        graph.render(&view, &mut canvas, Filter::All, "verifier:t1");
        (canvas.lines().join("\n"), node.height)
    };
    let (long_text, long_h) = {
        let graph = Graph::new(&view);
        let node = graph.node("verifier:t2").unwrap();
        let mut canvas = Canvas::new_window(node.x, node.y, WIDTH, node.height);
        graph.render(&view, &mut canvas, Filter::All, "verifier:t2");
        (canvas.lines().join("\n"), node.height)
    };
    assert_eq!(short_h, HEIGHT);
    assert_eq!(long_h, HEIGHT, "a long summary grows no card");
    assert!(short_text.contains("Check storage."), "{short_text}");
    assert!(
        long_text.contains('…'),
        "the long summary cuts: {long_text}"
    );
    assert!(
        long_text.contains('👩'),
        "emoji survive the cut: {long_text}"
    );
    for (_, _, path) in Graph::new(&view).edge_routes(Filter::All) {
        for node in Graph::new(&view).nodes.iter().filter(|n| n.visible) {
            assert!(
                !Graph::route_intersects_card(&path, node),
                "fixed cards keep their gaps clear"
            );
        }
    }
}

#[test]
fn details_fetch_is_guarded_by_task_plan_round_and_refetches_once_per_new_round() {
    use antislop::lean::tui::Update;
    let mut app = App::test_app();
    app.apply_update(Update::Events {
        conversation: "test-conv".into(),
        generation: 1,
        events: vec![json!({"seq": 1, "kind": "plan_proposed", "payload": {
            "plan": "p1", "tasks": [{"id": "t1", "title": "Store"}]}})],
        idle: true,
        autonomous: false,
    });
    app.apply_update(Update::Events {
        conversation: "test-conv".into(),
        generation: 1,
        events: vec![json!({"seq": 3, "kind": "task", "payload": {
            "id": "t1", "task": "u1", "status": "working", "round": 1}})],
        idle: false,
        autonomous: false,
    });
    app.open_details("worker:t1".into());
    let issued = app.task_request_id;
    assert_eq!(app.details.as_ref().unwrap().fetched_round, Some(1));
    // A late reply for the previous round is dropped, not shown.
    app.apply_update(Update::Task {
        documents: Default::default(),
        conversation: "test-conv".into(),
        plan: Some("p1".into()),
        generation: 1,
        request_id: issued,
        key: "worker:t1".into(),
        round: 0,
        task: json!({"id": "t1", "report": "old round"}),
    });
    assert!(app.details.as_ref().unwrap().task.is_none());
    // The reply for the round the fetch was issued for is taken.
    app.apply_update(Update::Task {
        documents: Default::default(),
        conversation: "test-conv".into(),
        plan: Some("p1".into()),
        generation: 1,
        request_id: issued,
        key: "worker:t1".into(),
        round: 1,
        task: json!({"id": "t1", "report": "round one"}),
    });
    assert_eq!(
        app.details.as_ref().unwrap().task.as_ref().unwrap()["report"],
        "round one"
    );
    // When the round moves, the open details refetch once — on the event,
    // never per frame.
    app.apply_update(Update::Events {
        conversation: "test-conv".into(),
        generation: 1,
        events: vec![json!({"seq": 3, "kind": "task", "payload": {
            "id": "t1", "task": "u1", "status": "working", "round": 2}})],
        idle: false,
        autonomous: false,
    });
    assert_eq!(app.task_request_id, issued + 1);
    assert_eq!(app.details.as_ref().unwrap().fetched_round, Some(2));
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    assert_eq!(
        app.task_request_id,
        issued + 1,
        "redraws issue no further fetches"
    );
    // The new round's record is accepted and the old one can no longer land.
    app.apply_update(Update::Task {
        documents: Default::default(),
        conversation: "test-conv".into(),
        plan: Some("p1".into()),
        generation: 1,
        request_id: issued + 1,
        key: "worker:t1".into(),
        round: 1,
        task: json!({"id": "t1", "report": "stale again"}),
    });
    assert_eq!(
        app.details.as_ref().unwrap().task.as_ref().unwrap()["report"],
        "round one",
        "a round-1 reply cannot overwrite round-2 details"
    );
    app.apply_update(Update::Task {
        documents: Default::default(),
        conversation: "test-conv".into(),
        plan: Some("p1".into()),
        generation: 1,
        request_id: issued + 1,
        key: "worker:t1".into(),
        round: 2,
        task: json!({"id": "t1", "report": "round two"}),
    });
    assert_eq!(
        app.details.as_ref().unwrap().task.as_ref().unwrap()["report"],
        "round two"
    );
    app.details.as_mut().unwrap().tab = 3; // Checks (Overview and Session are merged)
    let issued = app.task_request_id;
    let check = |id: &str, task: &str, seq| Update::Events {
        conversation: "test-conv".into(),
        generation: 1,
        events: vec![json!({"seq": seq, "kind": "check", "payload": {
            "id": id, "task": task, "round": 2, "check": "V01", "state": "settled"}})],
        idle: false,
        autonomous: false,
    };
    app.apply_update(check("another-task", "u2", 5));
    assert_eq!(
        app.task_request_id, issued,
        "unrelated Checks do not refetch"
    );
    app.apply_update(check("t1", "u1", 6));
    assert_eq!(app.task_request_id, issued + 1, "own Checks refetch once");
    terminal.draw(|f| app.draw(f)).unwrap();
    assert_eq!(
        app.task_request_id,
        issued + 1,
        "redraw does not refetch Checks"
    );
}

#[test]
fn every_reported_landing_earns_its_real_label_on_the_card() {
    use antislop::lean::tasks::Outcome;
    use antislop::lean::tui::{
        graph::{Canvas, Filter, Graph, WIDTH},
        view::View,
    };
    let mut view = View::default();
    view.apply(&json!({"seq": 1, "kind": "plan_proposed", "payload": {
        "plan": "p1", "tasks": [{"id": "t1", "title": "Store"}]}}));
    // The daemon stamps these names through `Outcome::name`; the card must
    // read every one of them back as its own label — never `not reported`.
    for (outcome, label) in [
        (Outcome::Committed, "committed"),
        (Outcome::Applied, "applied uncommitted"),
        (Outcome::LeftOnBranch, "on branch"),
        (Outcome::NoChanges, "no changes"),
        (Outcome::Conflict, "conflict"),
        (Outcome::UserEdits, "user edits"),
        (Outcome::InPlaceCommitted, "committed in place"),
        (Outcome::InPlaceApplied, "applied in place"),
        (Outcome::InPlaceUserEdits, "held for your edits"),
        (Outcome::NotApplicable, "read-only none"),
        (Outcome::ForceApproved, "force approved"),
        (Outcome::Cancelled, "cancelled"),
    ] {
        let name = serde_json::to_value(outcome).unwrap();
        view.apply(&json!({"seq": 2, "kind": "task", "payload": {
            "id": "t1", "task": "u1", "status": "approved", "round": 1,
            "feedback": {"round": 1, "effect": "1 file changed — src/a.rs",
                         "integration": {"outcome": name, "detail": "kept whole"}}}}));
        assert_eq!(view.tasks[0].integration.as_ref().unwrap()["outcome"], name);
        let graph = Graph::new(&view);
        let node = graph.node("worker:t1").expect("node");
        let mut canvas = Canvas::new_window(node.x, node.y, WIDTH, node.height);
        graph.render(&view, &mut canvas, Filter::All, "worker:t1");
        let text = canvas.lines().join(" ");
        assert!(
            text.contains(label),
            "{name} must read `{label}`, got: {text}"
        );
        assert!(
            !text.contains("not reported"),
            "{name} came back unlabeled: {text}"
        );
    }
}

#[test]
fn an_unblock_announcement_invalidates_the_card_like_a_retry() {
    use antislop::lean::tui::view::View;
    let mut view = View::default();
    view.apply(&json!({"seq": 1, "kind": "plan_proposed", "payload": {
        "plan": "p1", "tasks": [{"id": "t1", "title": "Store"}]}}));
    view.apply(&json!({"seq": 2, "kind": "task", "payload": {
        "id": "t1", "task": "u1", "status": "needs_user", "round": 1,
        "feedback": {"round": 1, "effect": "2 files changed",
                     "coverage": {"total": 3, "ok": 2, "missing": 1},
                     "integration": {"outcome": "conflict", "detail": "in src/a.rs"}}}}));
    assert!(view.tasks[0].effect.is_some() && view.tasks[0].integration.is_some());
    // The unblock key's own announcement starts the work over, so the
    // blocked round's card data is no longer shown.
    view.apply(&json!({"seq": 3, "kind": "task_recovery", "payload": {
        "task": "u1", "action": "unblock", "status": "queued",
        "detail": "unblocked and queued again"}}));
    assert_eq!(view.tasks[0].status, "queued");
    assert!(view.tasks[0].effect.is_none());
    assert!(view.tasks[0].obligations.is_none());
    assert!(view.tasks[0].integration.is_none());
}

#[test]
fn sent_history_survives_restart_and_recall_restores_the_draft() {
    let state = tempfile::tempdir().unwrap();
    let first = "first message";
    let last = "abąćęł 👩‍💻\ne\u{301} second line";
    {
        let mut app = App::test_app();
        app.load_history(state.path());
        app.record_history(first);
        app.input = InputBuffer::with_text(last);
        app.submit();
        assert!(app.input.text.is_empty());
    }
    let mut app = App::test_app();
    app.load_history(state.path());
    assert_eq!(app.history, [first, last]);
    app.record_history(last); // Consecutive duplicates stay suppressed after restart.
    app.input = InputBuffer::with_text("my draft 👩‍💻");
    let draft = app.input.clone();
    app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.input.text, last);
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.input, draft);
    drop(app);
    let mut reopened = App::test_app();
    reopened.load_history(state.path());
    assert_eq!(reopened.history, [first, last]);
}

#[test]
fn separate_frontends_append_history_without_losing_each_others_messages() {
    let state = tempfile::tempdir().unwrap();
    let mut first = App::test_app();
    let mut second = App::test_app();
    first.load_history(state.path());
    second.load_history(state.path());
    first.record_history("first window");
    second.record_history("second window");
    first.record_history("first window again");
    let mut reopened = App::test_app();
    reopened.load_history(state.path());
    assert_eq!(
        reopened.history,
        ["first window", "second window", "first window again"]
    );
}

#[test]
fn project_switches_use_each_daemons_history_directory() {
    use antislop::lean::tui::workspace::ProjectConn;
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let mut app = App::test_app();
    app.projects = vec![
        ProjectConn::new(first.path().into(), Client::at(first.path())),
        ProjectConn::new(second.path().into(), Client::at(second.path())),
    ];
    app.load_history(first.path());
    app.record_history("first project");
    app.switch_project(1, Some("second-conversation".into()));
    assert!(app.history.is_empty());
    app.record_history("second project");
    app.switch_project(0, Some("first-conversation".into()));
    assert_eq!(app.history, ["first project"]);
    app.switch_project(1, Some("second-conversation".into()));
    assert_eq!(app.history, ["second project"]);
}

#[test]
fn history_storage_errors_keep_the_input_and_report_the_raw_reason() {
    let state = tempfile::tempdir().unwrap();
    let mut app = App::test_app();
    app.load_history(state.path());
    let connection = rusqlite::Connection::open(state.path().join("input-history.db")).unwrap();
    connection.execute_batch("BEGIN EXCLUSIVE").unwrap();
    app.input = InputBuffer::with_text("keep this draft");
    let draft = app.input.clone();
    app.record_history("a message");
    assert_eq!(app.history, ["a message"]);
    assert_eq!(app.input, draft);
    let notice = app.notice.as_deref().unwrap();
    assert!(notice.contains("Cannot save input history"), "{notice}");
    assert!(notice.contains("database is locked"), "{notice}");
    connection.execute_batch("ROLLBACK").unwrap();
    // A subsequent submission retries storage normally.
    app.record_history("next message");
    let mut reopened = App::test_app();
    reopened.load_history(state.path());
    assert_eq!(reopened.history, ["next message"]);
}

#[test]
fn configuration_modal_preserves_chat_and_failed_draft_and_handles_tiny_terminals() {
    use antislop::lean::tui::config::ConfigEditor;
    let mut app = App::test_app();
    app.input = InputBuffer::with_text("unsent chat 👩‍💻");
    app.config_editor = Some(
        ConfigEditor::new(
            include_str!("../examples/antislop.toml"),
            "/project/antislop.toml".into(),
        )
        .unwrap(),
    );
    let selected = app.config_editor.as_ref().unwrap().selected;
    app.paste("e\u{301}👩‍💻");
    let draft = app.config_editor.as_ref().unwrap().fields[selected]
        .input
        .text
        .clone();
    app.apply_update(Update::Config {
        project: app.client.state_dir().to_string_lossy().into_owned(),
        save: true,
        reply: Err("daemon disconnected".into()),
    });
    assert_eq!(
        app.config_editor.as_ref().unwrap().fields[selected]
            .input
            .text,
        draft
    );
    assert_eq!(
        app.config_editor.as_ref().unwrap().status,
        "daemon disconnected"
    );
    app.key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
    assert!(
        !app.config_editor.as_ref().unwrap().fields[selected]
            .input
            .text
            .ends_with('💻')
    );
    assert_eq!(app.input.text, "unsent chat 👩‍💻");
    for (width, height) in [(1, 1), (11, 4), (12, 5), (80, 24)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
    }
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!app.config_editor.as_ref().unwrap().visible);
    app.command("/config");
    assert!(app.config_editor.as_ref().unwrap().visible);
}

#[test]
fn configuration_palette_and_profile_actions_are_available() {
    use antislop::lean::tui::config::ConfigEditor;
    let picker = Picker::new(Kind::Commands);
    assert!(
        picker
            .items(&Default::default())
            .iter()
            .any(|(label, _)| label.contains("/config"))
    );
    let mut editor = ConfigEditor::new(
        include_str!("../examples/antislop.toml"),
        "antislop.toml".into(),
    )
    .unwrap();
    let count = editor.fields.len();
    editor.key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL));
    assert!(editor.fields.len() > count);
    assert!(editor.source().is_some());
    editor.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
    assert_eq!(editor.fields.len(), count);
}

#[test]
fn checks_refetch_after_verdict_without_waiting_for_a_new_task_round() {
    use antislop::lean::tui::details::CHECKS;
    let mut app = App::test_app();
    app.apply_update(Update::Events {
        conversation: "test-conv".into(), generation: 1,
        events: vec![json!({"seq":1,"kind":"plan_proposed","payload":{"plan":"p1","tasks":[{"id":"t1","title":"Task"}]}}), json!({"seq":2,"kind":"task","payload":{"id":"t1","task":"u1","status":"working","round":1}})],
        idle: false, autonomous: false,
    });
    app.open_details("worker:t1".into());
    app.details.as_mut().unwrap().tab = CHECKS;
    let request = app.task_request_id;
    app.apply_update(Update::Task {
        documents: Default::default(),
        conversation: "test-conv".into(),
        plan: Some("p1".into()),
        generation: 1,
        request_id: request,
        key: "worker:t1".into(),
        round: 1,
        task: json!({"id":"u1","round":1,"checks":[]}),
    });
    app.apply_update(Update::Events {
        conversation: "test-conv".into(),
        generation: 1,
        events: vec![
            json!({"seq":3,"kind":"check","payload":{"task":"u1","round":1,"approved":true}}),
        ],
        idle: false,
        autonomous: false,
    });
    assert_eq!(app.task_request_id, request + 1);
    assert_eq!(
        app.details.as_ref().unwrap().fetched_checks,
        app.view.check_events
    );
    app.apply_update(Update::Events {
        conversation: "test-conv".into(),
        generation: 1,
        events: vec![],
        idle: false,
        autonomous: false,
    });
    assert_eq!(app.task_request_id, request + 1);
}

#[test]
fn configuration_save_shortcut_keeps_draft_after_transport_failure() {
    use antislop::lean::tui::config::ConfigEditor;
    let dir = tempfile::tempdir().unwrap();
    let (mut app, updates) = App::test_app_with_client(Client::at(dir.path()));
    app.config_editor = Some(
        ConfigEditor::new(
            include_str!("../examples/antislop.toml"),
            "antislop.toml".into(),
        )
        .unwrap(),
    );
    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert!(app.config_editor.as_ref().unwrap().pending);
    let update = updates.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(matches!(
        &update,
        Update::Config {
            save: true,
            reply: Err(_),
            ..
        }
    ));
    app.apply_update(update);
    let editor = app.config_editor.as_mut().unwrap();
    assert!(!editor.pending);
    assert!(!editor.status.is_empty());
    assert!(editor.source().is_some());
}

#[test]
fn configuration_load_and_save_use_live_daemon_requests() {
    let dir = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(socket(dir.path())).unwrap();
    let (captured, requests) = mpsc::channel();
    let server = thread::spawn(move || {
        for save in [false, false, true] {
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            captured.send(request.clone()).unwrap();
            let source = if save {
                request["toml"].as_str().unwrap()
            } else {
                include_str!("../examples/antislop.toml")
            };
            let response = json!({"data":{"toml":source,"path":"/project/antislop.toml","status":"applied","warnings":[]},"error":null});
            writeln!(stream, "{response}").unwrap();
        }
    });
    let (mut app, updates) = App::test_app_with_client(Client::at(dir.path()));
    app.input = InputBuffer::with_text("chat draft");
    app.command("/config");
    assert_eq!(
        requests.recv_timeout(Duration::from_secs(3)).unwrap()["op"],
        "get_config"
    );
    app.apply_update(updates.recv_timeout(Duration::from_secs(3)).unwrap());
    let editor = app.config_editor.as_mut().unwrap();
    let parallel = editor
        .fields
        .iter_mut()
        .find(|f| f.path == ["policy", "max_parallel"])
        .unwrap();
    parallel.input = InputBuffer::with_text("2");
    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert_eq!(
        requests.recv_timeout(Duration::from_secs(3)).unwrap()["op"],
        "get_config"
    );
    let saved = requests.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!(saved["op"], "set_config");
    assert_eq!(
        antislop::config::Config::from_toml(saved["toml"].as_str().unwrap())
            .unwrap()
            .policy
            .max_parallel,
        2
    );
    app.apply_update(updates.recv_timeout(Duration::from_secs(3)).unwrap());
    assert_eq!(
        app.config_editor.as_ref().unwrap().status,
        "Saved and applied live."
    );
    assert_eq!(app.input.text, "chat draft");
    server.join().unwrap();
}

mod config_editor_acceptance {
    use super::*;
    use antislop::{config::Config, lean::tui::config::ConfigEditor};
    use std::{
        collections::BTreeSet,
        path::Path,
        process::{Child, Command, Stdio},
        time::Instant,
    };

    const SOURCE: &str = include_str!("../examples/antislop.toml");

    struct Daemon {
        child: Child,
        client: Client,
    }
    impl Daemon {
        fn start(project: &Path, config: &Path, state: &Path, registry: &Path) -> Self {
            antislop::lean::trust::Registry::at(registry)
                .trust(project)
                .unwrap();
            let child = Command::new(env!("CARGO_BIN_EXE_antislop"))
                .args(["__core"])
                .arg(project)
                .arg(config)
                .arg(state)
                .env("ANTISLOP_CONFIG_DIR", registry)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap();
            let daemon = Self {
                child,
                client: Client::at(state),
            };
            wait(|| daemon.client.call(&Request::Identity).is_ok());
            daemon
        }
    }
    impl Drop for Daemon {
        fn drop(&mut self) {
            let _ = self.client.call(&Request::ForceShutdown);
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
    fn wait(mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < deadline, "timed out");
            thread::sleep(Duration::from_millis(10));
        }
    }
    fn pump(app: &mut App, updates: &mpsc::Receiver<Update>) {
        app.apply_update(updates.recv_timeout(Duration::from_secs(5)).unwrap());
    }
    fn edit(editor: &mut ConfigEditor, index: usize, text: &str, cycle: bool) {
        editor.selected = index;
        if cycle {
            for _ in 0..6 {
                if editor.fields[index].input.text == text {
                    return;
                }
                editor.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
            }
            panic!(
                "could not cycle {} to {text}",
                editor.fields[index].path.join(".")
            );
        }
        editor.key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
        while !editor.fields[index].input.is_empty() {
            editor.key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        }
        for ch in text.chars() {
            editor.key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        assert_eq!(editor.fields[index].input.text, text);
    }

    /// Explicit struct literals make additions fail compilation, including fields
    /// serde would otherwise omit. Populate optional values before enumerating
    /// serialized leaves independently of the editor's field construction.
    fn serialized_config_field_paths() -> BTreeSet<Vec<String>> {
        use antislop::config::{
            CommandLine, OrchestratorConfig, Policy, Profile, ProviderConfig, RoleRouting,
            WorkspaceConfig,
        };
        let config = Config::from_toml(SOURCE).unwrap();
        let fixture = Config {
            schema_version: config.schema_version,
            workspace: WorkspaceConfig {
                mode: config.workspace.mode,
            },
            policy: Policy {
                mode: config.policy.mode,
                local_write: config.policy.local_write,
                local_commits: config.policy.local_commits,
                required_gates: config.policy.required_gates,
                max_parallel: config.policy.max_parallel,
            },
            providers: config
                .providers
                .into_iter()
                .map(|(name, provider)| {
                    (
                        name,
                        ProviderConfig {
                            binary: provider.binary,
                            args: vec!["--schema-probe".into()],
                            command: Some(CommandLine(vec!["schema-probe".into()])),
                        },
                    )
                })
                .collect(),
            profiles: config
                .profiles
                .into_iter()
                .map(|(name, profile)| {
                    (
                        name,
                        Profile {
                            provider: profile.provider,
                            model: Some("schema-probe".into()),
                            effort: profile.effort,
                            binary: Some("schema-probe".into()),
                            args: vec!["--schema-probe".into()],
                            command: Some(CommandLine(vec!["schema-probe".into()])),
                        },
                    )
                })
                .collect(),
            orchestrator: OrchestratorConfig {
                profile: config.orchestrator.profile,
            },
            routing: config
                .routing
                .into_iter()
                .map(|(kind, route)| {
                    (
                        kind,
                        RoleRouting {
                            executor: route.executor,
                            verifier: route.verifier,
                        },
                    )
                })
                .collect(),
        };
        fn leaves(value: &Value, path: Vec<String>, output: &mut BTreeSet<Vec<String>>) {
            if let Some(table) = value.as_object() {
                for (key, value) in table {
                    let mut child = path.clone();
                    child.push(key.clone());
                    leaves(value, child, output);
                }
            } else {
                output.insert(path);
            }
        }
        let mut paths = BTreeSet::new();
        leaves(&serde_json::to_value(fixture).unwrap(), vec![], &mut paths);
        // Schema version is a protocol constant, not a user-editable option.
        assert!(paths.remove(&vec!["schema_version".into()]));
        paths
    }

    #[test]
    fn every_config_field_is_edited_and_written_to_the_selected_file() {
        for global_active in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let project = dir.path().join("project");
            let registry = dir.path().join("global");
            std::fs::create_dir_all(&project).unwrap();
            std::fs::create_dir_all(&registry).unwrap();
            let local = project.join("antislop.toml");
            let global = registry.join("antislop.toml");
            std::fs::write(&local, SOURCE).unwrap();
            std::fs::write(&global, SOURCE).unwrap();
            let (target, untouched) = if global_active {
                (&global, &local)
            } else {
                (&local, &global)
            };
            let daemon = Daemon::start(&project, target, &project.join(".antislop"), &registry);
            let (mut app, updates) = App::test_app_with_client(daemon.client.clone());
            app.command("/config");
            pump(&mut app, &updates);
            let editor = app.config_editor.as_mut().unwrap();
            let expected_paths = serialized_config_field_paths();
            assert_eq!(
                editor
                    .fields
                    .iter()
                    .map(|f| f.path.clone())
                    .collect::<BTreeSet<_>>(),
                expected_paths
            );
            let mut expected_values = vec![];
            for index in 0..editor.fields.len() {
                let field = &editor.fields[index];
                let path = field.path.clone();
                let old = field.input.text.clone();
                let key = path.last().unwrap().as_str();
                let (new, cycle) = match key {
                    "mode" if path[0] == "workspace" => (json!("in-place"), true),
                    "mode" => (json!("autonomous"), true),
                    "local_write" | "local_commits" | "required_gates" => {
                        (json!(old != "true"), true)
                    }
                    "max_parallel" => (json!(2), false),
                    "provider" => (json!(if old == "codex" { "agy" } else { "codex" }), true),
                    "effort" => (json!(if old == "low" { "medium" } else { "low" }), true),
                    "binary" => (json!("/tmp/config-editor-test-binary"), false),
                    "args" => (json!(["--edited", "two words"]), false),
                    "command" => (
                        json!(["/tmp/config-editor-test-command", "--launch"]),
                        false,
                    ),
                    "model" => (json!("edited-model"), false),
                    _ => (
                        json!(if old == "reasoning" {
                            "execution"
                        } else {
                            "reasoning"
                        }),
                        false,
                    ),
                };
                let text = new
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| new.to_string());
                assert_ne!(text, old, "field must actually change: {path:?}");
                edit(editor, index, &text, cycle);
                expected_values.push((path, new));
            }
            app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
            pump(&mut app, &updates);
            assert_eq!(
                app.config_editor.as_ref().unwrap().status,
                "Saved and applied live."
            );
            let written = serde_json::to_value(Config::load(target).unwrap()).unwrap();
            for (path, expected) in expected_values {
                let actual = path.iter().fold(&written, |value, key| &value[key]);
                assert_eq!(actual, &expected, "written value for {path:?}");
            }
            assert_eq!(std::fs::read_to_string(untouched).unwrap(), SOURCE);
            assert!(
                std::fs::read_to_string(target)
                    .unwrap()
                    .contains("# Copy to your project")
            );
            // Clearing optional overrides and argument lists must remove them from disk.
            let editor = app.config_editor.as_mut().unwrap();
            for index in 0..editor.fields.len() {
                let field = &editor.fields[index];
                let key = field.path.last().unwrap().as_str();
                if matches!(key, "args" | "command")
                    || field.path[0] == "profiles" && matches!(key, "binary" | "model")
                {
                    edit(editor, index, "", false);
                }
            }
            app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
            pump(&mut app, &updates);
            assert_eq!(
                app.config_editor.as_ref().unwrap().status,
                "Saved and applied live."
            );
            let cleared = Config::load(target).unwrap();
            assert!(
                cleared
                    .providers
                    .values()
                    .all(|p| p.args.is_empty() && p.command.is_none())
            );
            assert!(cleared.profiles.values().all(|p| p.args.is_empty()
                && p.command.is_none()
                && p.binary.is_none()
                && p.model.is_none()));
            // Add and remove a profile through actual SetConfig writes.
            let editor = app.config_editor.as_mut().unwrap();
            editor.selected = editor
                .fields
                .iter()
                .position(|f| f.path[0] == "profiles")
                .unwrap();
            app.key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL));
            let name = app
                .config_editor
                .as_ref()
                .unwrap()
                .fields
                .last()
                .unwrap()
                .path[1]
                .clone();
            app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
            pump(&mut app, &updates);
            assert!(Config::load(target).unwrap().profiles.contains_key(&name));
            app.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
            app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
            pump(&mut app, &updates);
            assert!(!Config::load(target).unwrap().profiles.contains_key(&name));
            assert_eq!(std::fs::read_to_string(untouched).unwrap(), SOURCE);
        }
    }

    #[test]
    fn configuration_render_respects_environment_and_shows_origin_notice_and_field_error() {
        const CHILD: &str = "ANTISLOP_CONFIG_EDITOR_RENDER_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let dir = tempfile::tempdir().unwrap();
            let output = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "config_editor_acceptance::configuration_render_respects_environment_and_shows_origin_notice_and_field_error", "--nocapture"])
                .env(CHILD, "1").env("NO_COLOR", "1").env("ANTISLOP_ASCII", "1").env("ANTISLOP_CONFIG_DIR", dir.path()).output().unwrap();
            assert!(
                output.status.success(),
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        let global = antislop::lean::trust::config_dir()
            .unwrap()
            .join("antislop.toml");
        for is_global in [false, true] {
            let path = if is_global {
                global.clone()
            } else {
                global.parent().unwrap().join("project/antislop.toml")
            };
            let mut editor =
                ConfigEditor::new(SOURCE, path.to_string_lossy().into_owned()).unwrap();
            let index = editor
                .fields
                .iter()
                .position(|f| f.path == ["policy", "max_parallel"])
                .unwrap();
            edit(&mut editor, index, "invalid-number", false);
            assert!(editor.source().is_none());
            let mut terminal = Terminal::new(TestBackend::new(160, 16)).unwrap();
            terminal.draw(|frame| editor.draw(frame)).unwrap();
            let buffer = terminal.backend().buffer();
            let text: String = buffer.content.iter().map(|c| c.symbol()).collect();
            assert!(text.contains(if is_global {
                "Configuration (global):"
            } else {
                "Configuration (project):"
            }));
            assert!(text.contains("Running turns finish with their old profile."));
            assert!(text.contains("policy.max_parallel = invalid-number ERROR:"));
            assert!(
                buffer
                    .content
                    .iter()
                    .all(|c| c.fg == Color::Reset && c.bg == Color::Reset && c.symbol().is_ascii())
            );
            assert!(
                buffer
                    .content
                    .iter()
                    .any(|c| c.modifier.contains(Modifier::REVERSED))
            );
        }
    }
    #[test]
    fn reload_and_presave_checks_preserve_drafts_and_reject_external_changes() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let path = project.join("antislop.toml");
        std::fs::write(&path, SOURCE).unwrap();
        let daemon = Daemon::start(
            &project,
            &path,
            &project.join(".antislop"),
            &dir.path().join("registry"),
        );
        let (mut app, updates) = App::test_app_with_client(daemon.client.clone());
        app.command("/config");
        pump(&mut app, &updates);
        let index = app
            .config_editor
            .as_ref()
            .unwrap()
            .fields
            .iter()
            .position(|f| f.path == ["policy", "max_parallel"])
            .unwrap();
        edit(app.config_editor.as_mut().unwrap(), index, "2", false);
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let external = SOURCE.replace("max_parallel = 4", "max_parallel = 10");
        std::fs::write(&path, &external).unwrap();
        app.command("/config");
        pump(&mut app, &updates);
        assert!(app.config_editor.as_ref().unwrap().conflict);
        assert_eq!(
            app.config_editor.as_ref().unwrap().fields[index].input.text,
            "2"
        );
        app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), external);
        app.key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        pump(&mut app, &updates);
        assert_eq!(
            app.config_editor.as_ref().unwrap().fields[index].input.text,
            "10"
        );
        assert!(!app.config_editor.as_ref().unwrap().is_dirty());
        edit(app.config_editor.as_mut().unwrap(), index, "3", false);
        let newer = external.replace("max_parallel = 10", "max_parallel = 11");
        std::fs::write(&path, &newer).unwrap();
        app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        pump(&mut app, &updates);
        assert!(
            app.config_editor
                .as_ref()
                .unwrap()
                .status
                .contains("changed externally")
        );
        assert_eq!(
            app.config_editor.as_ref().unwrap().fields[index].input.text,
            "3"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), newer);
        // A failed reload must retain the draft too.
        std::fs::write(&path, "broken = [").unwrap();
        app.key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        pump(&mut app, &updates);
        assert_eq!(
            app.config_editor.as_ref().unwrap().fields[index].input.text,
            "3"
        );
        assert!(!app.config_editor.as_ref().unwrap().pending);
    }

    #[test]
    fn a_clean_reopened_editor_refetches_and_malformed_replies_are_explicit() {
        let mut app = App::test_app();
        for reply in [
            json!({}),
            json!({"toml": SOURCE}),
            json!({"path":"file"}),
            json!({"toml":SOURCE,"path":""}),
        ] {
            app.apply_update(Update::Config {
                project: app.client.state_dir().to_string_lossy().into_owned(),
                save: false,
                reply: Ok(reply),
            });
            assert!(
                app.notice
                    .as_ref()
                    .unwrap()
                    .contains("Invalid configuration reply")
            );
            assert!(app.config_editor.is_none());
        }
        let mut editor = ConfigEditor::new(SOURCE, "antislop.toml".into()).unwrap();
        editor.visible = false;
        editor.refresh(
            ConfigEditor::new(
                &SOURCE.replace("max_parallel = 4", "max_parallel = 9"),
                "antislop.toml".into(),
            )
            .unwrap(),
        );
        assert_eq!(
            editor
                .fields
                .iter()
                .find(|f| f.path == ["policy", "max_parallel"])
                .unwrap()
                .input
                .text,
            "9"
        );
        editor.fields.retain(|f| f.path[0] != "providers");
        editor.key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL));
        assert!(editor.status.starts_with("Add providers: agy"));
    }

    #[test]
    fn saved_profile_affects_the_next_turn_and_keeps_the_running_turn() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let binary = dir.path().join("fake-codex");
        let log = dir.path().join("calls.log");
        let script = format!(
            r#"#!/bin/sh
printf '%s\n---\n' "$*" >> '{}'
echo '{{"type":"thread.started","thread_id":"cfg-test"}}'
sleep 2
echo '{{"type":"item.completed","item":{{"type":"agent_message","text":"done"}}}}'
echo '{{"type":"turn.completed","usage":{{"input_tokens":1,"output_tokens":1}}}}'
"#,
            log.display()
        );
        std::fs::write(&binary, script).unwrap();
        std::fs::set_permissions(&binary, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        let path = project.join("antislop.toml");
        let mut initial = ConfigEditor::new(SOURCE, path.to_string_lossy().into_owned()).unwrap();
        for (field, text) in [
            (
                vec!["providers", "codex", "binary"],
                binary.to_str().unwrap(),
            ),
            (vec!["profiles", "orchestrator", "model"], "before-save"),
        ] {
            let index = initial.fields.iter().position(|f| f.path == field).unwrap();
            edit(&mut initial, index, text, false);
        }
        std::fs::write(&path, initial.source().unwrap()).unwrap();
        let daemon = Daemon::start(
            &project,
            &path,
            &project.join(".antislop"),
            &dir.path().join("registry"),
        );
        let conversation = daemon.client.call(&Request::Start).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        daemon
            .client
            .call(&Request::Send {
                conversation: conversation.clone(),
                text: "first turn".into(),
            })
            .unwrap();
        wait(|| std::fs::read_to_string(&log).is_ok_and(|s| s.contains("before-save")));
        assert_eq!(
            daemon.client.call(&Request::Identity).unwrap()["busy"],
            true
        );
        let first = std::fs::read_to_string(&log).unwrap();
        let (mut app, updates) = App::test_app_with_client(daemon.client.clone());
        app.command("/config");
        pump(&mut app, &updates);
        let editor = app.config_editor.as_mut().unwrap();
        let index = editor
            .fields
            .iter()
            .position(|f| f.path == ["profiles", "orchestrator", "model"])
            .unwrap();
        edit(editor, index, "after-save", false);
        let index = editor
            .fields
            .iter()
            .position(|f| f.path == ["profiles", "orchestrator", "effort"])
            .unwrap();
        edit(editor, index, "low", true);
        app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        pump(&mut app, &updates);
        assert_eq!(
            app.config_editor.as_ref().unwrap().status,
            "Saved and applied live."
        );
        assert_eq!(
            Config::load(&path).unwrap().profiles["orchestrator"]
                .model
                .as_deref(),
            Some("after-save")
        );
        wait(|| daemon.client.call(&Request::Identity).unwrap()["busy"] == false);
        assert_eq!(
            std::fs::read_to_string(&log).unwrap(),
            first,
            "saving restarted the active CLI"
        );
        daemon
            .client
            .call(&Request::Send {
                conversation,
                text: "second turn".into(),
            })
            .unwrap();
        wait(|| {
            std::fs::read_to_string(&log)
                .unwrap()
                .contains("after-save")
        });
        wait(|| daemon.client.call(&Request::Identity).unwrap()["busy"] == false);
        let calls = std::fs::read_to_string(&log).unwrap();
        assert!(calls.starts_with(&first));
        let turns: Vec<_> = calls.split("\n---\n").filter(|s| !s.is_empty()).collect();
        assert_eq!(turns.len(), 2);
        assert!(turns[1].contains("after-save"));
        assert!(turns[1].contains("model_reasoning_effort=\"low\""));
    }
    #[test]
    fn semantic_failures_match_the_authoritative_config_validator() {
        for (path, input, replacement) in [
            (
                vec!["policy", "max_parallel"],
                "0",
                ("max_parallel = 4", "max_parallel = 0"),
            ),
            (
                vec!["orchestrator", "profile"],
                "missing",
                ("profile = \"orchestrator\"", "profile = \"missing\""),
            ),
            (
                vec!["providers", "codex", "command"],
                "[]",
                ("binary = \"codex\"", "binary = \"codex\"\ncommand = []"),
            ),
        ] {
            let mut editor = ConfigEditor::new(SOURCE, "antislop.toml".into()).unwrap();
            let index = editor.fields.iter().position(|f| f.path == path).unwrap();
            edit(&mut editor, index, input, false);
            let expected = Config::from_toml(&SOURCE.replace(replacement.0, replacement.1))
                .unwrap_err()
                .to_string();
            assert!(editor.source().is_none());
            assert_eq!(
                editor.fields[index].error.as_deref(),
                Some(expected.as_str())
            );
            assert_eq!(editor.fields[index].input.text, input);
        }
    }
}

mod text_selection_acceptance {
    use super::*;
    use antislop::lean::tui::{
        PlanView,
        clipboard::clipboard_sequence,
        selection::TextSelection,
        view::{Entry, TaskRow},
    };
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

    #[test]
    fn text_selection_copies_rendered_graphemes_and_clears_on_content_change() {
        let area = Rect::new(2, 3, 14, 2);
        let mut selection = TextSelection::default();
        let render = || {
            let mut buffer = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 20, 8));
            buffer.set_string(2, 3, "á 👩‍👩‍👧‍👦 test", Style::default());
            buffer
        };
        let mut buffer = render();
        selection.draw_scrolled(&mut buffer, Some(area), "details:0".into(), 0);
        let mouse = |kind, column| MouseEvent {
            kind,
            column,
            row: 3,
            modifiers: KeyModifiers::NONE,
        };
        selection.mouse(mouse(MouseEventKind::Down(MouseButton::Left), 2));
        selection.mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 5));
        selection.mouse(mouse(MouseEventKind::Up(MouseButton::Left), 5));
        assert_eq!(selection.text().as_deref(), Some("á 👩‍👩‍👧‍👦"));
        let mut buffer = render();
        selection.draw_scrolled(&mut buffer, Some(area), "details:0".into(), 0);
        assert!(buffer[(5, 3)].modifier.contains(Modifier::REVERSED));
        selection.draw_scrolled(&mut render(), Some(area), "details:1".into(), 0);
        assert_eq!(selection.text(), None);
        assert_eq!(
            clipboard_sequence("\x1b[31mfoo\x1b[0m"),
            "\x1b]52;c;Zm9v\x07"
        );
    }

    #[test]
    fn text_selection_integrates_chat_agent_details_plan_and_modal_isolation() {
        for mode in 0..3 {
            let mut app = App::test_app();
            app.input = InputBuffer::with_text("retained draft");
            app.view.idle = true;
            match mode {
                0 => app.view.transcript.push(Entry {
                    style: Style::default(),
                    text: "sample café 👩‍👩‍👧‍👦".into(),
                }),
                1 => {
                    app.view.tasks.push(TaskRow {
                        id: "t1".into(),
                        title: "Selection in the session card".into(),
                        ..Default::default()
                    });
                    app.view.agents.insert(
                        "worker:t1".into(),
                        antislop::lean::tui::view::Agent {
                            feed: vec![antislop::lean::tui::session::Block::Text(
                                "sample café 👩‍👩‍👧‍👦".into(),
                            )],
                            ..Default::default()
                        },
                    );
                    app.details = Some(Details::new("worker:t1".into()));
                }
                _ => {
                    app.view.plan_doc = "sample café 👩‍👩‍👧‍👦".into();
                    app.plan_view = Some(PlanView::new("p1".into()));
                }
            }
            let mut terminal = Terminal::new(TestBackend::new(60, 24)).unwrap();
            terminal.draw(|frame| app.draw(frame)).unwrap();
            let buffer = terminal.backend().buffer();
            let mut hit = None;
            for y in 0..24 {
                for x in 0..54 {
                    let text: String = (x..x + 6).map(|col| buffer[(col, y)].symbol()).collect();
                    if text == "sample" {
                        hit = Some((x, y));
                    }
                }
            }
            let (x, y) = hit.unwrap_or_else(|| {
                panic!("selection text must be visible in mode {mode}: {buffer:?}")
            });
            let mouse = |kind| MouseEvent {
                kind,
                column: x + 2,
                row: y,
                modifiers: KeyModifiers::NONE,
            };
            app.mouse(mouse(MouseEventKind::Down(MouseButton::Left)));
            app.mouse(mouse(MouseEventKind::Up(MouseButton::Left)));
            app.mouse(mouse(MouseEventKind::Down(MouseButton::Left)));
            app.mouse(mouse(MouseEventKind::Up(MouseButton::Left)));
            assert_eq!(
                app.text_selection.text().as_deref(),
                Some("sample"),
                "mode {mode}"
            );
            terminal
                .draw(|frame| {
                    app.draw(frame);
                    without_color(frame.buffer_mut());
                    in_ascii(frame.buffer_mut());
                })
                .unwrap();
            assert!(
                terminal.backend().buffer()[(x + 2, y)]
                    .modifier
                    .contains(Modifier::REVERSED)
            );
            assert_eq!(app.text_selection.text().as_deref(), Some("sample"));
            assert_eq!(app.input.text, "retained draft");
            if mode == 1 {
                app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
                assert!(
                    app.text_selection.text().is_none(),
                    "tab changes invalidate immediately"
                );
            } else {
                app.apply_update(Update::Events {
                conversation: app.conversation.clone(),
                generation: app.generation,
                events: vec![json!({"seq": 1, "kind": "conversation_state", "payload": {"state": "waiting"}})],
                idle: true,
                autonomous: false,
            });
                assert!(
                    app.text_selection.text().is_none(),
                    "content updates invalidate immediately"
                );
            }
            app.help = Some(Help::Chat);
            terminal.draw(|frame| app.draw(frame)).unwrap();
            app.mouse(mouse(MouseEventKind::Down(MouseButton::Left)));
            app.mouse(mouse(MouseEventKind::Drag(MouseButton::Left)));
            assert!(app.text_selection.text().is_none());
            assert_eq!(app.input.text, "retained draft");
        }
    }

    #[test]
    fn text_selection_survives_scrolling_without_copying_unseen_rows() {
        let area = Rect::new(0, 0, 8, 2);
        let mut selection = TextSelection::default();
        let render = |first: &str, second: &str| {
            let mut buffer = Buffer::empty(area);
            buffer.set_string(0, 0, first, Style::default());
            buffer.set_string(0, 1, second, Style::default());
            buffer
        };
        selection.draw_scrolled(&mut render("one", "café"), Some(area), "chat".into(), 10);
        let mouse = |kind, column| MouseEvent {
            kind,
            column,
            row: 1,
            modifiers: KeyModifiers::NONE,
        };
        selection.mouse(mouse(MouseEventKind::Down(MouseButton::Left), 0));
        selection.mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 3));
        selection.mouse(mouse(MouseEventKind::Up(MouseButton::Left), 3));
        let mut scrolled = render("café", "three");
        selection.draw_scrolled(&mut scrolled, Some(area), "chat".into(), 11);
        assert_eq!(selection.text().as_deref(), Some("café"));
        assert!(scrolled[(3, 0)].modifier.contains(Modifier::REVERSED));
        assert!(!scrolled[(0, 1)].modifier.contains(Modifier::REVERSED));
        selection.draw_scrolled(&mut render("three", "four"), Some(area), "chat".into(), 12);
        assert_eq!(selection.text().as_deref(), Some("café"));
        selection.draw_scrolled(
            &mut render("three", "four"),
            Some(area),
            "new content".into(),
            12,
        );
        assert!(selection.text().is_none());
    }
}

#[test]
fn checks_retry_failed_fetch_on_next_empty_event_batch() {
    use antislop::lean::tui::details::CHECKS;
    let dir = tempfile::tempdir().unwrap();
    let (mut app, updates) = App::test_app_with_client(Client::at(dir.path()));
    app.apply_update(Update::Events {
        conversation: "test-conv".into(), generation: 1,
        events: vec![json!({"seq":1,"kind":"plan_proposed","payload":{"plan":"p1","tasks":[{"id":"t1","title":"Task"}]}}), json!({"seq":2,"kind":"task","payload":{"id":"t1","task":"u1","status":"working","round":1}})],
        idle: false, autonomous: false,
    });
    app.open_details("worker:t1".into());
    app.details.as_mut().unwrap().tab = CHECKS;
    let issued = app.task_request_id;
    let failure = updates.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(matches!(&failure, Update::TaskFailed { .. }));
    app.apply_update(failure);
    assert!(
        app.notice
            .as_ref()
            .is_some_and(|notice| notice.contains('✗'))
    );
    let listener = UnixListener::bind(socket(dir.path())).unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut line = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut line)
            .unwrap();
        writeln!(stream, "{}", json!({"ok":true,"data":{"id":"u1","round":1,"checks":[{"id":"V01","status":"passed"}]}})).unwrap();
    });
    app.apply_update(Update::Events {
        conversation: "test-conv".into(),
        generation: 1,
        events: vec![],
        idle: false,
        autonomous: false,
    });
    assert_eq!(app.task_request_id, issued + 1);
    let reply = updates.recv_timeout(Duration::from_secs(3)).unwrap();
    app.apply_update(reply);
    server.join().unwrap();
    assert_eq!(
        app.details.as_ref().unwrap().task.as_ref().unwrap()["checks"][0]["status"],
        "passed"
    );
}

#[test]
fn checks_refetch_on_task_verdict_in_the_same_round() {
    use antislop::lean::tui::details::CHECKS;
    let mut app = App::test_app();
    app.apply_update(Update::Events {
        conversation: "test-conv".into(), generation: 1,
        events: vec![json!({"seq":1,"kind":"plan_proposed","payload":{"plan":"p1","tasks":[{"id":"t1","title":"Task"}]}}), json!({"seq":2,"kind":"task","payload":{"id":"t1","task":"u1","status":"verifying","round":1}})],
        idle: false, autonomous: false,
    });
    app.open_details("worker:t1".into());
    app.details.as_mut().unwrap().tab = CHECKS;
    let issued = app.task_request_id;
    app.apply_update(Update::Task {
        documents: Default::default(),
        conversation: "test-conv".into(),
        plan: Some("p1".into()),
        generation: 1,
        request_id: issued,
        key: "worker:t1".into(),
        round: 1,
        task: json!({"id":"u1","round":1,"checks":[]}),
    });
    app.apply_update(Update::Events {
        conversation: "test-conv".into(), generation: 1,
        events: vec![json!({"seq":3,"kind":"task","payload":{"id":"t1","task":"u1","status":"approved","round":1,"feedback":{"summary":"accepted"}}})],
        idle: false, autonomous: false,
    });
    assert_eq!(app.task_request_id, issued + 1);
}

#[test]
fn checks_refresh_uses_existing_snapshot_and_updates_report_and_verdict() {
    use antislop::lean::tui::details::CHECKS;
    let dir = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(socket(dir.path())).unwrap();
    let (mut app, updates) = App::test_app_with_client(Client::at(dir.path()));
    let server = thread::spawn(move || {
        for index in 0..3 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let request: serde_json::Value = serde_json::from_str(&line).unwrap();
            if index == 1 {
                assert_eq!(request["op"], "task_checks", "{request}");
                assert_eq!(request["conversation"], "test-conv");
                assert_eq!(request["plan"], "p1");
            }
            if index == 2 {
                assert_eq!(
                    request["op"], "task_checks",
                    "verdict must refresh its full record"
                );
            }
            let data = if index == 2 {
                json!({"id":"u1","round":1,"report":"updated report","verdict":{"coverage":{"V01":{"status":"ok","note":"accepted"}}},"checks":[]})
            } else if index == 0 {
                json!({"id":"u1","round":1,"report":"keep this report","checks":[]})
            } else {
                json!({"id":"u1","round":1,"report":"check refreshed report","checks":[{"id":"V01","status":"passed"}]})
            };
            writeln!(stream, "{}", json!({"data":data,"error":null})).unwrap();
        }
    });
    app.apply_update(Update::Events {
        conversation: "test-conv".into(), generation: 1,
        events: vec![json!({"seq":1,"kind":"plan_proposed","payload":{"plan":"p1","tasks":[{"id":"t1","title":"Task"}]}}), json!({"seq":2,"kind":"task","payload":{"id":"t1","task":"u1","status":"working","round":1}})],
        idle: false, autonomous: false,
    });
    app.open_details("worker:t1".into());
    app.apply_update(updates.recv_timeout(Duration::from_secs(3)).unwrap());
    app.details.as_mut().unwrap().tab = CHECKS;
    app.apply_update(Update::Events {
        conversation: "test-conv".into(),
        generation: 1,
        events: vec![json!({"seq":3,"kind":"check","payload":{"task":"u1","round":1}})],
        idle: false,
        autonomous: false,
    });
    app.apply_update(updates.recv_timeout(Duration::from_secs(3)).unwrap());
    let task = app.details.as_ref().unwrap().task.as_ref().unwrap();
    assert_eq!(task["report"], "check refreshed report");
    assert_eq!(task["checks"][0]["status"], "passed");
    app.apply_update(Update::Events {
        conversation: "test-conv".into(), generation: 1,
        events: vec![json!({"seq":4,"kind":"task","payload":{"id":"t1","task":"u1","status":"approved","round":1}})],
        idle: false, autonomous: false,
    });
    app.apply_update(updates.recv_timeout(Duration::from_secs(3)).unwrap());
    server.join().unwrap();
    let task = app.details.as_ref().unwrap().task.as_ref().unwrap();
    assert_eq!(task["verdict"]["coverage"]["V01"]["note"], "accepted");
}

#[test]
fn task_replies_match_saved_conversation_plan_round_and_request_generation() {
    let mut app = App::test_app();
    app.view.plan = Some("plan-a".into());
    app.view.tasks.push(antislop::lean::tui::view::TaskRow {
        id: "t1".into(),
        round: 3,
        ..Default::default()
    });
    app.open_details("worker:t1".into());
    let request = app.task_request_id;
    let generation = app.generation;
    let conversation = app.conversation.clone();
    let reply = |plan: &str, round, generation| Update::Task {
        documents: Default::default(),
        conversation: conversation.clone(),
        plan: Some(plan.into()),
        generation,
        request_id: request,
        key: "worker:t1".into(),
        round,
        task: json!({"round":round,"report":"saved response"}),
    };
    let mut wrong_payload_round = reply("plan-a", 3, generation);
    if let Update::Task { task, .. } = &mut wrong_payload_round {
        task["round"] = json!(4);
    }
    app.apply_update(wrong_payload_round);
    assert!(app.details.as_ref().unwrap().task.is_none());
    app.apply_update(reply("plan-a", 3, generation + 100));
    assert!(app.details.as_ref().unwrap().task.is_none());
    app.open("other-conversation".into());
    app.view.plan = Some("plan-b".into());
    app.view.tasks.push(antislop::lean::tui::view::TaskRow {
        id: "t1".into(),
        round: 1,
        ..Default::default()
    });
    app.apply_update(reply("plan-b", 3, generation));
    app.apply_update(reply("plan-a", 2, generation));
    assert!(
        app.conversation_states[&conversation]
            .details
            .as_ref()
            .unwrap()
            .task
            .is_none()
    );
    // The inactive conversation uses its own round, not the active one's.
    app.apply_update(reply("plan-a", 3, generation));
    assert_eq!(
        app.conversation_states[&conversation]
            .details
            .as_ref()
            .unwrap()
            .task
            .as_ref()
            .unwrap()["round"],
        3
    );
    app.open(conversation.clone());
    // A→B→A changes the poller's generation; the saved request remains valid.
    app.apply_update(reply("plan-a", 3, generation));
    assert_eq!(
        app.details.as_ref().unwrap().task.as_ref().unwrap()["report"],
        "saved response"
    );
}

#[test]
fn checks_three_rounds_ignore_reverse_ordered_old_replies() {
    use antislop::lean::tui::details::CHECKS;
    let mut app = App::test_app();
    app.view.plan = Some("p1".into());
    app.view.tasks.push(antislop::lean::tui::view::TaskRow {
        id: "t1".into(),
        round: 1,
        ..Default::default()
    });
    app.open_details("worker:t1".into());
    app.details.as_mut().unwrap().tab = CHECKS;
    let mut issued = Vec::new();
    for round in 1..=3 {
        if round > 1 {
            app.apply_update(Update::Events {
                conversation:"test-conv".into(), generation:1,
                events:vec![json!({"seq":round,"kind":"task","payload":{"id":"t1","task":"u1","status":"verifying","round":round}})],
                idle:false, autonomous:false,
            });
        }
        issued.push((round, app.task_request_id));
        for &(reply_round, request_id) in issued.iter().rev() {
            app.apply_update(Update::Task {
        documents: Default::default(),
                conversation:"test-conv".into(), plan:Some("p1".into()), generation:1,
                request_id, key:"worker:t1".into(), round:reply_round,
                task:json!({"round":reply_round,"checks":[{"id":format!("round-{reply_round}") }]}),
            });
        }
        assert_eq!(
            app.details.as_ref().unwrap().task.as_ref().unwrap()["checks"][0]["id"],
            format!("round-{round}")
        );
    }
}

#[test]
fn selection_at_minimum_terminal_with_real_ascii_and_no_color_environment() {
    use antislop::lean::tui::selection::TextSelection;
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    if std::env::var_os("ANTISLOP_SELECTION_ENV_CHILD").is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "selection_at_minimum_terminal_with_real_ascii_and_no_color_environment",
            ])
            .env("ANTISLOP_SELECTION_ENV_CHILD", "1")
            .env("NO_COLOR", "1")
            .env("ANTISLOP_ASCII", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        return;
    }
    let area = Rect::new(0, 0, 12, 5);
    let render = || {
        let mut buffer = Buffer::empty(area);
        for (y, row) in antislop::lean::tui::view::wrap("abcdefghij👩‍👩‍👧‍👦café", 12)
            .iter()
            .enumerate()
        {
            buffer.set_string(0, y as u16, row, Style::default().fg(Color::Red));
        }
        buffer
    };
    let mut selection = TextSelection::default();
    for _ in 0..2 {
        let mut buffer = render();
        selection.draw_scrolled(&mut buffer, Some(area), "wrapped".into(), 0);
        let mouse = |kind, column, row| MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        };
        selection.mouse(mouse(MouseEventKind::Down(MouseButton::Left), 11, 0));
        selection.mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 3, 1));
        selection.mouse(mouse(MouseEventKind::Up(MouseButton::Left), 3, 1));
        selection.draw_scrolled(&mut buffer, Some(area), "wrapped".into(), 0);
        without_color(&mut buffer);
        in_ascii(&mut buffer);
        assert_eq!(selection.text().as_deref(), Some("👩‍👩‍👧‍👦\ncafé"));
        assert!(buffer[(10, 0)].modifier.contains(Modifier::REVERSED));
        assert_eq!(buffer[(10, 0)].fg, Color::Reset);
        let mut tiny = Buffer::empty(Rect::new(0, 0, 11, 4));
        selection.draw_scrolled(&mut tiny, None, "terminal-too-small".into(), 0);
        assert!(selection.text().is_none());
    }
}

#[test]
fn persistent_checks_fetch_failure_backs_off_and_does_not_repeat_notice() {
    use antislop::lean::tui::details::CHECKS;
    let dir = tempfile::tempdir().unwrap();
    let (mut app, updates) = App::test_app_with_client(Client::at(dir.path()));
    app.view.plan = Some("p1".into());
    app.view.tasks.push(antislop::lean::tui::view::TaskRow {
        id: "t1".into(),
        round: 1,
        ..Default::default()
    });
    app.open_details("worker:t1".into());
    app.details.as_mut().unwrap().tab = CHECKS;
    let batch = || Update::Events {
        conversation: "test-conv".into(),
        generation: 1,
        events: vec![],
        idle: false,
        autonomous: false,
    };
    app.apply_update(updates.recv_timeout(Duration::from_secs(3)).unwrap());
    assert!(app.notice.take().is_some());
    app.apply_update(batch());
    app.apply_update(updates.recv_timeout(Duration::from_secs(3)).unwrap());
    let issued = app.task_request_id;
    assert!(
        app.notice.is_none(),
        "the same failure must not re-raise a dismissed notice"
    );
    for _ in 0..100 {
        app.apply_update(batch());
    }
    assert_eq!(app.task_request_id, issued);
    assert!(updates.try_recv().is_err());
    app.details.as_mut().unwrap().fetch_retry_after = Some(std::time::Instant::now());
    app.apply_update(batch());
    assert_eq!(app.task_request_id, issued + 1);
    app.apply_update(Update::Task {
        documents: Default::default(),
        conversation: "test-conv".into(),
        generation: 1,
        plan: Some("p1".into()),
        request_id: app.task_request_id,
        key: "worker:t1".into(),
        round: 1,
        task: json!({"id":"u1","round":1,"checks":[]}),
    });
    let details = app.details.as_ref().unwrap();
    assert_eq!(details.fetch_failures, 0);
    assert!(details.fetch_retry_after.is_none());
    assert!(details.fetch_error.is_none());
}

#[test]
fn session_card_keeps_summary_fixed_and_follows_until_manually_scrolled() {
    use antislop::lean::tui::session::Block;
    use antislop::lean::tui::view::{Agent, TaskRow};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    let mut app = App::test_app();
    app.view.tasks.push(TaskRow {
        id: "t1".into(),
        title: "Compact summary".into(),
        status: "working".into(),
        round: 1,
        goal: "Full goal available on demand".into(),
        ..Default::default()
    });
    app.view.agents.insert(
        "worker:t1".into(),
        Agent {
            provider: "codex".into(),
            running: true,
            feed: (0..100)
                .map(|i| Block::Text(format!("message {i:03}")))
                .collect(),
            ..Default::default()
        },
    );
    app.details = Some(Details::new("worker:t1".into()));
    app.details.as_mut().unwrap().panel_expanded[0] = true;
    let mut terminal = Terminal::new(TestBackend::new(90, 18)).unwrap();
    let screen = |terminal: &Terminal<TestBackend>| -> Vec<String> {
        terminal
            .backend()
            .buffer()
            .content
            .chunks(90)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect())
            .collect()
    };
    terminal.draw(|f| app.draw(f)).unwrap();
    let first = screen(&terminal);
    assert!(first[1].contains("Compact summary"));
    assert!(first[2].contains("Round 1"));
    assert!(first.iter().any(|line| line.contains("message 099")));
    assert!(
        first
            .iter()
            .any(|line| line.contains("Session") && line.contains("LIVE"))
    );
    assert!(app.details.as_ref().unwrap().text_area.y >= 4);
    let area = app.details.as_ref().unwrap().text_area;
    let thumb = antislop::lean::tui::theme::Theme::from_env().glyphs().thumb;
    assert_eq!(
        terminal.backend().buffer()[(area.right(), area.bottom() - 1)].symbol(),
        thumb,
        "the live scrollbar reaches the bottom"
    );

    for _ in 0..2 {
        app.key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
    }
    terminal.draw(|f| app.draw(f)).unwrap();
    let top = screen(&terminal);
    assert_eq!(
        &first[1..4],
        &top[1..4],
        "the summary never scrolls with the transcript"
    );
    assert!(top.iter().any(|line| line.contains("message 000")));
    app.view
        .agents
        .get_mut("worker:t1")
        .unwrap()
        .feed
        .push(Block::Text("newest message".into()));
    terminal.draw(|f| app.draw(f)).unwrap();
    assert_eq!(app.details.as_ref().unwrap().scroll, 0);
    assert!(
        !screen(&terminal)
            .iter()
            .any(|line| line.contains("newest message"))
    );

    app.key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT));
    terminal.draw(|f| app.draw(f)).unwrap();
    assert!(
        screen(&terminal)
            .iter()
            .any(|line| line.contains("newest message"))
    );
    app.view
        .agents
        .get_mut("worker:t1")
        .unwrap()
        .feed
        .push(Block::Text("live appended".into()));
    terminal.draw(|f| app.draw(f)).unwrap();
    assert!(
        screen(&terminal)
            .iter()
            .any(|line| line.contains("live appended"))
    );
    app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert!(!app.details.as_ref().unwrap().follow);
    app.key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE));
    terminal.draw(|f| app.draw(f)).unwrap();
    assert!(
        screen(&terminal)
            .iter()
            .any(|line| line.contains("Full goal available on demand"))
    );
    app.key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE));
    terminal.draw(|f| app.draw(f)).unwrap();
    assert!(
        screen(&terminal)
            .iter()
            .any(|line| line.contains("live appended"))
    );
}

#[test]
fn vim_details_navigation_isolated_from_search_and_other_commands() {
    let mut app = App::test_app();
    app.details = Some(Details::new("worker:t1".into()));
    app.details.as_mut().unwrap().scroll = 50;
    app.key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
    assert_eq!(app.details.as_ref().unwrap().scroll, 50);
    app.key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
    assert_eq!(
        app.details.as_ref().unwrap().scroll,
        50,
        "gg must be consecutive"
    );
    app.key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
    for c in ['g', 'g', 'G'] {
        app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    assert_eq!(app.details.as_ref().unwrap().query, "ggG");
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
    assert!(app.details.as_ref().unwrap().scroll == 0);
    app.details.as_mut().unwrap().scroll = 50;
    app.key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    app.details.as_mut().unwrap().scroll = 20;
    app.key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
    assert_eq!(
        app.details.as_ref().unwrap().scroll,
        20,
        "tab change clears the prefix"
    );
    app.key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
    assert_eq!(app.details.as_ref().unwrap().scroll, 0);
    app.key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::NONE));
    assert_eq!(app.details.as_ref().unwrap().scroll, usize::MAX);
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)); // no overflow before redraw
}

#[test]
fn configuration_provider_and_route_structural_edits_round_trip() {
    use antislop::lean::tui::config::ConfigEditor;
    for (section, name) in [("providers", "claude"), ("routing", "research")] {
        let mut config = include_str!("../examples/antislop.toml")
            .parse::<toml_edit::DocumentMut>()
            .unwrap();
        if section == "routing" {
            config["routing"]
                .as_table_like_mut()
                .unwrap()
                .remove("research");
        }
        let source = config.to_string();
        let mut editor = ConfigEditor::new(&source, "antislop.toml".into()).unwrap();
        editor.selected = editor
            .fields
            .iter()
            .position(|f| f.path[0] == section)
            .unwrap();
        editor.key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL));
        assert!(editor.status.contains(name));
        editor.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(editor.fields[editor.selected].path[1], name);
        let saved = editor.source().unwrap();
        let config = antislop::config::Config::from_toml(&saved).unwrap();
        if section == "providers" {
            let provider = &config.providers[&antislop::config::ProviderKind::Claude];
            assert_eq!(provider.binary, std::path::PathBuf::from("claude"));
            assert!(provider.args.is_empty());
            assert!(provider.command.is_none());
        } else {
            let route = &config.routing[&antislop::config::WorkKind::Research];
            let first_profile = editor
                .fields
                .iter()
                .find(|f| f.path[0] == "profiles")
                .unwrap()
                .path[1]
                .clone();
            assert_eq!(route.executor, first_profile);
            assert_eq!(route.verifier, first_profile);
        }
        let mut loaded = ConfigEditor::new(&saved, "antislop.toml".into()).unwrap();
        loaded.selected = loaded
            .fields
            .iter()
            .position(|f| f.path[0] == section && f.path[1] == name)
            .unwrap();
        loaded.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        let saved = loaded.source().unwrap();
        let loaded = ConfigEditor::new(&saved, "antislop.toml".into()).unwrap();
        assert!(
            !loaded
                .fields
                .iter()
                .any(|f| f.path[0] == section && f.path[1] == name)
        );
    }
}

#[test]
fn configuration_invalid_structural_deletion_preserves_draft_and_marks_field() {
    use antislop::lean::tui::config::ConfigEditor;
    for (section, name) in [
        ("providers", "agy"),
        ("routing", "testing"),
        ("routing", "orchestration"),
    ] {
        let mut editor = ConfigEditor::new(
            include_str!("../examples/antislop.toml"),
            "antislop.toml".into(),
        )
        .unwrap();
        editor.selected = editor
            .fields
            .iter()
            .position(|f| f.path[0] == section && f.path[1] == name)
            .unwrap();
        editor.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        let draft: Vec<_> = editor
            .fields
            .iter()
            .map(|f| (f.path.clone(), f.input.text.clone()))
            .collect();
        assert!(editor.source().is_none());
        let error = editor.fields[editor.selected].error.as_deref().unwrap();
        match (section, name) {
            ("providers", _) => assert!(
                error.contains("profile refers to an unconfigured provider"),
                "{error}"
            ),
            ("routing", "testing") => assert!(
                error.contains("policy.required_gates adds Testing tasks")
                    && error.contains("no route for that work"),
                "{error}"
            ),
            ("routing", "orchestration") => {
                assert_eq!(error, "Cannot remove routing.orchestration.")
            }
            _ => unreachable!(),
        }
        assert_eq!(
            editor.fields.iter().filter(|f| f.error.is_some()).count(),
            1
        );
        assert!(editor.is_dirty());
        assert_eq!(
            draft,
            editor
                .fields
                .iter()
                .map(|f| (f.path.clone(), f.input.text.clone()))
                .collect::<Vec<_>>()
        );
        assert!(
            !editor
                .fields
                .iter()
                .any(|f| f.path[0] == section && f.path[1] == name)
        );
    }
}

#[test]
fn configuration_sparse_routes_and_empty_providers_can_be_restored() {
    use antislop::lean::tui::config::ConfigEditor;
    let mut doc = include_str!("../examples/antislop.toml")
        .parse::<toml_edit::DocumentMut>()
        .unwrap();
    let options = doc["routing"]["options"].clone();
    doc["routing"] = toml_edit::Item::Table(toml_edit::Table::new());
    doc["routing"]["options"] = options;
    doc["policy"]["required_gates"] = toml_edit::value(false);
    let mut editor = ConfigEditor::new(&doc.to_string(), "antislop.toml".into()).unwrap();
    editor.selected = editor
        .fields
        .iter()
        .position(|f| f.path[0] == "routing")
        .unwrap();
    editor.key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL));
    assert!(editor.status.contains("orchestration"));
    editor.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let restored = antislop::config::Config::from_toml(&editor.source().unwrap()).unwrap();
    let route = &restored.routing[&antislop::config::WorkKind::Orchestration];
    assert_eq!(route.executor, restored.orchestrator.profile);
    assert!(restored.profiles.contains_key(&route.verifier));
    editor.fields.retain(|f| f.path[0] != "providers");
    editor.selected = 0;
    editor.key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL));
    assert!(editor.status.contains("providers"));
    editor.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(editor.fields.iter().any(|f| f.path[0] == "providers"));
}

#[test]
fn configuration_kind_picker_includes_every_missing_kind() {
    use antislop::config::{Config, ProviderKind};
    use antislop::lean::tui::config::ConfigEditor;
    use std::collections::BTreeSet;

    let source = include_str!("../examples/antislop.toml");
    // The example configures agy and codex. Each other supported provider must
    // be offered, including the final declared variant, with usable defaults.
    for (index, name) in ["claude", "gemini", "opencode"].into_iter().enumerate() {
        let mut editor = ConfigEditor::new(source, "antislop.toml".into()).unwrap();
        editor.selected = editor
            .fields
            .iter()
            .position(|f| f.path[0] == "providers")
            .unwrap();
        editor.key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL));
        for _ in 0..index {
            editor.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        }
        assert!(
            editor
                .status
                .starts_with(&format!("Add providers: {name}  "))
        );
        editor.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let config = Config::from_toml(&editor.source().unwrap()).unwrap();
        let kind: ProviderKind = serde_json::from_value(json!(name)).unwrap();
        let provider = &config.providers[&kind];
        assert_eq!(provider.binary, std::path::PathBuf::from(name));
        assert!(provider.args.is_empty());
        assert!(provider.command.is_none());
    }

    // Keep the mandatory options route and offer every missing route exactly once.
    let mut expected: BTreeSet<String> = serde_json::to_value(Config::from_toml(source).unwrap())
        .unwrap()["routing"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    assert!(expected.remove("options"));
    let mut doc = source.parse::<toml_edit::DocumentMut>().unwrap();
    let options = doc["routing"]["options"].clone();
    doc["routing"] = toml_edit::Item::Table(toml_edit::Table::new());
    doc["routing"]["options"] = options;
    doc["policy"]["required_gates"] = toml_edit::value(false);
    let mut editor = ConfigEditor::new(&doc.to_string(), "antislop.toml".into()).unwrap();
    editor.selected = editor
        .fields
        .iter()
        .position(|f| f.path[0] == "routing")
        .unwrap();
    editor.key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL));
    let initial_status = editor.status.clone();
    let mut offered = BTreeSet::new();
    for _ in 0..expected.len() {
        let name = editor
            .status
            .strip_prefix("Add routing: ")
            .unwrap()
            .split("  ")
            .next()
            .unwrap();
        assert!(offered.insert(name.to_owned()), "duplicate route: {name}");
        editor.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    }
    assert_eq!(offered, expected);
    assert_eq!(
        editor.status, initial_status,
        "cycle must contain exactly all kinds"
    );
    editor.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(
        !editor.is_dirty(),
        "browsing and cancelling must preserve the draft"
    );
}

#[test]
fn inspector_round_navigation_and_role_drafts_survive_reopening() {
    use antislop::lean::tui::view::TaskRow;
    let mut app = App::test_app();
    app.view.plan = Some("plan".into());
    let uuid = uuid::Uuid::new_v4().to_string();
    app.view.tasks.push(TaskRow {
        id: "t1".into(),
        uuid: Some(uuid.clone()),
        round: 2,
        ..Default::default()
    });
    app.open_details("worker:t1".into());
    {
        let details = app.details.as_mut().unwrap();
        details.available_rounds = vec![1, 2];
        details.task = Some(json!({"round":2,"id":uuid}));
    }
    app.key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
    app.paste("worker 👩‍💻e\u{301}");
    app.key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
    app.paste("verifier draft");
    let details = app.details.as_ref().unwrap();
    let worker = (app.conversation.clone(), format!("worker:{uuid}"));
    let verifier = (app.conversation.clone(), format!("verifier:{uuid}"));
    assert_eq!(
        details.drafts.borrow()[&worker].input.text,
        "worker 👩‍💻e\u{301}"
    );
    assert_eq!(
        details.drafts.borrow()[&verifier].input.text,
        "verifier draft"
    );
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    assert_eq!(app.details.as_ref().unwrap().selected_round, Some(1));
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.details.as_ref().unwrap().selected_round, Some(1));
    app.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    assert_eq!(app.details.as_ref().unwrap().selected_round, Some(1));
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.open_details("worker:t1".into());
    let details = app.details.as_ref().unwrap();
    assert_eq!(details.selected_round, None);
    assert_eq!(
        details.drafts.borrow()[&worker].input.text,
        "worker 👩‍💻e\u{301}"
    );
    assert_eq!(
        details.drafts.borrow()[&verifier].input.text,
        "verifier draft"
    );
}

#[test]
fn inspection_fetches_are_serialized_and_changes_during_a_fetch_are_not_lost() {
    use antislop::lean::tui::details::CHECKS;
    let mut app = App::test_app();
    app.apply_update(Update::Events {conversation:"test-conv".into(),generation:1,events:vec![json!({"seq":1,"kind":"plan_proposed","payload":{"plan":"p1","tasks":[{"id":"t1","title":"Task"}]}}),json!({"seq":2,"kind":"task","payload":{"id":"t1","task":"u1","status":"working","round":1}})],idle:false,autonomous:false});
    app.open_details("worker:t1".into());
    app.details.as_mut().unwrap().tab = CHECKS;
    let issued = app.task_request_id;
    for seq in 3..10 {
        app.apply_update(Update::Events {
            conversation: "test-conv".into(),
            generation: 1,
            events: vec![json!({"seq":seq,"kind":"check","payload":{"task":"u1","round":1}})],
            idle: false,
            autonomous: false,
        });
    }
    assert_eq!(
        app.task_request_id, issued,
        "one transfer remains in flight"
    );
    app.apply_update(Update::Task {
        documents: Default::default(),
        conversation: "test-conv".into(),
        plan: Some("p1".into()),
        generation: 1,
        request_id: issued,
        key: "worker:t1".into(),
        round: 1,
        task: json!({"id":"u1","round":1,"checks":[]}),
    });
    assert_eq!(
        app.task_request_id,
        issued + 1,
        "one follow-up fetch incorporates all arrivals"
    );
}

#[test]
fn inspector_delivery_failure_before_send_receipt_remains_visible_until_delivery() {
    use antislop::lean::tui::details::Composer;
    let mut app = App::test_app();
    app.open_details("orchestrator".into());
    let drafts = std::rc::Rc::clone(&app.details.as_ref().unwrap().drafts);
    let recipient = "worker:task-uuid".to_owned();
    let key = ("test-conv".to_owned(), recipient.clone());
    let operation = uuid::Uuid::new_v4().to_string();
    let text = "full draft 👩‍💻".to_owned();
    drafts.borrow_mut().insert(
        key.clone(),
        Composer {
            input: InputBuffer::with_text(text.clone()),
            sending: true,
            operation: Some(operation.clone()),
            address: Some(recipient.clone()),
            sent_text: Some(text.clone()),
            ..Default::default()
        },
    );
    app.apply_update(Update::Events {conversation:"test-conv".into(),generation:1,events:vec![json!({"seq":1,"kind":"mail_delivery_failed","payload":{"recipient":recipient,"ids":[7],"reason":"authentication required"}})],idle:true,autonomous:false});
    app.apply_update(Update::AgentMessage {
        conversation: "test-conv".into(),
        recipient: recipient.clone(),
        operation,
        text: text.clone(),
        result: Ok(json!({"mail":7,"state":"queued"})),
    });
    assert!(
        drafts.borrow()[&key]
            .error
            .as_ref()
            .unwrap()
            .contains("authentication required")
    );
    assert_eq!(drafts.borrow()[&key].sent_text.as_ref(), Some(&text));
    app.apply_update(Update::Events {
        conversation: "test-conv".into(),
        generation: 1,
        events: vec![json!({"seq":2,"kind":"mail_delivered","payload":{"ids":[7]}})],
        idle: true,
        autonomous: false,
    });
    assert!(drafts.borrow()[&key].error.is_none());
    assert_eq!(drafts.borrow()[&key].delivery.as_deref(), Some("delivered"));
}

#[test]
fn inspector_receipt_uses_delivery_state_from_its_own_conversation() {
    use antislop::lean::tui::details::Composer;
    let mut app = App::test_app();
    app.open_details("orchestrator".into());
    let drafts = std::rc::Rc::clone(&app.details.as_ref().unwrap().drafts);
    let operation = uuid::Uuid::new_v4().to_string();
    let key = ("another-conversation".to_owned(), "orchestrator".to_owned());
    drafts.borrow_mut().insert(
        key.clone(),
        Composer {
            operation: Some(operation.clone()),
            sending: true,
            ..Default::default()
        },
    );
    app.view.messages.push(antislop::lean::tui::MessageEntry {
        id: "7".into(),
        sender: "user".into(),
        target: "orchestrator".into(),
        state: "delivered".into(),
        text: "unrelated".into(),
        kind: "agent_work".into(),
    });
    app.apply_update(Update::AgentMessage {
        conversation: key.0.clone(),
        recipient: key.1.clone(),
        operation,
        text: "sent".into(),
        result: Ok(json!({"mail":7,"state":"queued"})),
    });
    assert_eq!(drafts.borrow()[&key].delivery.as_deref(), Some("queued"));
    let operation = uuid::Uuid::new_v4().to_string();
    {
        let mut drafts = drafts.borrow_mut();
        let composer = drafts.get_mut(&key).unwrap();
        composer.operation = Some(operation.clone());
        composer.sending = true;
        composer.address = Some(key.1.clone());
    }
    app.apply_update(Update::Events {conversation:key.0.clone(),generation:1,events:vec![json!({"seq":9,"kind":"mail_delivered","payload":{"recipient":"orchestrator","ids":[9,10]}})],idle:true,autonomous:false});
    app.apply_update(Update::AgentMessage {
        conversation: key.0.clone(),
        recipient: key.1.clone(),
        operation,
        text: "sent".into(),
        result: Ok(json!({"mail":9,"state":"queued"})),
    });
    assert_eq!(drafts.borrow()[&key].delivery.as_deref(), Some("delivered"));
}

#[test]
fn header_shows_repository_branch_for_old_sessions_and_has_no_empty_separators() {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        std::process::Command::new("git")
            .args(["init", "-b", "feature/repo-branch"])
            .current_dir(dir.path())
            .output()
            .unwrap()
            .status
            .success()
    );
    let mut app = App::test_app();
    app.projects
        .push(antislop::lean::tui::workspace::ProjectConn::new(
            dir.path().to_path_buf(),
            app.client.clone(),
        ));
    for mode in [None, Some(false), Some(true)] {
        app.view.autonomous = mode;
        let line = app.header_line(180);
        let text: String = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert!(text.contains("branch: feature/repo-branch"), "{text}");
        let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(!normalized.contains("· ·"), "{text}");
    }
    app.view.session_branch = Some("feature/session-start".into());
    let line = app.header_line(180);
    let text: String = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();
    assert!(text.contains("branch: feature/session-start"));
    assert!(!text.contains("feature/repo-branch"));
}

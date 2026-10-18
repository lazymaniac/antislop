use architect::lean::{
    daemon::{Client, Request, socket},
    tui::{
        App, Details, DiffView, Editor, Filter, Focus, Help, InputBuffer, Kind, Picker, Update,
        editor::{
            MAX_PASTE_BYTES, compute_visual_lines, sanitize_paste, sanitize_single_line_paste,
            visual_text_lines,
        },
        in_ascii,
        view::{
            MIN_TERM_HEIGHT, MIN_TERM_WIDTH, PanelFocus, SIDE_BY_SIDE_BREAKPOINT,
            draw_min_term_guard, main_footer_hint, render_header, transcript_empty_state,
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
};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

#[test]
fn large_multiline_paste_keeps_the_cursor_and_last_line_visible() {
    let mut app = App::test_app();
    app.paste(&"\n".repeat(MAX_PASTE_BYTES));
    app.paste("koniec");
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
    assert_eq!(terminal.backend().buffer()[(1, cursor.y)].symbol(), "k");

    app.editor = Some(Editor {
        target: "worker:t1".into(),
        input: app.input.clone(),
    });
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let cursor = terminal.get_cursor_position().unwrap();
    assert_eq!(cursor.y, 9);
    assert_eq!(terminal.backend().buffer()[(1, cursor.y)].symbol(), "k");
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
fn long_activity_labels_fit_the_progress_overlay() {
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
    assert!(rendered.contains(" teraz "));
    assert!(rendered.contains(&"a".repeat(30)));
}

#[test]
fn tui_input_buffer_unicode_character_boundary_safety() {
    let mut buf = InputBuffer::new();
    assert!(buf.is_empty());
    assert_eq!(buf.len(), 0);

    // Polish diacritics (multi-byte UTF-8: 2 bytes each)
    buf.insert_str("zażółć");
    assert_eq!(buf.text, "zażółć");
    assert_eq!(buf.cursor, "zażółć".len());

    // Backspace on multi-byte char removes exact code point
    assert!(buf.backspace());
    assert_eq!(buf.text, "zażół");
    assert_eq!(buf.cursor, "zażół".len());

    // Insert 4-byte emoji
    buf.insert_char('🦀');
    assert_eq!(buf.text, "zażół🦀");
    assert_eq!(buf.cursor, "zażół🦀".len());

    // Move left past emoji (4 bytes)
    buf.move_left();
    assert_eq!(buf.cursor, "zażół".len());

    // Delete forward deletes emoji
    assert!(buf.delete());
    assert_eq!(buf.text, "zażół");

    // Home / End
    buf.move_home();
    assert_eq!(buf.cursor, 0);
    buf.move_end();
    assert_eq!(buf.cursor, buf.len());

    // Word navigation and deletion
    buf.clear();
    buf.insert_str("pierwsze drugie trzecie");
    buf.move_word_left();
    assert_eq!(buf.cursor, "pierwsze drugie ".len());
    buf.delete_word_back();
    assert_eq!(buf.text, "pierwsze trzecie");
    assert_eq!(buf.cursor, "pierwsze ".len());

    buf.delete_to_start();
    assert_eq!(buf.text, "trzecie");
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
    let long_text = "pierwsza linia tekstu która powinna zostać zawinięta do następnego wiersza";
    let wrapped = compute_visual_lines(long_text, 20);
    assert!(wrapped.len() > 1);
    for line in &wrapped {
        assert!(line.width <= 20);
    }
}

#[test]
fn tui_paste_sanitization_and_character_boundary_capping() {
    // Normalization of CRLF and CR
    let raw = "linia 1\r\nlinia 2\rlinia 3";
    let sanitized = sanitize_paste(raw);
    assert_eq!(sanitized, "linia 1\nlinia 2\nlinia 3");

    // Single-line paste flattens newlines to space and trims
    let single = sanitize_single_line_paste("  polecenie\r\n parametr  \n");
    assert_eq!(single, "polecenie  parametr");

    // 64 KB cap truncates cleanly without splitting multi-byte UTF-8
    let unit = "zażółć gęślą jaźń ";
    let repeat_count = (MAX_PASTE_BYTES / unit.len()) + 10;
    let huge = unit.repeat(repeat_count);
    assert!(huge.len() > MAX_PASTE_BYTES);

    let capped = sanitize_paste(&huge);
    assert!(capped.len() <= MAX_PASTE_BYTES);
    assert!(capped.is_char_boundary(capped.len()));
}

#[test]
fn tui_responsive_breakpoints_and_terminal_guard() {
    assert_eq!(MIN_TERM_WIDTH, 60);
    assert_eq!(MIN_TERM_HEIGHT, 12);
    assert_eq!(SIDE_BY_SIDE_BREAKPOINT, 100);

    // Below 60x12 guard
    let backend_small = TestBackend::new(59, 11);
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
    assert!(rendered.contains("Terminal zbyt mały (wymagane min. 60x12)"));

    // At 60x12 minimum guard passes
    let backend_ok = TestBackend::new(60, 12);
    let mut term_ok = Terminal::new(backend_ok).unwrap();
    let mut triggered_ok = false;
    term_ok
        .draw(|f| {
            triggered_ok = draw_min_term_guard(f, f.area());
        })
        .unwrap();
    assert!(!triggered_ok);

    // Breakpoint at 99 (<100) -> compact banner
    let header_compact = render_header(
        "projekt",
        "conv-test",
        Some(true),
        "gotowy",
        PanelFocus::Chat,
        99,
    );
    let text_compact: String = header_compact
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect();
    assert!(text_compact.contains("[Czat]"));
    assert!(text_compact.contains("Zespół"));
    assert!(text_compact.contains("(Tab: przełącz)"));

    // Breakpoint at 100 (>=100) -> side by side mode
    let header_wide = render_header(
        "projekt",
        "conv-test",
        Some(true),
        "gotowy",
        PanelFocus::Chat,
        100,
    );
    let text_wide: String = header_wide
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect();
    assert!(!text_wide.contains("[Czat]"));
    assert!(!text_wide.contains("(Tab: przełącz)"));
    assert!(text_wide.contains("projekt"));
    assert!(text_wide.contains("autonomiczny"));

    // Polish empty state
    let empty_lines = transcript_empty_state(80);
    assert_eq!(
        empty_lines[0].spans[0].content.as_ref(),
        "Brak wiadomości w tej rozmowie."
    );

    // Prioritized footer hints
    let hint_120 = main_footer_hint(PanelFocus::Chat, false, false, None, 120);
    assert!(hint_120.contains("Alt+Enter nowa linia"));
    let hint_75 = main_footer_hint(PanelFocus::Chat, false, false, None, 75);
    assert!(hint_75.contains("Ctrl+P"));
    let hint_50 = main_footer_hint(PanelFocus::Chat, false, false, None, 50);
    assert!(!hint_50.contains("Ctrl+P"));
    assert!(hint_50.contains("Tab zespół"));
}

#[test]
fn app_interaction_sequences_editing_and_paste() {
    let mut app = App::test_app();
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    // 1. Synthetic character entry with Polish diacritics
    for ch in "zażółć gęślą jaźń".chars() {
        app.key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    assert_eq!(app.input.text, "zażółć gęślą jaźń");
    assert_eq!(app.input.cursor, "zażółć gęślą jaźń".len());

    // 2. Word jump and deletion
    app.key(KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL));
    assert_eq!(app.input.cursor, "zażółć gęślą ".len());

    app.key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
    assert_eq!(app.input.text, "zażółć jaźń");

    // 3. Clear line to start and end
    app.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    assert_eq!(app.input.text, "jaźń");
    assert_eq!(app.input.cursor, 0);

    app.key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL));
    assert_eq!(app.input.cursor, "jaźń".len());

    // 4. Multiline editing via Alt+Enter and Ctrl+J
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
    for ch in "druga linia".chars() {
        app.key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    app.key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
    for ch in "trzecia linia".chars() {
        app.key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    assert_eq!(app.input.text, "jaźń\ndruga linia\ntrzecia linia");

    // Multiline vertical cursor navigation
    app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    let (_, row) = app.input.visual_cursor(app.input_width.max(1));
    assert_eq!(row, 1, "Cursor should move up to line 1");

    // 5. Bracketed paste with newline normalization
    app.input.clear();
    app.paste("wklejka 1\r\nwklejka 2\r");
    assert_eq!(app.input.text, "wklejka 1\nwklejka 2\n");

    // 6. Huge paste cap and notice
    let big = "a".repeat(MAX_PASTE_BYTES + 100);
    app.paste(&big);
    assert_eq!(
        app.notice.as_deref(),
        Some("Wklejony tekst przycięto do 64 KB")
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
    assert!(text.contains("Wklejony tekst"));
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
        app.input = InputBuffer::with_text("szkic czatu?");
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
                input: InputBuffer::with_text("szkic komentarza?"),
            });
        }
        if help == Help::Picker {
            let mut picker = Picker::new(Kind::Commands);
            picker.query = "tryb".into();
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
        assert_eq!(app.input.text, "szkic czatu?");
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
            assert_eq!(editor.input.text, "szkic komentarza?");
        }
        if let Some(picker) = &app.picker {
            assert_eq!(picker.query, "tryb");
            assert_eq!(picker.selected, 1);
        }
    }
}

#[test]
fn help_blocks_typing_and_action_shortcuts_until_explicitly_dismissed() {
    let mut app = App::test_app();
    app.input = InputBuffer::with_text("szkic");
    for dismiss in [KeyCode::Esc, KeyCode::F(1), KeyCode::Char('?')] {
        app.key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE));
        for ch in "przypadkowo wpisany tekst".chars() {
            app.key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        for ch in ['a', 'r', 'x', 'n', 'p', 'f', 'u'] {
            app.key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL));
        }
        for code in [KeyCode::Enter, KeyCode::Tab, KeyCode::Backspace] {
            app.key(KeyEvent::new(code, KeyModifiers::NONE));
        }
        app.paste("wklejony tekst");
        assert_eq!(app.help, Some(Help::Chat));
        assert_eq!(app.input.text, "szkic");
        assert_eq!(app.focus, Focus::Chat);
        assert!(app.picker.is_none());

        app.key(KeyEvent::new(dismiss, KeyModifiers::NONE));
        assert!(app.help.is_none());
        assert_eq!(app.help_scroll, 0);
        app.key(KeyEvent::new(KeyCode::Char('!'), KeyModifiers::NONE));
        assert_eq!(app.input.text, "szkic!");
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
    picker.query = "zapisane zapytanie?".into();
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
    assert_eq!(app.picker.as_ref().unwrap().query, "zapisane zapytanie?");
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
    let pump_until = |app: &mut App, predicate: fn(&architect::lean::tui::Update) -> bool| {
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
    app.input = InputBuffer::with_text("wiadomość do orkiestratora");
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(req["op"], "send");
    assert_eq!(req["conversation"], "test-conv");
    assert_eq!(req["text"], "wiadomość do orkiestratora");
    assert!(app.input.is_empty(), "Send must clear chat draft");
    pump_until(&mut app, |u| {
        matches!(u, architect::lean::tui::Update::SendResult { .. })
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
        .insert_str("Komentarz do t1");
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(req["op"], "comment");
    assert_eq!(req["conversation"], "test-conv");
    assert_eq!(req["target"], "worker:t1");
    assert_eq!(req["text"], "Komentarz do t1");
    assert!(app.editor.is_none());
    pump_until(&mut app, |u| {
        matches!(u, architect::lean::tui::Update::CommentResult { .. })
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
        .insert_str("Komentarz w detalach t2");
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(req["op"], "comment");
    assert_eq!(req["conversation"], "test-conv");
    assert_eq!(req["target"], "worker:t2");
    assert_eq!(req["text"], "Komentarz w detalach t2");
    assert!(app.editor.is_none());
    pump_until(&mut app, |u| {
        matches!(u, architect::lean::tui::Update::CommentResult { .. })
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
            "title": "Budowa",
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
    app.key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    assert_ne!(app.details.as_ref().unwrap().tab, tab0);
    app.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
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
    assert!(text1.contains("Brak wiadomości w tej rozmowie."));

    // 2. Incoming updates: message, task, notice
    app.view.apply(&json!({
        "seq": 1,
        "kind": "message",
        "payload": {
            "author": "orchestrator",
            "text": "Witaj w projekcie testowym"
        }
    }));
    app.notice = Some("Krok 1 zakończony sukcesem".into());

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
    assert!(text2.contains("Witaj w projekcie testowym"));
    assert!(text2.contains("Krok 1 zakończony sukcesem"));
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
    assert!(text_wide.contains("autonomiczny"));
    assert!(!text_wide.contains("[Czat]"));

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
    assert!(text_compact.contains("[Czat]"));

    // 50x10 (below minimum 60x12)
    let mut term_tiny = Terminal::new(TestBackend::new(50, 10)).unwrap();
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
    assert!(text_tiny.contains("Terminal zbyt mały (wymagane min. 60x12)"));

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

    // 4. Progress overlay collision avoidance
    app.view.activity = Some("trwa kompilacja testów".into());
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
    assert!(text_overlay.contains("teraz"));
    assert!(text_overlay.contains("trwa kompilacja testów"));

    // 5. ASCII conversion
    let mut buffer = Buffer::empty(Rect::new(0, 0, 15, 1));
    buffer.set_string(0, 0, "╭─✓ łódź 漢", Style::default());
    in_ascii(&mut buffer);
    let text_ascii: String = buffer.content.iter().map(|c| c.symbol()).collect();
    assert_eq!(text_ascii, "+-+ lodz ?     ");

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
        text: "powitanie orkiestratorze".into(),
    });
    assert!(res.is_ok());
    let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(req["op"], "send");
    assert_eq!(req["conversation"], "conv-1");
    assert_eq!(req["text"], "powitanie orkiestratorze");

    // 2. Comment
    let res = client.call(&Request::Comment {
        conversation: "conv-1".into(),
        target: "worker:t1".into(),
        text: "uwaga do kodu".into(),
    });
    assert!(res.is_ok());
    let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(req["op"], "comment");
    assert_eq!(req["target"], "worker:t1");
    assert_eq!(req["text"], "uwaga do kodu");

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
    app.record_history("pierwsza wiadomość");
    app.record_history("druga wiadomość");
    app.record_history("druga wiadomość"); // duplicate ignored
    app.record_history("trzecia wiadomość");

    assert_eq!(app.history.len(), 3);
    assert_eq!(app.history[0], "pierwsza wiadomość");
    assert_eq!(app.history[1], "druga wiadomość");
    assert_eq!(app.history[2], "trzecia wiadomość");

    // Start editing a fresh draft
    app.input = InputBuffer::with_text("mój szkic w trakcie pisania");
    assert_eq!(app.history_index, None);

    // Press Up on first row -> stashes draft and recalls newest message (index 2)
    app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.history_index, Some(2));
    assert_eq!(app.input.text, "trzecia wiadomość");
    assert_eq!(app.history_draft, "mój szkic w trakcie pisania");

    // Press Up -> recalls index 1
    app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.history_index, Some(1));
    assert_eq!(app.input.text, "druga wiadomość");

    // Press Up -> recalls index 0
    app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.history_index, Some(0));
    assert_eq!(app.input.text, "pierwsza wiadomość");

    // Press Up at oldest -> stays at index 0 without overflow
    app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.history_index, Some(0));
    assert_eq!(app.input.text, "pierwsza wiadomość");

    // Press Down -> recalls index 1
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.history_index, Some(1));
    assert_eq!(app.input.text, "druga wiadomość");

    // Press Down -> recalls index 2
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.history_index, Some(2));
    assert_eq!(app.input.text, "trzecia wiadomość");

    // Press Down past newest entry -> restores stashed draft!
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.history_index, None);
    assert_eq!(app.input.text, "mój szkic w trakcie pisania");

    // Now test Esc while browsing history
    app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.history_index, Some(2));
    assert_eq!(app.input.text, "trzecia wiadomość");

    // Esc immediately cancels history recall and restores draft
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.history_index, None);
    assert_eq!(app.input.text, "mój szkic w trakcie pisania");
}

#[test]
fn tui_send_reliability_draft_retention_on_error_and_in_flight_typing() {
    let mut app = App::test_app();
    app.conversation = "conv-abc".into();

    // 1. Send chat message
    app.input = InputBuffer::with_text("oryginalna treść wiadomości");
    app.submit();

    assert!(app.sending);
    assert_eq!(
        app.pending_send.as_deref(),
        Some("oryginalna treść wiadomości")
    );
    assert!(
        app.input.is_empty(),
        "Draft is cleared so user can keep typing while in flight"
    );

    // 2. User adds new typing while waiting for the daemon response
    app.input.insert_str("nowy dopisek podczas oczekiwania");

    // 3. Daemon fails with network error
    let req_id = app.send_request_id;
    app.apply_update(Update::SendResult {
        conversation: app.conversation.clone(),
        generation: app.generation,
        request_id: req_id,
        sent_text: "oryginalna treść wiadomości".into(),
        result: Err("połączenie zerwane".into()),
    });

    assert!(!app.sending);
    assert_eq!(app.pending_send, None);
    // Crucial requirement: DO NOT automatically glue old message with new draft!
    // Active typing remains pristine:
    assert_eq!(app.input.text, "nowy dopisek podczas oczekiwania");
    // Failed message is preserved in failed_send without concatenating:
    assert_eq!(
        app.failed_send.as_deref(),
        Some("oryginalna treść wiadomości")
    );
    assert_eq!(
        app.notice.as_deref(),
        Some("✗ połączenie zerwane (niewysłana wiadomość zachowana)")
    );

    // 4. When input was empty upon error, draft is restored directly to input
    app.input.clear();
    app.failed_send = None;
    app.input = InputBuffer::with_text("inna wiadomość");
    app.submit();
    let req_id_2 = app.send_request_id;
    assert!(app.input.is_empty());
    app.apply_update(Update::SendResult {
        conversation: app.conversation.clone(),
        generation: app.generation,
        request_id: req_id_2,
        sent_text: "inna wiadomość".into(),
        result: Err("błąd sieci".into()),
    });
    assert!(!app.sending);
    assert_eq!(app.input.text, "inna wiadomość");
    assert_eq!(
        app.notice.as_deref(),
        Some("✗ błąd sieci (szkic przywrócony)")
    );
}

#[test]
fn tui_comment_send_retention_and_protection_on_error() {
    let mut app = App::test_app();
    app.conversation = "conv-xyz".into();

    // Open comment editor for task t1
    app.editor = Some(Editor {
        target: "worker:t1".into(),
        input: InputBuffer::with_text("szkic ważnego komentarza"),
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
        result: Err("baza zablokowana".into()),
    });

    assert!(!app.sending_comment);
    assert!(app.pending_comment.is_none());
    // Comment editor is restored with draft intact!
    assert!(app.editor.is_some());
    let restored = app.editor.as_ref().unwrap();
    assert_eq!(restored.target, "worker:t1");
    assert_eq!(restored.input.text, "szkic ważnego komentarza");
    assert_eq!(
        app.notice.as_deref(),
        Some("✗ baza zablokowana (szkic komentarza zachowany)")
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
        result: Err("błąd zapisu".into()),
    });
    assert!(!app.sending_comment);
    // Active editor is NOT replaced or glued
    assert_eq!(app.editor.as_ref().unwrap().target, "worker:t2");
    assert_eq!(app.editor.as_ref().unwrap().input.text, "inny edytor otwarty");
    // Failed comment was preserved in drafts
    let saved = app.get_comment_draft("worker:t1");
    assert_eq!(saved.text, "szkic ważnego komentarza");
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
    app.input = InputBuffer::with_text("pierwsza wysyłka");
    app.submit();
    assert!(app.sending);

    // User types more and presses Enter again while sending is in flight
    app.input.insert_str("druga próba");
    app.submit(); // Should be blocked!
    assert_eq!(
        app.notice.as_deref(),
        Some("Wysyłanie w toku; szkic zachowany.")
    );
    assert_eq!(app.input.text, "druga próba");

    // Also verify Ctrl+S is blocked
    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert_eq!(
        app.notice.as_deref(),
        Some("Wysyłanie w toku; szkic zachowany.")
    );

    // 2. Comment duplicate send
    app.sending = false;
    app.editor = Some(Editor {
        target: "task:t1".into(),
        input: InputBuffer::with_text("komentarz raz"),
    });
    app.submit_comment();
    assert!(app.sending_comment);

    // Reopen editor and attempt duplicate submit
    app.editor = Some(Editor {
        target: "task:t1".into(),
        input: InputBuffer::with_text("komentarz dwa"),
    });
    app.submit_comment();
    assert_eq!(
        app.notice.as_deref(),
        Some("Wysyłanie komentarza w toku; szkic zachowany.")
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
    app.input = InputBuffer::with_text("szkic w rozmowie 1");

    // Switch to conv-2
    app.open("conv-2".into());
    assert_eq!(app.conversation, "conv-2");
    assert!(app.input.is_empty(), "Fresh conversation has empty draft");

    // Type a draft in conv-2
    app.input = InputBuffer::with_text("szkic w rozmowie 2");

    // Switch back to conv-1
    app.open("conv-1".into());
    assert_eq!(app.conversation, "conv-1");
    assert_eq!(
        app.input.text, "szkic w rozmowie 1",
        "Draft in conv-1 was restored!"
    );

    // Switch back to conv-2
    app.open("conv-2".into());
    assert_eq!(app.conversation, "conv-2");
    assert_eq!(
        app.input.text, "szkic w rozmowie 2",
        "Draft in conv-2 was restored!"
    );
}

#[test]
fn tui_established_keymap_ctrl_s_and_newlines() {
    let mut app = App::test_app();

    // 1. Chat: Alt+Enter and Ctrl+J insert newlines
    app.input = InputBuffer::with_text("linia1");
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
    app.input.insert_str("linia2");
    app.key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
    app.input.insert_str("linia3");
    assert_eq!(app.input.text, "linia1\nlinia2\nlinia3");

    // 2. Chat: Ctrl+S triggers submit
    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert!(app.sending);
    assert_eq!(app.pending_send.as_deref(), Some("linia1\nlinia2\nlinia3"));
    assert!(app.input.is_empty());

    // 3. Comment: Alt+Enter and Ctrl+J insert newlines
    app.sending = false;
    app.editor = Some(Editor {
        target: "worker:t1".into(),
        input: InputBuffer::with_text("komentarz linia1"),
    });
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
    app.editor
        .as_mut()
        .unwrap()
        .input
        .insert_str("komentarz linia2");
    app.key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
    app.editor
        .as_mut()
        .unwrap()
        .input
        .insert_str("komentarz linia3");
    assert_eq!(
        app.editor.as_ref().unwrap().input.text,
        "komentarz linia1\nkomentarz linia2\nkomentarz linia3"
    );

    // 4. Comment: Ctrl+S triggers submit_comment
    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert!(app.sending_comment);
    assert_eq!(
        app.pending_comment.as_ref().unwrap().input.text,
        "komentarz linia1\nkomentarz linia2\nkomentarz linia3"
    );
}

#[test]
fn tui_delayed_responses_from_old_conversations_and_older_requests_do_not_alter_active_state() {
    let mut app = App::test_app();

    // 1. In Conv-A: send message and comment, open details and diff
    app.conversation = "conv-A".into();
    app.input = InputBuffer::with_text("wiadomość z A");
    app.submit();
    let conv_a_send_id = app.send_request_id;
    assert!(app.sending);

    app.editor = Some(Editor {
        target: "worker:t1".into(),
        input: InputBuffer::with_text("komentarz z A"),
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
    app.input = InputBuffer::with_text("szkic aktywny w B");
    app.editor = Some(Editor {
        target: "worker:t9".into(),
        input: InputBuffer::with_text("komentarz aktywny w B"),
    });
    app.details = Some(Details::new("worker:t9".into()));
    app.diff = Some(DiffView::new("t9".into()));

    // 3. Delayed responses from Conv-A arrive while in Conv-B:
    // 3a. SendResult for Conv-A
    app.apply_update(Update::SendResult {
        conversation: "conv-A".into(),
        generation: 1,
        request_id: conv_a_send_id,
        sent_text: "wiadomość z A".into(),
        result: Err("błąd wysyłki w starej rozmowie".into()),
    });
    // Active Conv-B state remains pristine:
    assert_eq!(app.input.text, "szkic aktywny w B");
    assert!(!app.sending);
    assert_eq!(app.pending_send, None);
    assert_eq!(app.failed_send, None);
    // Preserved Conv-A state was updated:
    let state_a = app.conversation_states.get("conv-A").unwrap();
    assert!(!state_a.sending);
    assert_eq!(state_a.pending_send, None);
    assert_eq!(state_a.input.text, "wiadomość z A");
    assert_eq!(state_a.failed_send, None);

    // 3b. CommentResult for Conv-A
    app.apply_update(Update::CommentResult {
        conversation: "conv-A".into(),
        plan: None,
        generation: 1,
        request_id: conv_a_comment_id,
        editor: Editor {
            target: "worker:t1".into(),
            input: InputBuffer::with_text("komentarz z A"),
        },
        result: Err("błąd komentarza w starej rozmowie".into()),
    });
    // Active Conv-B comment editor remains pristine:
    assert_eq!(app.editor.as_ref().unwrap().target, "worker:t9");
    assert_eq!(
        app.editor.as_ref().unwrap().input.text,
        "komentarz aktywny w B"
    );
    assert!(!app.sending_comment);

    // 3c. Task update for Conv-A
    app.apply_update(Update::Task {
        conversation: "conv-A".into(),
        plan: None,
        generation: 1,
        request_id: conv_a_task_id,
        key: "worker:t1".into(),
        task: json!({"id": "t1", "title": "stare zadanie"}),
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
    app.input = InputBuffer::with_text("żądanie 1 w B");
    app.submit();
    let b_req1 = app.send_request_id;
    // Request 1 succeeds
    app.apply_update(Update::SendResult {
        conversation: "conv-B".into(),
        generation: app.generation,
        request_id: b_req1,
        sent_text: "żądanie 1 w B".into(),
        result: Ok(()),
    });
    assert!(!app.sending);

    // Start request 2
    app.input = InputBuffer::with_text("żądanie 2 w B");
    app.submit();
    let b_req2 = app.send_request_id;
    assert!(b_req2 > b_req1);
    assert!(app.sending);
    assert_eq!(app.pending_send.as_deref(), Some("żądanie 2 w B"));

    // User types new draft while waiting for request 2
    app.input.insert_str("nowy dopisek w toku żądania 2");

    // Stale/delayed response for request 1 arrives late:
    app.apply_update(Update::SendResult {
        conversation: "conv-B".into(),
        generation: app.generation,
        request_id: b_req1,
        sent_text: "żądanie 1 w B".into(),
        result: Err("opóźniony błąd starego żądania".into()),
    });
    // Must NOT alter request 2 in flight or overwrite draft:
    assert!(app.sending);
    assert_eq!(app.pending_send.as_deref(), Some("żądanie 2 w B"));
    assert_eq!(app.input.text, "nowy dopisek w toku żądania 2");
}

#[test]
fn tui_identical_sends_correlation() {
    let mut app = App::test_app();
    app.conversation = "conv-idem".into();

    // 1. Send identical text first time
    app.input = InputBuffer::with_text("identyczna treść");
    app.submit();
    let req1 = app.send_request_id;
    assert!(app.sending);
    assert_eq!(app.pending_send.as_deref(), Some("identyczna treść"));

    // In-flight typing during send 1
    app.input.insert_str("wpis podczas oczekiwania 1");

    // Out-of-order older response with wrong ID arrives:
    app.apply_update(Update::SendResult {
        conversation: "conv-idem".into(),
        generation: app.generation,
        request_id: req1 - 1,
        sent_text: "identyczna treść".into(),
        result: Err("stary błąd".into()),
    });
    assert!(app.sending);
    assert_eq!(app.pending_send.as_deref(), Some("identyczna treść"));
    assert_eq!(app.input.text, "wpis podczas oczekiwania 1");

    // Send 1 fails with network error
    app.apply_update(Update::SendResult {
        conversation: "conv-idem".into(),
        generation: app.generation,
        request_id: req1,
        sent_text: "identyczna treść".into(),
        result: Err("przekroczono limit czasu".into()),
    });
    assert!(!app.sending);
    assert_eq!(app.pending_send, None);
    // Crucial: not glued, active draft kept intact, failed preserved in failed_send
    assert_eq!(app.input.text, "wpis podczas oczekiwania 1");
    assert_eq!(app.failed_send.as_deref(), Some("identyczna treść"));

    // 2. User submits identical text second time
    app.input = InputBuffer::with_text("identyczna treść");
    app.submit();
    let req2 = app.send_request_id;
    assert!(req2 > req1);
    assert!(app.sending);
    assert_eq!(app.pending_send.as_deref(), Some("identyczna treść"));

    // A delayed duplicate response for req1 arrives:
    app.apply_update(Update::SendResult {
        conversation: "conv-idem".into(),
        generation: app.generation,
        request_id: req1,
        sent_text: "identyczna treść".into(),
        result: Ok(()),
    });
    // Request 2 must still be in flight!
    assert!(app.sending);
    assert_eq!(app.pending_send.as_deref(), Some("identyczna treść"));

    // When the real response for req2 arrives:
    app.apply_update(Update::SendResult {
        conversation: "conv-idem".into(),
        generation: app.generation,
        request_id: req2,
        sent_text: "identyczna treść".into(),
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
    app.input = InputBuffer::with_text("wiersz 1\nwiersz 2 ze szkicem");
    app.input.cursor = 12;
    app.input.preferred_col = Some(15);

    app.editor = Some(Editor {
        target: "worker:t1".into(),
        input: InputBuffer::with_text("komentarz roboczy"),
    });
    app.editor.as_mut().unwrap().input.cursor = 5;
    app.editor.as_mut().unwrap().input.preferred_col = Some(5);

    app.save_comment_draft("worker:t2", InputBuffer::with_text("inny szkic t2"));

    app.sending = true;
    app.pending_send = Some("wysyłana treść A".into());
    app.send_request_id = 101;

    app.sending_comment = true;
    app.pending_comment = Some(Editor {
        target: "worker:t3".into(),
        input: InputBuffer::with_text("wysyłany komentarz t3"),
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
    app.input = InputBuffer::with_text("szkic rozmowy B");

    // 3. Switch back to Conv-A (B -> A)
    app.open("conv-a".into());
    assert_eq!(app.conversation, "conv-a");
    // Full input buffer state preserved:
    assert_eq!(app.input.text, "wiersz 1\nwiersz 2 ze szkicem");
    assert_eq!(app.input.cursor, 12);
    assert_eq!(app.input.preferred_col, Some(15));

    // Full comment editor state preserved:
    assert!(app.editor.is_some());
    let ed = app.editor.as_ref().unwrap();
    assert_eq!(ed.target, "worker:t1");
    assert_eq!(ed.input.text, "komentarz roboczy");
    assert_eq!(ed.input.cursor, 5);
    assert_eq!(ed.input.preferred_col, Some(5));

    // Comment drafts preserved:
    assert_eq!(app.get_comment_draft("worker:t2").text, "inny szkic t2");

    // In-flight sending state preserved:
    assert!(app.sending);
    assert_eq!(app.pending_send.as_deref(), Some("wysyłana treść A"));
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
        sent_text: "wysyłana treść A".into(),
        result: Err("błąd sieci po A->B->A".into()),
    });
    // Flight flag cleared - no permanent lock!
    assert!(!app.sending);
    assert_eq!(app.pending_send, None);
    // Draft retained without automatic gluing:
    assert_eq!(app.input.text, "wiersz 1\nwiersz 2 ze szkicem");
    assert_eq!(app.failed_send.as_deref(), Some("wysyłana treść A"));
    assert_eq!(
        app.notice.as_deref(),
        Some("✗ błąd sieci po A->B->A (niewysłana wiadomość zachowana)")
    );

    // 4b. CommentResult arrives for request 102 with error
    app.apply_update(Update::CommentResult {
        conversation: "conv-a".into(),
        plan: None,
        generation: 1,
        request_id: 102,
        editor: Editor {
            target: "worker:t3".into(),
            input: InputBuffer::with_text("wysyłany komentarz t3"),
        },
        result: Err("błąd bazy po A->B->A".into()),
    });
    // Comment flight flag cleared:
    assert!(!app.sending_comment);
    assert_eq!(app.pending_comment, None);
    // Active editor for worker:t1 not overwritten or glued:
    assert_eq!(app.editor.as_ref().unwrap().target, "worker:t1");
    assert_eq!(app.editor.as_ref().unwrap().input.text, "komentarz roboczy");
    // Failed comment for worker:t3 preserved in drafts and failed_comment:
    assert_eq!(
        app.get_comment_draft("worker:t3").text,
        "wysyłany komentarz t3"
    );
    assert_eq!(
        app.failed_comment.as_ref().map(|e| e.target.as_str()),
        Some("worker:t3")
    );

    // 4c. Details and Diff arrive after A -> B -> A
    app.apply_update(Update::Task {
        conversation: "conv-a".into(),
        plan: None,
        generation: 1,
        request_id: app.task_request_id,
        key: "worker:t1".into(),
        task: json!({"id": "t1", "title": "zadanie po A->B->A"}),
    });
    assert_eq!(
        app.details.as_ref().unwrap().task.as_ref().unwrap()["title"],
        "zadanie po A->B->A"
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
    assert_eq!(app.input.text, "szkic rozmowy B");
}

#[test]
fn tui_in_flight_operations_complete_and_restore_drafts_after_a_b_a_switch() {
    let mut app = App::test_app();
    app.open("conv-flight-a".into());

    // 1. Send message: input is cleared and message is in flight
    app.input = InputBuffer::with_text("pierwotna treść wiadomości");
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
        sent_text: "pierwotna treść wiadomości".into(),
        result: Err("awaria sieci".into()),
    });

    // Verification: sending flag is cleared, draft is restored to input!
    assert!(!app.sending, "sending must be cleared after A->B->A error");
    assert_eq!(app.pending_send, None);
    assert_eq!(app.input.text, "pierwotna treść wiadomości");
    assert_eq!(
        app.notice.as_deref(),
        Some("✗ awaria sieci (szkic przywrócony)")
    );

    // 4. Send comment: editor is cleared and comment is in flight
    app.editor = Some(Editor {
        target: "task:t5".into(),
        input: InputBuffer::with_text("ważna uwaga do zadania"),
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
        result: Err("odrzucono komentarz".into()),
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
    assert_eq!(ed.input.text, "ważna uwaga do zadania");
    assert_eq!(
        app.notice.as_deref(),
        Some("✗ odrzucono komentarz (szkic komentarza zachowany)")
    );

    // 7. Verify response arriving while user is in B (A -> B), then returns to A (B -> A)
    app.input = InputBuffer::with_text("wiadomość rozstrzygana w tle");
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
        sent_text: "wiadomość rozstrzygana w tle".into(),
        result: Err("timeout serwera".into()),
    });

    // Switch back to A: draft was restored into state.input and sending is false!
    app.open("conv-flight-a".into());
    assert_eq!(app.conversation, "conv-flight-a");
    assert!(!app.sending);
    assert_eq!(app.input.text, "wiadomość rozstrzygana w tle");
}

#[test]
fn tui_comments_with_same_target_in_different_conversations_and_plans() {
    let mut app = App::test_app();

    // 1. (conv1, None, "worker:t1")
    app.open("conv1".into());
    app.view.plan = None;
    app.save_comment_draft(
        "worker:t1",
        InputBuffer::with_text("szkic conv1 bez planu"),
    );

    // 2. (conv1, Some("plan-alpha"), "worker:t1")
    app.view.plan = Some("plan-alpha".into());
    app.save_comment_draft(
        "worker:t1",
        InputBuffer::with_text("szkic conv1 plan alpha"),
    );

    // 3. (conv1, Some("plan-beta"), "worker:t1")
    app.view.plan = Some("plan-beta".into());
    app.save_comment_draft(
        "worker:t1",
        InputBuffer::with_text("szkic conv1 plan beta"),
    );

    // 4. (conv2, Some("plan-alpha"), "worker:t1")
    app.open("conv2".into());
    app.view.plan = Some("plan-alpha".into());
    app.save_comment_draft(
        "worker:t1",
        InputBuffer::with_text("szkic conv2 plan alpha"),
    );

    // Verify all 4 are isolated and accessible without collision:
    assert_eq!(
        app.get_comment_draft("worker:t1").text,
        "szkic conv2 plan alpha"
    );

    // Switch back to conv1
    app.open("conv1".into());
    // Under plan-beta:
    app.view.plan = Some("plan-beta".into());
    assert_eq!(
        app.get_comment_draft("worker:t1").text,
        "szkic conv1 plan beta"
    );

    // Under plan-alpha:
    app.view.plan = Some("plan-alpha".into());
    assert_eq!(
        app.get_comment_draft("worker:t1").text,
        "szkic conv1 plan alpha"
    );

    // Under None:
    app.view.plan = None;
    assert_eq!(
        app.get_comment_draft("worker:t1").text,
        "szkic conv1 bez planu"
    );
}

#[test]
fn tui_history_recall_with_cursor_restoration() {
    let mut app = App::test_app();

    app.record_history("pierwsze polecenie");
    app.record_history("drugie polecenie");

    // Draft in progress with specific cursor position and preferred_col
    app.input = InputBuffer::with_text("szkic z kursorem w środku");
    app.input.cursor = 7;
    app.input.preferred_col = Some(7);

    // Up arrow navigates into history
    app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.history_index, Some(1));
    assert_eq!(app.input.text, "drugie polecenie");

    // Up arrow again to older history
    app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.history_index, Some(0));
    assert_eq!(app.input.text, "pierwsze polecenie");

    // Down arrow navigates back down
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.history_index, Some(1));
    assert_eq!(app.input.text, "drugie polecenie");

    // Down arrow returns to draft with exact cursor and preferred_col restored!
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.history_index, None);
    assert_eq!(app.input.text, "szkic z kursorem w środku");
    assert_eq!(app.input.cursor, 7);
    assert_eq!(app.input.preferred_col, Some(7));

    // Also test Esc cancels history browsing and restores the draft:
    app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.history_index, Some(1));
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.history_index, None);
    assert_eq!(app.input.text, "szkic z kursorem w środku");
    assert_eq!(app.input.cursor, 7);
    assert_eq!(app.input.preferred_col, Some(7));
}

#[test]
fn tui_preferred_column_memory_and_grapheme_editing() {
    // 1. Preferred column memory across lines of differing lengths
    let text = "linia pierwsza ma dwadzieścia znaków\nkrótka\ntrzecia linia jest znowu bardzo długa";
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

    // 2. Grapheme cluster editing: Polish diacritics & composite emojis
    let mut gbuf = InputBuffer::with_text("Zażółć 🚀 👨‍👩‍👧‍👦 koniec");
    // Position cursor right after the family emoji
    let family_idx = gbuf.text.find(" 👨‍👩‍👧‍👦").unwrap() + " 👨‍👩‍👧‍👦".len();
    gbuf.cursor = family_idx;

    // Backspace must remove the entire 18-byte ZWJ sequence atomically:
    assert!(gbuf.backspace());
    assert_eq!(gbuf.text, "Zażółć 🚀  koniec");

    // Position cursor right before 🚀
    let rocket_idx = gbuf.text.find('🚀').unwrap();
    gbuf.cursor = rocket_idx;
    // Delete must remove the 4-byte rocket emoji atomically:
    assert!(gbuf.delete());
    assert_eq!(gbuf.text, "Zażółć   koniec");

    // Stepping left/right across multi-byte Polish characters:
    let mut pol = InputBuffer::with_text("zażółć");
    pol.move_buffer_start();
    assert_eq!(pol.cursor, 0);
    pol.move_right(); // 'z'
    assert_eq!(pol.cursor, 1);
    pol.move_right(); // 'a'
    assert_eq!(pol.cursor, 2);
    pol.move_right(); // 'ż' (2 bytes)
    assert_eq!(pol.cursor, 4);
    pol.move_right(); // 'ó' (2 bytes)
    assert_eq!(pol.cursor, 6);
    pol.move_right(); // 'ł' (2 bytes)
    assert_eq!(pol.cursor, 8);
    pol.move_right(); // 'ć' (2 bytes)
    assert_eq!(pol.cursor, 10);
    pol.move_left(); // back over 'ć'
    assert_eq!(pol.cursor, 8);
    assert!(pol.backspace()); // backspace 'ł'
    assert_eq!(pol.text, "zażóć");
    assert_eq!(pol.cursor, 6);
}


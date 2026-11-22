use super::*;
use crate::lean::tui::{sanitize::VISITED_BYTES, theme::MATERIALIZED_ROWS};

#[test]
fn inspector_suffix_work() {
    let mut previous_work = None;
    for size in [1024, 65536, 1048576] {
        let theme = Theme::new(false, false);
        let mut view = View::default();
        view.plan_doc = "x".repeat(size);
        VISITED_BYTES.with(|n| n.set(0));
        MATERIALIZED_ROWS.with(|n| n.set(0));
        let lines = role_content(&view, crate::lean::PLANNER, None, 32, 5, &theme);
        let visited = VISITED_BYTES.with(|n| n.get());
        let rows = MATERIALIZED_ROWS.with(|n| n.get());
        eprintln!(
            "real role_content planner input_bytes={size} width=32 height=5 visited_bytes={visited} materialized_rows={rows} visible_rows={}",
            lines.len()
        );
        if let Some(work) = previous_work {
            assert_eq!((visited, rows), work);
        }
        previous_work = Some((visited, rows));
        assert_eq!(rows, 5);
        assert!(visited < 256);
        assert_eq!(lines.len(), 5);
        assert!(lines.last().unwrap().to_string().ends_with("..."));
    }
}

fn message(sender: &str, text: String) -> view::MessageEntry {
    view::MessageEntry {
        id: "message".into(),
        sent_at: None,
        sender: sender.into(),
        target: ORCHESTRATOR.into(),
        text,
        state: "delivered".into(),
        kind: "agent".into(),
    }
}

#[test]
fn inspector_streams_messages_and_selected_roles() {
    let theme = Theme::new(false, false);
    let mut previous = None;
    for size in [1024, 65536, 1048576] {
        let mut view = View::default();
        view.messages.push(message("You", "👩‍💻 é ".repeat(size / 8)));
        VISITED_BYTES.with(|n| n.set(0));
        MATERIALIZED_ROWS.with(|n| n.set(0));
        let lines = role_content(&view, ORCHESTRATOR, None, 32, 5, &theme);
        let work = (
            VISITED_BYTES.with(|n| n.get()),
            MATERIALIZED_ROWS.with(|n| n.get()),
        );
        eprintln!(
            "real role_content messages input_bytes={} width=32 height=5 visited_bytes={} materialized_rows={}",
            view.messages[0].text.len(),
            work.0,
            work.1
        );
        assert_eq!(work.1, 5);
        if let Some(previous) = previous {
            assert_eq!(work, previous);
        }
        previous = Some(work);
        assert!(lines[1].to_string().starts_with("You: 👩‍💻 é"));
        assert!(lines.last().unwrap().to_string().ends_with("..."));
        assert_eq!(lines[0].style, theme.accent());
        assert_eq!(lines[1].style, theme.text());
    }
    let mut view = View::default();
    view.messages.push(message(
        "You\x1b]",
        "hidden\x07e\x1b[31ḿ 👩\x1b[0m‍💻".into(),
    ));
    let lines = role_content(&view, ORCHESTRATOR, None, 32, 5, &theme);
    assert_eq!(lines[1].to_string(), "Youé 👩‍💻");
    for key in [
        PLAN_REVIEWER,
        AUDITOR,
        crate::lean::PLANNER,
        ORCHESTRATOR,
        "worker:t1",
        "reviewer:t1",
    ] {
        let task = TaskRow {
            id: "t1".into(),
            goal: "Goal é 👩‍💻".into(),
            summary: Some("Worker é 👩‍💻".into()),
            reviewer_summary: Some("Review é 👩‍💻".into()),
            ..Default::default()
        };
        for width in 0..8 {
            for height in 0..6 {
                let lines = role_content(&view, key, Some(&task), width, height, &theme);
                assert!(lines.len() <= height);
                if width == 0 {
                    assert!(lines.is_empty());
                }
                for line in lines {
                    // Wide clusters are kept whole, as in the eager wrapper.
                    assert!(line.width() <= width.max(2));
                    assert!(!line.to_string().contains('\x1b'));
                }
            }
        }
    }
}

#[test]
fn inspector_continuation_and_sanitized_line_boundaries() {
    let theme = Theme::new(false, false);
    for text in [
        "",
        "a",
        "a\n",
        "\n",
        "a\n\n",
        "cat on mat",
        "- list words here",
        "e\x1b[31ḿ 👩\x1b[0m‍💻",
        "x\x1b]unfinished",
        "x\u{7}y",
        "a\r\nb",
    ] {
        for width in 1..16 {
            for limit in 0..6 {
                let mut preview = Preview {
                    lines: vec![],
                    width,
                    height: limit,
                    theme: &theme,
                };
                preview.text(text, theme.error(), limit);
                let all = view::clean(text)
                    .lines()
                    .flat_map(|line| view::wrap(line, width))
                    .collect::<Vec<_>>();
                let mut expected = all.iter().take(limit).cloned().collect::<Vec<_>>();
                if all.len() > limit
                    && let Some(last) = expected.last_mut()
                {
                    let marker = &"..."[..width.min(3)];
                    *last = format!("{}{marker}", view::truncate(last, width - marker.len()));
                }
                assert_eq!(
                    preview
                        .lines
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>(),
                    expected,
                    "{text:?} width={width} limit={limit}"
                );
                assert!(preview.lines.iter().all(|line| line.style == theme.error()));
            }
        }
    }
}

#[test]
fn actual_app_draw_inspector_suffix_work() {
    use crate::lean::tui::{App, work_units::InspectorAction};
    use ratatui::{Terminal, backend::TestBackend};
    let mut previous = None;
    for size in [1024, 65536, 1048576] {
        let mut app = App::test_app();
        app.graph_inspector_visible = true;
        app.view.plan_preview = Some("plan ".repeat(size / 5));
        app.view
            .agents
            .insert(crate::lean::PLANNER.into(), view::Agent::default());
        app.select_work_stage(crate::lean::PLANNER.into());
        let mut terminal = Terminal::new(TestBackend::new(160, 40)).unwrap();
        VISITED_BYTES.with(|n| n.set(0));
        MATERIALIZED_ROWS.with(|n| n.set(0));
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let work = (
            VISITED_BYTES.with(|n| n.get()),
            MATERIALIZED_ROWS.with(|n| n.get()),
        );
        eprintln!(
            "actual App::draw planner input_bytes={} terminal=160x40 visited_bytes={} materialized_rows={}",
            app.view.plan_preview.as_ref().unwrap().len(),
            work.0,
            work.1
        );
        assert!(app.inspector_hits.iter().any(|(_, action)| matches!(action, InspectorAction::Results(key) if key == crate::lean::PLANNER)));
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Latest submitted plan"));
        assert!(text.contains("plan plan"));
        if let Some(previous) = previous {
            assert_eq!(work, previous);
        }
        previous = Some(work);
    }
}

#[test]
fn inspector_bounds_word_and_indent_lookahead_but_finishes_graphemes_and_controls() {
    let theme = Theme::new(false, false);
    for prefix in ["", "word ", "- ", "   123. ", "123456", "                 "] {
        let mut previous = None;
        for size in [1024, 65536] {
            let text = format!("{prefix}{}", "x".repeat(size));
            let mut preview = Preview {
                lines: vec![],
                width: 16,
                height: 2,
                theme: &theme,
            };
            VISITED_BYTES.with(|n| n.set(0));
            MATERIALIZED_ROWS.with(|n| n.set(0));
            preview.text(&text, theme.text(), 2);
            let work = (
                VISITED_BYTES.with(|n| n.get()),
                MATERIALIZED_ROWS.with(|n| n.get()),
            );
            assert_eq!(work.1, 2);
            assert!(work.0 < 80);
            if let Some(previous) = previous {
                assert_eq!(work, previous);
            }
            previous = Some(work);
        }
    }
    for size in [64, 4096] {
        // Finding the end of a grapheme or an escape payload requires visiting
        // that context, even with a fixed viewport. It is not a byte budget.
        for (kind, text) in [
            ("combining", format!("e{}x", "\u{301}".repeat(size))),
            ("control", format!("x\x1b]{}\x07y", "z".repeat(size))),
        ] {
            let mut preview = Preview {
                lines: vec![],
                width: 8,
                height: 1,
                theme: &theme,
            };
            VISITED_BYTES.with(|n| n.set(0));
            preview.text(&text, theme.text(), 1);
            let visited = VISITED_BYTES.with(|n| n.get());
            eprintln!(
                "necessary context kind={kind} input_bytes={} width=8 height=1 visited_bytes={visited}",
                text.len()
            );
            assert_eq!(visited, text.len());
            assert_eq!(preview.lines[0].to_string(), view::clean(&text));
        }
    }
}

#[test]
fn inspector_keeps_space_combining_and_accounts_for_plan_presence_scan() {
    let theme = Theme::new(false, false);
    let mut view = View::default();
    view.plan_doc = "a \u{301}b".into();
    let lines = role_content(&view, crate::lean::PLANNER, None, 2, 3, &theme);
    assert_eq!(
        lines
            .iter()
            .skip(1)
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["a \u{301}", "b"]
    );
    for size in [64, 4096] {
        for (kind, text, probe) in [
            ("white suffix", format!("x{}", " ".repeat(size)), 1),
            ("white prefix", format!("{}x", " ".repeat(size)), size + 1),
            ("white only", " ".repeat(size), size),
        ] {
            view.plan_doc = text;
            PLAN_PROBE_BYTES.with(|n| n.set(0));
            let _lines = role_content(&view, crate::lean::PLANNER, None, 16, 3, &theme);
            let visited = PLAN_PROBE_BYTES.with(|n| n.get());
            eprintln!(
                "plan presence kind={kind} input_bytes={} visited_bytes={visited}",
                view.plan_doc.len()
            );
            assert_eq!(visited, probe);
        }
    }
}

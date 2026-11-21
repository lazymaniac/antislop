//! Owner-level regressions through App::draw; each environment runs in a fresh process.
use super::*;
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};
use serde_json::json;

const DOCUMENT: &str = "Zażółć 👩‍💻 e\u{301} │ ╭ ·";

#[test]
fn environment_modes_are_isolated() {
    for (ascii, no_color) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "lean::tui::theme_modes_tests::actual_draw_mode_child",
                "--exact",
                "--ignored",
                "--nocapture",
            ])
            .env("LC_ALL", "en_US.UTF-8")
            .env_remove("ANTISLOP_ASCII")
            .env_remove("NO_COLOR");
        if ascii {
            command.env("ANTISLOP_ASCII", "1");
        }
        if no_color {
            command.env("NO_COLOR", "1");
        }
        let output = command.output().unwrap();
        eprintln!(
            "isolated App::draw: ANTISLOP_ASCII={ascii} NO_COLOR={no_color} status={}",
            output.status
        );
        assert!(
            output.status.success(),
            "ascii={ascii} no_color={no_color}\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn app() -> App {
    let mut app = App::test_app();
    app.view.apply(&json!({"seq":1,"kind":"plan_proposed","payload":{"plan":"p1","tasks":[
        {"id":"t1","title":"Prepare","goal":"Implement","kind":"implementation","change_kind":"code","depends_on":[]},
        {"id":"t2","title":"Verify","goal":"Check","kind":"testing","change_kind":"none","depends_on":["t1"]}
    ]}}));
    app.navigation.screen = Screen::Graph;
    app.input = InputBuffer::with_text("Zażółć 👩‍💻 e\u{301}");
    app
}

fn draw(app: &mut App, size: (u16, u16)) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(size.0, size.1)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    terminal.backend().buffer().clone()
}

fn text(buffer: &Buffer) -> String {
    let mut result = String::new();
    for row in buffer.content.chunks(usize::from(buffer.area.width).max(1)) {
        let mut skip = 0;
        for cell in row {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            result.push_str(cell.symbol());
            skip = unicode_width::UnicodeWidthStr::width(cell.symbol()).saturating_sub(1);
        }
    }
    result
}

fn application_cells(buffer: &Buffer, theme: &Theme, owner: &str) {
    // These fixtures have ASCII data except DOCUMENT, whose cell positions are
    // excluded from chrome checks by checking structural glyphs at owner sites.
    for (index, cell) in buffer.content.iter().enumerate() {
        if !theme.color {
            assert_eq!(cell.fg, Color::Reset, "{owner}: {} fg", cell.symbol());
            assert_eq!(cell.bg, Color::Reset, "{owner}: {} bg", cell.symbol());
        }
        if theme.ascii {
            assert_ne!(
                cell.symbol(),
                "·",
                "{owner}: application separator at index {index} in {:?}: {}",
                buffer.area,
                text(buffer)
            );
        }
        assert!(
            !matches!(cell.symbol(), "╭" | "╮" | "╯" | "╰"),
            "{owner}: rounded chrome"
        );
    }
}

#[test]
#[ignore = "invoked by environment_modes_are_isolated in four fresh processes"]
fn actual_draw_mode_child() {
    compact_statuses();
    let theme = Theme::from_env();
    assert_eq!(theme.color, std::env::var_os("NO_COLOR").is_none());
    assert_eq!(theme.ascii, std::env::var_os("ANTISLOP_ASCII").is_some());
    for size in [(100, 30), (40, 12), (12, 5), (11, 4)] {
        let mut graph = app();
        let buffer = draw(&mut graph, size);
        application_cells(&buffer, &theme, "graph");
        assert_eq!(graph.input.text, "Zażółć 👩‍💻 e\u{301}");
        assert_eq!(graph.view.tasks.len(), 2);
        if theme.ascii {
            assert!(
                !text(&buffer).contains(['│', '─', '┌', '┐', '└', '┘', '┼', '├', '┤', '┬', '┴']),
                "graph chrome"
            );
        }
        let mut plan = app();
        plan.view.plan_doc = "# Plan\n\nOwner document".into();
        plan.plan_view = Some(PlanView::new("p1".into()));
        let buffer = draw(&mut plan, size);
        application_cells(&buffer, &theme, "plan");
        if theme.ascii {
            assert!(
                !text(&buffer).contains(['│', '─', '┌', '┐', '└', '┘', '┼', '├', '┤', '┬', '┴']),
                "plan chrome"
            );
        }
        if size == (100, 30) {
            assert_eq!(buffer[(0, 0)].symbol(), if theme.ascii { "+" } else { "┌" });
            assert_eq!(
                buffer[(0, 0)].fg,
                theme.separator(true).fg.unwrap_or(Color::Reset)
            );
        }
        let mut config = app();
        config.config_editor = Some(
            config::ConfigEditor::new(
                include_str!("../../../examples/antislop.toml"),
                "antislop.toml".into(),
            )
            .unwrap(),
        );
        let buffer = draw(&mut config, size);
        application_cells(&buffer, &theme, "config");
        if size == (100, 30) {
            assert_eq!(
                buffer[(0, 0)].bg,
                theme.overlay().bg.unwrap_or(Color::Reset)
            );
        }
        let mut palette = app();
        palette.picker = Some(Picker::new(Kind::Commands));
        let buffer = draw(&mut palette, size);
        application_cells(&buffer, &theme, "palette");
        if theme.ascii {
            assert!(
                !text(&buffer).contains(['│', '─', '┌', '┐', '└', '┘']),
                "palette chrome"
            );
        }
        for kind in [
            Kind::Agents,
            Kind::Recipients,
            Kind::Conversations,
            Kind::Rigor,
        ] {
            let mut palette = app();
            let mut picker = Picker::new(kind);
            if kind == Kind::Conversations {
                picker.confirm_delete = true;
                picker.delete_target = Some(palette::ConversationItem {
                    id: "old".into(),
                    project: palette.project.clone(),
                    title: "Prior session".into(),
                });
            }
            if kind == Kind::Rigor {
                picker.recommendation =
                    Some((crate::lean::tasks::Team::Feature, "Recommended plan".into()));
            }
            palette.picker = Some(picker);
            application_cells(&draw(&mut palette, size), &theme, "palette variant");
        }
        let mut recovery = app();
        recovery.view.tasks[0].status = "needs_user".into();
        recovery.view.tasks[0].reason = Some("Manual overlap".into());
        recovery.navigation.screen = Screen::Workbench;
        recovery.navigation.target = WorkbenchTarget::Agent("worker:t1".into());
        recovery.details = Some(Details::new("worker:t1".into()));
        application_cells(&draw(&mut recovery, size), &theme, "recovery");
        let mut workbench = app();
        workbench.navigation.screen = Screen::Workbench;
        workbench.navigation.target = WorkbenchTarget::Agent("worker:t1".into());
        workbench.details = Some(Details::new("worker:t1".into()));
        let buffer = draw(&mut workbench, size);
        application_cells(&buffer, &theme, "workbench");
        if theme.ascii {
            assert!(
                !text(&buffer).contains(['│', '─', '┌', '┐', '└', '┘']),
                "workbench chrome"
            );
        }
        let mut approval = app();
        approval.view.approvals.push(view::PendingApproval {
            request: "a1".into(),
            role: "worker:t1".into(),
            provider: "codex".into(),
            summary: "Run check".into(),
            options: vec![("1".into(), "Allow".into(), "allow_once".into())],
            orchestrator: false,
        });
        let buffer = draw(&mut approval, size);
        application_cells(&buffer, &theme, "approval card");
        if theme.ascii {
            assert!(
                !text(&buffer).contains(['│', '─', '┌', '┐', '└', '┘']),
                "approval card chrome"
            );
        }
        approval.open_decision_inbox();
        let buffer = draw(&mut approval, size);
        application_cells(&buffer, &theme, "approval");
        if theme.ascii {
            assert!(
                !text(&buffer).contains(['│', '─', '┌', '┐', '└', '┘']),
                "approval chrome"
            );
        }
    }
    for kind in [
        Kind::Agents,
        Kind::Recipients,
        Kind::Conversations,
        Kind::Rigor,
    ] {
        let mut palette = app();
        palette.view.tasks[0].title = "User · name".into();
        let mut picker = Picker::new(kind);
        if kind == Kind::Conversations {
            picker.confirm_delete = true;
            picker.delete_target = Some(palette::ConversationItem {
                id: "old".into(),
                project: palette.project.clone(),
                title: "User · name".into(),
            });
        }
        if kind == Kind::Rigor {
            picker.recommendation = Some((crate::lean::tasks::Team::Feature, "User · name".into()));
        }
        palette.picker = Some(picker);
        assert!(
            text(&draw(&mut palette, (100, 30))).contains("User · name"),
            "palette user text"
        );
    }
    let mut recovery = app();
    recovery.view.tasks[0].title = "User · name".into();
    recovery.view.tasks[0].status = "needs_user".into();
    recovery.view.tasks[0].reason = Some(DOCUMENT.into());
    recovery.navigation.screen = Screen::Workbench;
    recovery.navigation.target = WorkbenchTarget::Agent("worker:t1".into());
    recovery.details = Some(Details::new("worker:t1".into()));
    let buffer = draw(&mut recovery, (100, 60));
    assert!(text(&buffer).contains("User · name"));
    assert!(text(&buffer).contains(DOCUMENT));
    let mut plan = app();
    plan.view.tasks.clear();
    plan.view.plan_doc = DOCUMENT.into();
    plan.plan_view = Some(PlanView::new("p1".into()));
    assert!(
        text(&draw(&mut plan, (100, 30))).contains(DOCUMENT),
        "Unicode document preserved"
    );

    let mut diff = app();
    diff.navigation.screen = Screen::Workbench;
    diff.navigation.target = WorkbenchTarget::Agent("worker:t1".into());
    diff.details = Some(Details::new("worker:t1".into()));
    diff.navigation
        .update_presentation(|p| p.tab = WorkbenchTab::Diff);
    let mut content = DiffView::new("t1".into());
    content.load(Ok(json!({"base":"abc","head":"def","files":[{"path":"code.rs","status":"modified","added":1,"removed":1,"binary":false,"lines":[
        {"kind":"del","old":1,"new":null,"text":"removed"},
        {"kind":"add","old":null,"new":1,"text":DOCUMENT}
    ]}]})));
    diff.diff = Some(content);
    for size in [(40, 12), (12, 5), (11, 4)] {
        draw(&mut diff, size);
    }
    let buffer = draw(&mut diff, (100, 30));
    assert!(text(&buffer).contains(DOCUMENT));
    // The code starts after a 14-cell gutter. Its own box drawing must remain.
    let cell = buffer
        .content
        .iter()
        .position(|cell| cell.symbol() == "Z")
        .expect("code text");
    let code = &buffer.content[cell];
    assert_eq!(code.fg, theme.ok().fg.unwrap_or(Color::Reset));
    assert_eq!(buffer.content[cell - 1].symbol(), theme.glyphs().vertical);
    assert_eq!(theme.clean(DOCUMENT), DOCUMENT);
    for color in [Color::Red, Color::Indexed(201), Color::Rgb(2, 3, 4)] {
        let styled = theme.style(
            Style::default()
                .fg(color)
                .bg(color)
                .add_modifier(Modifier::DIM | Modifier::REVERSED),
        );
        assert!(
            styled
                .add_modifier
                .contains(Modifier::DIM | Modifier::REVERSED)
        );
        if !theme.color {
            assert_eq!(styled.fg, None);
            assert_eq!(styled.bg, None);
        }
    }
    diff.diff.as_mut().unwrap().load(Err("owner error".into()));
    let buffer = draw(&mut diff, (100, 30));
    let error_cell = buffer
        .content
        .iter()
        .find(|cell| cell.symbol() == "E")
        .expect("diff error");
    assert_eq!(error_cell.fg, theme.error().fg.unwrap_or(Color::Reset));
    separator_owner_states(&theme);
    search_footer_preserves_query(&theme);
    native_cli_keeps_its_cells(&theme);
}

fn search_footer_preserves_query(theme: &Theme) {
    for tab in [WorkbenchTab::Results, WorkbenchTab::Comments] {
        let mut app = app();
        app.navigation.screen = Screen::Workbench;
        app.navigation.target = WorkbenchTarget::Agent("worker:t1".into());
        app.navigation.update_presentation(|p| p.tab = tab);
        let mut details = Details::new("worker:t1".into());
        details.tab = if tab == WorkbenchTab::Results {
            details::RESULT
        } else {
            details::COMMENTS
        };
        details.searching = true;
        details.query = DOCUMENT.into();
        app.details = Some(details);
        let buffer = draw(&mut app, (160, 40));
        let expected = format!(
            "Search records: {DOCUMENT}{}",
            theme.chrome("_ (Enter: confirm · Esc: cancel)")
        );
        let row = buffer
            .content
            .chunks(160)
            .find(|row| {
                row.iter()
                    .map(|cell| cell.symbol())
                    .collect::<String>()
                    .contains("Search records:")
            })
            .expect("search footer rendered by App::draw");
        let footer = Buffer {
            area: Rect::new(0, 0, 160, 1),
            content: row.to_vec(),
        };
        assert!(
            text(&footer).contains(&expected),
            "{tab:?}: {}",
            text(&footer)
        );
        if !theme.color {
            for cell in row {
                assert_eq!((cell.fg, cell.bg), (Color::Reset, Color::Reset));
            }
        }
        assert_eq!(app.details.as_ref().unwrap().query, DOCUMENT);
        for size in [(40, 12), (12, 5), (11, 4)] {
            draw(&mut app, size);
            assert_eq!(app.details.as_ref().unwrap().query, DOCUMENT);
        }
    }
}

fn native_cli_keeps_its_cells(theme: &Theme) {
    let mut app = app();
    app.navigation.screen = Screen::Workbench;
    app.navigation.target = WorkbenchTarget::Session;
    app.navigation.update_presentation(|p| p.split = true);
    let mut details = Details::new("orchestrator".into());
    details.native_areas[0] = Rect::new(25, 2, 50, 10);
    details.editing = false;
    app.details = Some(details);
    app.workbench_sessions
        .push(crate::lean::store::SessionInfo {
            turn_id: "theme-turn".into(),
            plan_id: None,
            role: "orchestrator".into(),
            provider: "codex".into(),
            session_id: Some("capture-session".into()),
            status: "running".into(),
            error: None,
            upto_message: 1,
            started_ms: 0,
            finished_ms: None,
            pid: None,
        });
    let screen = crate::lean::terminal::Screen {
        handle: "theme-test".into(),
        revision: 1,
        columns: 50,
        rows: 10,
        cursor: None,
        error: None,
        cells: vec![crate::lean::terminal::Cell {
            x: 0,
            y: 0,
            text: "╭".into(),
            width: 1,
            foreground: Some([1, 2, 3]),
            background: Some([4, 5, 6]),
            modifiers: 1,
        }],
    };
    app.native_views.insert(
        format!("{}:orchestrator", app.conversation),
        native_view::capture_fixture_view(screen, None),
    );
    let buffer = draw(&mut app, (120, 40));
    let cli = buffer
        .content
        .iter()
        .find(|cell| cell.fg == Color::Rgb(1, 2, 3))
        .expect("native CLI color remains independent of application mode");
    assert_eq!(cli.symbol(), "╭");
    assert_eq!(cli.bg, Color::Rgb(4, 5, 6));
    assert!(cli.modifier.contains(Modifier::BOLD));
    assert_eq!(theme.accent().fg, theme.color.then_some(theme::ayu::CYAN));
}

#[test]
fn shared_d3_surfaces_and_semantic_tones() {
    let theme = Theme::new(true, false);
    for (actual, expected) in [
        (theme.overlay().bg, Color::Rgb(0x10, 0x16, 0x19)),
        (theme.panel().bg, Color::Rgb(0x19, 0x22, 0x26)),
        (theme.selected().bg, Color::Rgb(0x2a, 0x3c, 0x40)),
        (theme.raised().bg, Color::Rgb(0x22, 0x30, 0x35)),
        (theme.text().fg, Color::Rgb(0xe7, 0xed, 0xeb)),
        (theme.metadata().fg, Color::Rgb(0xa1, 0xb1, 0xb3)),
        (theme.accent().fg, Color::Rgb(0xa4, 0xdf, 0xcc)),
        (theme.decision().fg, Color::Rgb(0xe9, 0xc5, 0x93)),
    ] {
        assert_eq!(actual, Some(expected));
    }
    assert_ne!(theme.error().fg, theme.ok().fg);
    assert_eq!(theme.borders().top_left, "┌");
    assert_eq!(Theme::new(false, true).borders().top_left, "+");
}

fn separator_owner_states(theme: &Theme) {
    let separator = theme.chrome("·");
    let mut graph = app();
    graph.selected = "worker:t1".into();
    graph.graph_inspector_visible = true;
    let buffer = draw(&mut graph, (166, 19));
    application_cells(&buffer, theme, "graph inspector 166x19");
    assert!(text(&buffer).contains("Enter Workbench"));
    graph.graph_expanded = Some("worker:t1".into());
    graph.view.tasks[0].round = 3;
    let mut details = Details::new("worker:t1".into());
    details.available_rounds = vec![1, 2, 3];
    details.selected_round = Some(2);
    graph.details = Some(details);
    let buffer = draw(&mut graph, (200, 60));
    application_cells(&buffer, theme, "expanded graph round");
    assert!(text(&buffer).contains(&format!(
        "Round 2 (2/3) {separator} , previous {separator} . next"
    )));

    for mode in 0..4 {
        let mut owner = app();
        let task = &owner.view.tasks[0];
        owner.flow_editor = Some(match mode {
            0 => FlowEditor::edit_task(&owner.view, task),
            1 => FlowEditor::add_dependent(&owner.view, &task.id),
            2 => FlowEditor::split_task(&owner.view, task),
            _ => FlowEditor::retire_task(task),
        });
        for size in [(166, 40), (40, 12), (12, 5), (11, 4)] {
            application_cells(&draw(&mut owner, size), theme, "flow editor variant");
        }
    }
    let mut filters = app();
    filters.graph_filters = Some(Default::default());
    application_cells(&draw(&mut filters, (166, 40)), theme, "graph filters");
    let mut actions = app();
    actions.view.tasks[0].status = "needs_user".into();
    actions.unblock = Some(unblock::Unblock::new("p1", &actions.view.tasks[0]));
    application_cells(&draw(&mut actions, (166, 40)), theme, "task actions");
    for confirming in [false, true] {
        let mut owner = app();
        owner.recovery_consent = Some(consent::RecoveryConsent {
            plan: "p1".into(),
            task: "t1".into(),
            status: "needs_user".into(),
            target: "project".into(),
            base: "base".into(),
            result: "snapshot".into(),
            token: "token".into(),
            require_all: false,
            files: vec![("code.rs".into(), true)],
            selected: 0,
            scroll: 0,
            confirming,
            error: None,
        });
        application_cells(&draw(&mut owner, (166, 40)), theme, "consent variant");
    }
    for target in [
        "unblock:retry_with_guidance:p1:t1",
        "reject:p1",
        "worker:t1",
    ] {
        let mut owner = app();
        owner.editor = Some(Editor {
            target: target.into(),
            input: InputBuffer::default(),
        });
        application_cells(&draw(&mut owner, (240, 40)), theme, "editor title variant");
    }
    for state in 0..3 {
        let mut owner = app();
        let mut browser = planhistory::Browser::new();
        if state > 0 {
            browser.loaded(crate::lean::store::PlansPage {
                plans: if state == 2 {
                    vec![crate::lean::store::PlanHistory {
                        scope_id: None,
                        id: "p1".into(),
                        conversation_id: "c1".into(),
                        created_ms: 0,
                        status: "approved".into(),
                        is_active: true,
                        tasks: vec![],
                        agents: vec![],
                        turns: vec![],
                        reports: vec![],
                        verdicts: vec![],
                    }]
                } else {
                    vec![]
                },
                offset: 0,
                limit: 1,
                total: usize::from(state == 2),
                has_more: false,
                active_plan_id: None,
            });
        }
        owner.plan_history = Some(browser);
        application_cells(
            &draw(&mut owner, (166, 40)),
            theme,
            "history navigation variant",
        );
    }
    // Dynamic title and status belong to the user, even where they resemble chrome.
    graph.view.tasks[0].title = "User · title".into();
    graph.view.tasks[0].status = "User · status".into();
    let buffer = draw(&mut graph, (240, 60));
    assert!(text(&buffer).contains(&format!("User · title {separator} User · status")));
}

fn compact_statuses() {
    let directory = tempfile::tempdir().unwrap();
    let store = crate::lean::Store::open(&directory.path().join("history.db")).unwrap();
    let conversation = store.create_conversation("project").unwrap();
    for _ in 0..2 {
        store
            .add_plan(
                &conversation.id,
                &[json!({"id":"saved", "title":"Saved task", "change_kind":"none"})],
            )
            .unwrap();
    }
    let page = store.plans_page(&conversation.id, 0, 1).unwrap();
    let theme = Theme::from_env();
    for size in [(12, 5), (40, 12), (80, 24), (100, 30), (120, 40), (160, 50)] {
        let mut app = app();
        app.flow_editor = Some(FlowEditor::edit_task(&app.view, &app.view.tasks[0]));
        app.flow_editor.as_mut().unwrap().criteria_input =
            InputBuffer::with_text("C1: Status remains accessible");
        app.flow_editor.as_mut().unwrap().sending = true;
        let buffer = draw(&mut app, size);
        assert!(
            text(&buffer).contains("Saving"),
            "{size:?}: {}",
            text(&buffer)
        );
        application_cells(&buffer, &theme, "flow waiting");
        let editor = app.flow_editor.as_mut().unwrap();
        editor.sending = false;
        let error = "BEGIN raw 👩‍💻 e\u{301} ".to_owned()
            + &(0..30)
                .map(|index| format!("segment-{index:02}-abcdefghij"))
                .collect::<String>()
            + " END";
        editor.error = Some(error.clone());
        let title = editor.title_input.text.clone();
        let width = usize::from(size.0.saturating_sub(4).clamp(12, 82) - 2);
        let expected = theme::wrap(&error, width);
        let mut seen = vec![false; expected.len()];
        let mut seen_end = false;
        for _ in 0..error.len() {
            let buffer = draw(&mut app, size);
            assert!(
                text(&buffer).contains("Err Pg"),
                "{size:?}: {}",
                text(&buffer)
            );
            application_cells(&buffer, &theme, "flow error");
            let visible = text(&buffer);
            for (row, found) in expected.iter().zip(&mut seen) {
                *found |= visible.contains(row);
            }
            if visible.contains("END") {
                seen_end = true;
                break;
            }
            app.key(event::KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        }
        assert!(seen_end, "{size:?}: error tail inaccessible");
        assert!(
            seen.iter().all(|found| *found),
            "{size:?}: skipped visual error rows"
        );
        for _ in 0..expected.len() {
            app.key(event::KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        }
        assert!(text(&draw(&mut app, size)).contains("BEGIN"));
        app.key(event::KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        let editor = app.flow_editor.as_ref().unwrap();
        assert_eq!(editor.error.as_deref(), Some(error.as_str()));
        assert_eq!(editor.title_input.text, title);
        assert_eq!(editor.field, 0);
        let buffer = draw(&mut app, size);
        assert!(text(&buffer).contains("Title:") || text(&buffer).contains("Title:"));
        for _ in 0..8 {
            app.key(event::KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        }
        assert_eq!(app.flow_editor.as_ref().unwrap().field, 8);
        app.key(event::KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        assert!(app.flow_editor.as_ref().unwrap().request_ai_review);
        let editor = app.flow_editor.as_ref().unwrap();
        assert!(editor.build_revision(&app.view).unwrap().1);
        let title = editor.title_input.text.clone();
        app.flow_editor.as_mut().unwrap().title_input = InputBuffer::new();
        app.key(event::KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.flow_editor.as_ref().unwrap().error.is_some());
        assert!(!app.flow_editor.as_ref().unwrap().sending);
        app.flow_editor.as_mut().unwrap().title_input = InputBuffer::with_text(title);
        app.key(event::KeyEvent::new(
            KeyCode::Char('s'),
            KeyModifiers::CONTROL,
        ));
        assert!(app.flow_editor.as_ref().unwrap().sending);
        assert!(text(&draw(&mut app, size)).contains("Saving"));
        app.key(event::KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.flow_editor.is_none());
        app.plan_history = Some(planhistory::Browser::new());
        let buffer = draw(&mut app, size);
        assert!(
            text(&buffer).contains("loading"),
            "{size:?}: {}",
            text(&buffer)
        );
        assert!(!text(&buffer).contains("No tasks proposed"));
        application_cells(&buffer, &theme, "history loading");
        app.apply_update(Update::PlanHistory {
            conversation: app.conversation.clone(),
            generation: app.generation,
            offset: 0,
            reply: Ok(page.clone()),
        });
        let buffer = draw(&mut app, size);
        assert!(
            text(&buffer).contains("plan 1"),
            "{size:?}: {}",
            text(&buffer)
        );
        app.key(event::KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE));
        let browser = app.plan_history.as_ref().unwrap();
        assert!(browser.loading);
        assert_eq!(browser.offset, 1);
        let buffer = draw(&mut app, size);
        assert!(text(&buffer).contains("loading"));
        if size.0 >= 80 {
            assert!(text(&buffer).contains("Original plan"));
        }
        app.key(event::KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE));
        assert_eq!(app.plan_history.as_ref().unwrap().offset, 1);
        app.key(event::KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        app.key(event::KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.plan_history.is_none());
        assert_eq!(app.input.text, "Zażółć 👩‍💻 e\u{301}");
        assert_eq!(app.view.tasks.len(), 2);
    }
}

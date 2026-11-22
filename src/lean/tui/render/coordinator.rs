use super::super::*;

impl App {
    pub fn draw(&mut self, frame: &mut Frame) {
        self.draw_with_theme(frame, &Theme::from_env());
    }

    pub(in crate::lean::tui) fn draw_with_theme(&mut self, frame: &mut Frame, theme: &Theme) {
        self.phase_action_area = None;
        self.phase_action_command = None;
        self.now_click_targets.clear();
        self.inspector_hits.clear();
        self.chrome_keys.clear();
        self.direction_actions = None;
        self.direction_area = Rect::default();
        self.direction_input_area = Rect::default();
        self.direction_choices.clear();
        self.sync_interview();
        self.terminal_area = frame.area();
        self.graph_area = None;
        self.now_area = None;
        self.text_area = None;
        frame.render_widget(Block::default().style(theme.overlay()), frame.area());
        if draw_min_term_guard(frame, frame.area()) {
            self.text_selection = Default::default();
            return;
        }
        self.sync_native_views();
        self.sync_approval();
        self.sync_rigor_recommendation();
        if super::plan_history::draw_plan_history(self, frame, theme) {
            return;
        }
        if let Some(editor) = self.config_editor.as_ref().filter(|e| e.visible) {
            self.text_selection = Default::default();
            editor.draw_themed(frame, theme);
            return;
        }
        super::specialized_screens::draw_secondary_or_main(self, frame, theme);
        super::overlays::draw_remaining(self, frame, theme);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lean::tui::{Editor, InputBuffer, palette::Help};
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn workbench_f5_targets_the_visible_agent_before_graph_and_audit() {
        for diff in [false, true] {
            for audit_retryable in [false, true] {
                let mut app = App::test_app();
                app.view.plan = Some("p1".into());
                app.view.tasks = vec![view::TaskRow {
                    id: "t1".into(),
                    status: if audit_retryable {
                        "approved"
                    } else {
                        "working"
                    }
                    .into(),
                    ..Default::default()
                }];
                if audit_retryable {
                    app.view.audit_error = Some("audit interrupted".into());
                }
                assert_eq!(app.view.audit_retryable(), audit_retryable);
                app.selected = ORCHESTRATOR.into();
                app.input = InputBuffer::with_text("draft 👩‍💻 é");
                if diff {
                    app.open_diff("worker:t1");
                } else {
                    app.open_details("worker:t1".into());
                }
                let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
                terminal.draw(|frame| app.draw(frame)).unwrap();
                assert_eq!(app.selected, ORCHESTRATOR);
                app.key(event::KeyEvent::new(
                    KeyCode::F(5),
                    event::KeyModifiers::NONE,
                ));
                let f5 = app.unblock.take();
                let notice = app.notice.take();
                if audit_retryable {
                    assert!(f5.is_none(), "approved task has no recovery action");
                    assert_eq!(
                        notice.as_deref(),
                        Some("✗ no step is stopped; nothing to unblock")
                    );
                } else {
                    assert_eq!(f5.as_ref().unwrap().task, "t1");
                }
                // Compare with the existing explicit Details action on the same target.
                app.diff = None;
                app.key(event::KeyEvent::new(
                    KeyCode::Char('u'),
                    event::KeyModifiers::NONE,
                ));
                assert_eq!(app.unblock, f5);
                assert_eq!(app.notice, notice);
                assert_eq!(app.input.text, "draft 👩‍💻 é");
            }
        }
    }

    #[test]
    fn workbench_navigation_keeps_agent_and_comment_editor_text() {
        let mut app = App::test_app();
        app.input = InputBuffer::with_text("chat draft");
        app.open_details("worker:t1".into());
        let initial_tab = app.navigation.presentation().tab;
        app.details.as_mut().unwrap().editing = true;
        for character in "pz,.".chars() {
            app.key(event::KeyEvent::new(
                KeyCode::Char(character),
                event::KeyModifiers::NONE,
            ));
        }
        let details = app.details.as_ref().unwrap();
        let draft_key = details.composer_key();
        assert_eq!(details.drafts.borrow()[&draft_key].input.text, "pz,.");
        assert_eq!(details.selected_round, None);
        assert_eq!(app.navigation.presentation().tab, initial_tab);
        assert!(!app.navigation.presentation().zoomed);
        assert!(!app.navigation.presentation().pinned);

        app.details.as_mut().unwrap().editing = false;
        app.editor = Some(Editor {
            target: "worker:t1".into(),
            input: InputBuffer::with_text(""),
        });
        for character in "pz,.".chars() {
            app.key(event::KeyEvent::new(
                KeyCode::Char(character),
                event::KeyModifiers::NONE,
            ));
        }
        assert_eq!(app.editor.as_ref().unwrap().input.text, "pz,.");
        assert_eq!(
            app.details.as_ref().unwrap().drafts.borrow()[&draft_key]
                .input
                .text,
            "pz,."
        );
        assert_eq!(app.input.text, "chat draft");
    }

    #[test]
    fn global_project_shortcut_reaches_workbench_diff_but_respects_editors() {
        let mut app = App::test_app();
        app.input = InputBuffer::with_text("chat draft");
        app.open_diff("worker:t1");
        let project_key = event::KeyEvent::new(KeyCode::Char('g'), event::KeyModifiers::CONTROL);
        app.key(project_key);
        assert_eq!(app.picker.as_ref().unwrap().kind, Kind::Projects);
        app.key(event::KeyEvent::new(
            KeyCode::Esc,
            event::KeyModifiers::NONE,
        ));
        assert!(app.diff.is_some());
        assert_eq!(app.navigation.presentation().tab, WorkbenchTab::Diff);
        assert_eq!(app.input.text, "chat draft");
        app.diff = None;
        app.details.as_mut().unwrap().editing = true;
        app.key(project_key);
        assert!(app.picker.is_none());
        assert!(app.details.as_ref().unwrap().editing);
        assert_eq!(app.input.text, "chat draft");

        app.details.as_mut().unwrap().editing = false;
        app.navigation
            .update_presentation(|presentation| presentation.tab = WorkbenchTab::Results);
        app.details
            .as_mut()
            .unwrap()
            .prepare_workbench_details(None, WorkbenchTab::Results);
        app.key(event::KeyEvent::new(
            KeyCode::Char('1'),
            event::KeyModifiers::NONE,
        ));
        assert_eq!(app.details.as_ref().unwrap().tab, details::RESULT);
        assert_eq!(app.navigation.presentation().tab, WorkbenchTab::Results);
        app.key(event::KeyEvent::new(
            KeyCode::Right,
            event::KeyModifiers::CONTROL,
        ));
        assert_eq!(app.navigation.presentation().tab, WorkbenchTab::Checks);
        assert_eq!(app.details.as_ref().unwrap().tab, details::CHECKS);
    }

    #[test]
    fn task_diff_uses_workbench_and_keeps_the_chat_draft() {
        let mut app = App::test_app();
        app.view.tasks = vec![view::TaskRow {
            id: "t1".into(),
            ..Default::default()
        }];
        app.input = InputBuffer::with_text("draft 👩‍💻 é");
        assert_eq!(app.navigation.screen, Screen::Graph);
        app.open_diff("worker:t1");
        assert_eq!(app.navigation.screen, Screen::Workbench);
        assert_eq!(
            app.navigation.target,
            WorkbenchTarget::Agent("worker:t1".into())
        );
        assert_eq!(app.navigation.presentation().tab, WorkbenchTab::Diff);
        assert_eq!(app.details.as_ref().unwrap().key, "worker:t1");
        assert_eq!(app.diff.as_ref().unwrap().task, "t1");
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert!(app.workbench_areas.content.width > 0);
        assert!(app.workbench_areas.terminal.height > 0);
        assert!(app.graph_area.is_none());
        assert_eq!(app.input.text, "draft 👩‍💻 é");
        app.key(event::KeyEvent::new(
            KeyCode::Esc,
            event::KeyModifiers::NONE,
        ));
        assert!(app.diff.is_none());
        assert!(app.details.is_some());
        assert_eq!(app.navigation.presentation().tab, WorkbenchTab::Summary);
        assert_eq!(app.input.text, "draft 👩‍💻 é");

        app.details.as_mut().unwrap().selected_round = Some(1);
        app.key(event::KeyEvent::new(
            KeyCode::Esc,
            event::KeyModifiers::NONE,
        ));
        app.selected = "worker:t1".into();
        app.key(event::KeyEvent::new(
            KeyCode::Char('v'),
            event::KeyModifiers::NONE,
        ));
        assert_eq!(app.navigation.screen, Screen::Workbench);
        assert_eq!(app.details.as_ref().unwrap().selected_round, Some(1));
        assert!(app.historical_diff_selected());
        assert_eq!(app.input.text, "draft 👩‍💻 é");
    }

    #[test]
    fn draw_keeps_graph_viewport_in_sync_and_guards_small_terminals() {
        let mut app = App::test_app();
        for (width, height) in [(140, 30), (100, 24), (99, 24), (85, 24), (84, 24), (12, 5)] {
            for focus in [Focus::Chat, Focus::Graph] {
                app.focus = focus;
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                let theme = Theme::new(true, false);
                terminal
                    .draw(|frame| app.draw_with_theme(frame, &theme))
                    .unwrap();
                assert_eq!(app.terminal_area, Rect::new(0, 0, width, height));
                let graph_area = app.graph_area.expect("Canvas always renders");
                assert_eq!(
                    app.graph_viewport,
                    Some((graph_area.width, graph_area.height))
                );
                assert!(graph_area.right() <= width);
                assert!(
                    graph_area.right() >= width.saturating_sub(if width >= 120 { 44 } else { 3 })
                );
            }
        }

        app.now_click_targets
            .push(Some(("stale-agent".into(), true)));
        let mut tiny = Terminal::new(TestBackend::new(11, 4)).unwrap();
        tiny.draw(|frame| app.draw(frame)).unwrap();
        assert!(app.graph_area.is_none());
        assert!(app.text_area.is_none());
        assert!(app.now_click_targets.is_empty());
        let rendered = (0..4)
            .map(|row| {
                (0..11)
                    .map(|column| tiny.backend().buffer()[(column, row)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join(" ");
        let rendered = rendered.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(rendered.contains("Terminal too small"));
    }

    #[test]
    fn scope_decision_waits_for_resize_below_minimum_terminal_size() {
        let mut app = App::test_app();
        app.view.workflow_phase = Some("interview".into());
        app.view.user_decision = true;
        app.view.scope_confirmation_summary = Some("FULL-SCOPE-CONTENT".into());
        let question = serde_json::from_value(serde_json::json!({
            "id":"scope-confirmation",
            "context":"Confirm scope.",
            "question":"Proceed?",
            "options":["Confirm scope", "Request changes"]
        }))
        .unwrap();
        app.view.interview_question = Some((42, question));

        let mut terminal = Terminal::new(TestBackend::new(11, 4)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(text.contains("Terminal too small"));
        assert!(!text.contains("FULL-SCOPE-CONTENT"));
        assert!(!text.contains("Confirm scope"));
    }

    #[test]
    fn short_multiline_editor_preserves_overlay_precedence() {
        let mut app = App::test_app();
        app.editor = Some(Editor {
            target: "plan:p1".into(),
            input: InputBuffer::with_text("\n\n\n\n"),
        });
        app.help = Some(Help::Comment);
        let mut terminal = Terminal::new(TestBackend::new(20, 7)).unwrap();

        terminal.draw(|frame| app.draw(frame)).unwrap();

        let rendered = (0..7)
            .map(|row| {
                (0..20)
                    .map(|column| terminal.backend().buffer()[(column, row)].symbol())
                    .collect::<String>()
            })
            .collect::<String>();
        assert!(!rendered.contains("Enter, Ctrl+S"));
    }

    #[test]
    fn direction_edits_in_place_and_questions_preserve_the_background_draft() {
        for (width, height) in [(80, 24), (160, 50)] {
            let mut app = App::test_app();
            app.input = InputBuffer::with_text("direction draft 👩‍💻é");
            app.key(event::KeyEvent::new(
                KeyCode::Char('k'),
                KeyModifiers::CONTROL,
            ));
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| app.draw(frame)).unwrap();
            assert!(app.direction_input_area.y >= height - 8);
            assert!(
                app.direction_input_area
                    .contains(terminal.get_cursor_position().unwrap())
            );
            let input_bottom = app.direction_area.bottom();
            assert!(
                app.chrome_keys
                    .iter()
                    .any(|(rect, code, _)| rect.y == input_bottom && *code == KeyCode::Enter)
            );
            assert!(app.graph_area.unwrap().bottom() <= app.direction_area.y);
            if width == 160 {
                let help = app
                    .chrome_keys
                    .iter()
                    .find(|(_, code, _)| *code == KeyCode::F(1))
                    .unwrap()
                    .0;
                let send = app.direction_actions.unwrap().0;
                app.mouse(event::MouseEvent {
                    kind: event::MouseEventKind::Down(event::MouseButton::Left),
                    column: help.x,
                    row: help.y,
                    modifiers: KeyModifiers::NONE,
                });
                assert!(app.help.is_some());
                app.mouse(event::MouseEvent {
                    kind: event::MouseEventKind::Down(event::MouseButton::Left),
                    column: send.x,
                    row: send.y,
                    modifiers: KeyModifiers::NONE,
                });
                assert!(
                    !app.sending,
                    "a covering dialog isolates the dock's Send action"
                );
                app.key(event::KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
            }
            app.view.apply(
                &serde_json::json!({"seq":1, "kind":"user_decision_requested", "payload":{
                "id":"scope", "question":"Which scope?", "context":"Choose the scope to implement.",
                "options":[{"label":"Small"},{"label":"Full"}]}}),
            );
            terminal.draw(|frame| app.draw(frame)).unwrap();
            assert_eq!(app.navigation.focus, NavigationFocus::Composer);
            assert!(
                !app.interview_visible(),
                "incoming questions do not steal focus"
            );
            let own = app
                .direction_choices
                .iter()
                .find(|(_, action)| matches!(action, super::super::direction::Action::Answer(2)))
                .unwrap()
                .0;
            app.mouse(event::MouseEvent {
                kind: event::MouseEventKind::Down(event::MouseButton::Left),
                column: own.x,
                row: own.y,
                modifiers: KeyModifiers::NONE,
            });
            assert!(app.interview_visible());
            app.paste("custom answer 👩‍💻é");
            terminal.draw(|frame| app.draw(frame)).unwrap();
            assert!(
                app.direction_input_area
                    .contains(terminal.get_cursor_position().unwrap())
            );
            let first = app
                .direction_choices
                .iter()
                .find(|(_, action)| matches!(action, super::super::direction::Action::Answer(0)))
                .unwrap()
                .0;
            app.mouse(event::MouseEvent {
                kind: event::MouseEventKind::Down(event::MouseButton::Left),
                column: first.x,
                row: first.y,
                modifiers: KeyModifiers::NONE,
            });
            assert_eq!(app.interview_forms[&app.conversation].selected, 0);
            assert!(
                !app.interview_forms[&app.conversation].sending,
                "selection does not submit"
            );
            app.key(event::KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
            assert!(!app.interview_visible());
            assert_eq!(app.input.text, "direction draft 👩‍💻é");
            assert_eq!(
                app.interview_forms[&app.conversation].input.text,
                "custom answer 👩‍💻é"
            );
            assert!(app.decision_inbox.is_none());
        }
    }

    #[test]
    fn session_header_shows_branch_status_and_total_time_without_collisions() {
        for width in [40, 80, 160] {
            for ascii in [false, true] {
                let theme = Theme::new(true, ascii);
                let mut app = App::test_app();
                app.view.idle = true;
                app.projects.push(workspace::ProjectConn {
                    path: "test".into(),
                    name: "test".into(),
                    client: app.client.clone(),
                    branch: Some("main".into()),
                    conversations: Vec::new(),
                });
                for (seq, kind, timestamp) in
                    [(1, "turn_started", 1_000), (2, "turn_finished", 61_000)]
                {
                    app.view.apply(&serde_json::json!({"seq":seq, "kind":kind, "turn_id":"header-turn", "created_ms":timestamp,
                        "payload":{"role":"orchestrator", "status":"done"}}));
                }
                let mut terminal = Terminal::new(TestBackend::new(width, 40)).unwrap();
                terminal
                    .draw(|frame| app.draw_with_theme(frame, &theme))
                    .unwrap();
                let row = |terminal: &Terminal<TestBackend>| {
                    (0..width)
                        .map(|x| terminal.backend().buffer()[(x, 1)].symbol())
                        .collect::<String>()
                };
                assert!(row(&terminal).contains("> antislop  main"));
                assert!(row(&terminal).contains("ready"));
                assert!(row(&terminal).contains("01:00"));
                app.view.session_branch = Some(format!("feat/👩‍💻é{}", "-long".repeat(80)));
                app.view.paused = true;
                app.navigation.screen = Screen::Workbench;
                app.details = Some(Details::new("conversation".into()));
                terminal
                    .draw(|frame| app.draw_with_theme(frame, &theme))
                    .unwrap();
                let header = row(&terminal);
                assert!(header.contains("feat/👩‍💻"), "{width}: {header}");
                assert!(header.contains("é"), "{width}: {header}");
                assert!(header.contains("paused"), "{header}");
                assert!(header.contains("01:00"), "{header}");
                assert!(!header.contains("Canvas") && !header.contains("/ session"));
            }
        }
    }

    #[test]
    fn approved_reference_spacing_and_surfaces_at_160_by_50() {
        let theme = Theme::new(true, false);
        let mut app = App::test_app();
        let mut terminal = Terminal::new(TestBackend::new(160, 50)).unwrap();
        terminal
            .draw(|frame| app.draw_with_theme(frame, &theme))
            .unwrap();
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(2, 1)].symbol(), ">");
        assert_eq!(buffer[(2, 1)].fg, crate::lean::tui::theme::ayu::CYAN);
        assert_eq!(buffer[(122, 3)].bg, crate::lean::tui::theme::ayu::PANEL);
        assert_eq!(buffer[(121, 3)].bg, crate::lean::tui::theme::ayu::BG);
        assert_eq!(buffer[(2, 46)].bg, crate::lean::tui::theme::ayu::PANEL);
        assert_eq!(buffer[(2, 48)].symbol(), "↑");
        assert!(
            app.chrome_keys
                .iter()
                .any(|(rect, key, mods)| rect.height == 2
                    && *key == KeyCode::Char('k')
                    && mods.contains(KeyModifiers::CONTROL))
        );
        app.view.tasks.push(view::TaskRow {
            id: "t1".into(),
            title: "Create PR".into(),
            change_kind: crate::lean::tasks::ChangeKind::Code,
            ..Default::default()
        });
        app.open_details("worker:t1".into());
        terminal
            .draw(|frame| app.draw_with_theme(frame, &theme))
            .unwrap();
        let areas = app.workbench_areas;
        assert_eq!(areas.list.width, 30);
        assert_eq!(areas.content.x, 35);
        assert!(areas.content.height.abs_diff(areas.terminal.height) <= 1);
        let buffer = terminal.backend().buffer();
        assert_eq!(
            buffer[(areas.content.x, areas.content.y)].bg,
            crate::lean::tui::theme::ayu::PANEL
        );
        assert_eq!(
            buffer[(areas.content.x - 1, areas.content.y)].bg,
            crate::lean::tui::theme::ayu::BG
        );
        assert_eq!(
            buffer[(areas.terminal.x, areas.terminal.y)].bg,
            crate::lean::tui::theme::ayu::RAISED
        );
        assert_eq!(
            buffer[(areas.terminal.x, areas.terminal.y + 3)].bg,
            crate::lean::tui::theme::ayu::TERMINAL
        );
    }
}

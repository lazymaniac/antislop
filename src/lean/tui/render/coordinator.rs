use super::super::*;

impl App {
    pub fn draw(&mut self, frame: &mut Frame) {
        self.draw_with_theme(frame, &Theme::from_env());
    }

    pub(in crate::lean::tui) fn draw_with_theme(&mut self, frame: &mut Frame, theme: &Theme) {
        self.phase_action_area = None;
        self.phase_action_command = None;
        self.now_click_targets.clear();
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
        assert_eq!(app.navigation.presentation().tab, WorkbenchTab::Comments);
        assert_eq!(app.details.as_ref().unwrap().tab, details::COMMENTS);
    }

    #[test]
    fn task_diff_uses_workbench_and_keeps_the_chat_draft() {
        let mut app = App::test_app();
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
        app.navigation.screen = Screen::Graph;
        app.focus = Focus::Graph;
        app.selected = "worker:t1".into();
        app.key(event::KeyEvent::new(
            KeyCode::Char('d'),
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
                if width >= SIDE_BY_SIDE_BREAKPOINT || focus == Focus::Graph {
                    let graph_area = app.graph_area.expect("graph panel rendered");
                    assert_eq!(
                        app.graph_viewport,
                        Some((graph_area.width, graph_area.height))
                    );
                    assert!(graph_area.right() >= width.saturating_sub(3));
                    let fill = if focus == Focus::Graph {
                        theme.panel()
                    } else {
                        theme.overlay()
                    };
                    for y in [0, graph_area.y] {
                        let edge = &terminal.backend().buffer()[(width - 1, y)];
                        assert_eq!(edge.symbol(), " ", "canvas has no outer border");
                        assert_eq!(edge.bg, fill.bg.unwrap());
                    }
                    if width >= SIDE_BY_SIDE_BREAKPOINT {
                        assert!(graph_area.x > app.chat_area.right());
                    }
                } else {
                    assert!(app.graph_area.is_none());
                    assert_eq!(app.chat_area.width, width);
                }
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
}

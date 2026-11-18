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
        if draw_min_term_guard(frame, frame.area()) {
            self.text_selection = Default::default();
            return;
        }
        self.sync_native_views();
        self.sync_approval();
        if super::plan_history::draw_plan_history(self, frame) {
            return;
        }
        if let Some(editor) = self.config_editor.as_ref().filter(|e| e.visible) {
            self.text_selection = Default::default();
            editor.draw(frame);
            return;
        }
        super::specialized_screens::draw_secondary_or_main(self, frame, theme);
        super::overlays::draw_remaining(self, frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lean::tui::{Editor, InputBuffer, palette::Help};
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn draw_keeps_graph_viewport_in_sync_and_guards_small_terminals() {
        let mut app = App::test_app();
        for (width, height) in [(140, 30), (100, 24), (99, 24), (85, 24), (84, 24), (12, 5)] {
            for focus in [Focus::Chat, Focus::Graph] {
                app.focus = focus;
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal.draw(|frame| app.draw(frame)).unwrap();
                assert_eq!(app.terminal_area, Rect::new(0, 0, width, height));
                if width >= SIDE_BY_SIDE_BREAKPOINT || focus == Focus::Graph {
                    let graph_area = app.graph_area.expect("graph panel rendered");
                    assert_eq!(
                        app.graph_viewport,
                        Some((graph_area.width, graph_area.height))
                    );
                    assert!(graph_area.right() >= width.saturating_sub(3));
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

use super::super::*;

impl App {
    pub fn draw(&mut self, frame: &mut Frame) {
        self.phase_action_area = None;
        self.phase_action_command = None;
        self.sync_interview();
        self.terminal_area = frame.area();
        self.graph_area = None;
        self.text_area = None;
        if draw_min_term_guard(frame, frame.area()) {
            self.text_selection = Default::default();
            return;
        }
        self.sync_native_views();
        if super::ledger::draw_history_or_ledger(self, frame) {
            return;
        }
        if let Some(editor) = self.config_editor.as_ref().filter(|e| e.visible) {
            self.text_selection = Default::default();
            editor.draw(frame);
            return;
        }
        super::specialized_screens::draw_secondary_or_main(self, frame);
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
        app.focus = Focus::Graph;
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let graph_area = app.graph_area.expect("graph panel rendered");
        assert_eq!(
            app.graph_viewport,
            Some((graph_area.width, graph_area.height))
        );
        assert_eq!(app.terminal_area, Rect::new(0, 0, 80, 24));

        let mut tiny = Terminal::new(TestBackend::new(11, 4)).unwrap();
        tiny.draw(|frame| app.draw(frame)).unwrap();
        assert!(app.graph_area.is_none());
        assert!(app.text_area.is_none());
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

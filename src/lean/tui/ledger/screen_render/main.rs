use super::super::*;
use crate::lean::tui::Theme;
use ratatui::{Frame, layout::Rect, widgets::Paragraph};

impl LedgerView {
    pub fn draw(&mut self, frame: &mut Frame, area: Rect) {
        self.area = area;
        if area.width < 12 || area.height < 5 {
            frame.render_widget(Paragraph::new("Resize terminal"), area);
            return;
        }
        if self.editing.is_some() && self.preview.is_none() {
            self.draw_form(frame, area);
            return;
        }
        let theme = Theme::from_env();
        if self.confirm_new_run {
            self.draw_new_run_confirmation(frame, area, &theme);
            return;
        }
        self.draw_body(frame, area, &theme);
    }
}

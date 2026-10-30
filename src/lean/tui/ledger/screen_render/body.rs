use super::super::*;
use crate::lean::tui::Theme;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    widgets::Block,
};

impl LedgerView {
    pub(super) fn draw_body(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        frame.render_widget(Block::default().style(theme.panel()), area);
        let rows = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Min(0),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);
        self.draw_header(frame, &rows, theme);
        self.list_area = Rect::default();
        self.detail_area = Rect::default();
        if self.preview.is_some() {
            self.draw_review(frame, &rows, theme);
            return;
        }
        self.draw_browse(frame, &rows, theme);
    }

    fn draw_browse(&mut self, frame: &mut Frame, rows: &[Rect], theme: &Theme) {
        let body = rows[3];
        let wide = body.width >= 96;
        let panes = Layout::default()
            .direction(if wide {
                ratatui::layout::Direction::Horizontal
            } else {
                ratatui::layout::Direction::Vertical
            })
            .constraints(if wide {
                vec![Constraint::Percentage(38), Constraint::Percentage(62)]
            } else {
                vec![
                    Constraint::Length((body.height / 3).max(3).min(body.height)),
                    Constraint::Min(0),
                ]
            })
            .split(body);
        self.draw_record_list(frame, panes[0], theme);
        self.draw_detail_pane(frame, panes[1], theme);
        self.draw_browse_footer(frame, rows[4], rows[5], theme);
    }
}

use super::super::comparison::panel;
use super::super::model::TABS;
use super::super::*;
use crate::lean::tui::Theme;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::Line,
    widgets::Paragraph,
};

impl LedgerView {
    pub(super) fn draw_detail_pane(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let block = panel(theme, " Details ");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(inner);
        self.draw_tabs(frame, rows[0], theme);
        self.detail_area = rows[1];
        self.draw_detail_content(frame, theme);
    }

    fn draw_tabs(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let tabs = TABS
            .iter()
            .enumerate()
            .map(|(index, label)| {
                let style = if self.tab == index {
                    theme.selected()
                } else {
                    theme.dim()
                };
                ratatui::text::Span::styled(format!(" {label} "), style)
            })
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(Line::from(tabs)), area);
    }

    fn draw_detail_content(&mut self, frame: &mut Frame, theme: &Theme) {
        let lines = self.detail_lines(theme, self.detail_area.width as usize);
        self.scroll = self
            .scroll
            .min(lines.len().saturating_sub(self.detail_area.height as usize));
        frame.render_widget(
            Paragraph::new(lines).scroll((self.scroll.min(u16::MAX as usize) as u16, 0)),
            self.detail_area,
        );
    }
}

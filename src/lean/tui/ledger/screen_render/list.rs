use super::super::comparison::panel;
use super::super::*;
use crate::lean::tui::{Theme, view::wrap};
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::Paragraph,
};

impl LedgerView {
    pub(super) fn draw_record_list(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let block = panel(theme, " Records ");
        self.list_area = block.inner(area);
        frame.render_widget(block, area);
        let visible = (self.list_area.height as usize / 3).max(1);
        self.list_start = self.selected.saturating_sub(visible - 1);
        let entries = self.filtered();
        let rows = self.record_lines(&entries, visible, theme);
        frame.render_widget(Paragraph::new(rows), self.list_area);
    }

    fn record_lines(
        &self,
        entries: &[&crate::lean::ledger::LedgerEntry],
        visible: usize,
        theme: &Theme,
    ) -> Vec<Line<'static>> {
        let mut rows = Vec::new();
        for (index, entry) in entries
            .iter()
            .enumerate()
            .skip(self.list_start)
            .take(visible)
        {
            rows.extend(self.entry_lines(index, entry, theme));
        }
        if entries.is_empty() {
            rows.extend(self.empty_lines(theme));
        }
        rows
    }

    fn entry_lines(
        &self,
        index: usize,
        entry: &crate::lean::ledger::LedgerEntry,
        theme: &Theme,
    ) -> Vec<Line<'static>> {
        vec![
            self.entry_title(index, entry, theme),
            self.entry_status(entry, theme),
            Line::from(""),
        ]
    }

    fn entry_title(
        &self,
        index: usize,
        entry: &crate::lean::ledger::LedgerEntry,
        theme: &Theme,
    ) -> Line<'static> {
        let style = if index == self.selected {
            theme.selected()
        } else {
            theme.text()
        };
        let pointer = if index == self.selected {
            theme.glyphs().pointer
        } else {
            " "
        };
        let text = theme.fit(
            &format!(" {pointer} {}", entry.spec.interpretation),
            self.list_area.width as usize,
        );
        Line::from(Span::styled(text, style))
    }

    fn entry_status(
        &self,
        entry: &crate::lean::ledger::LedgerEntry,
        theme: &Theme,
    ) -> Line<'static> {
        let id = theme.fit(
            &format!("   {}  ", entry.spec.id),
            self.list_area.width as usize / 2,
        );
        let label = format!("{} {}", theme.status_glyph(&entry.status), entry.status);
        Line::from(vec![
            Span::styled(id, theme.dim()),
            Span::styled(label, theme.status(&entry.status)),
        ])
    }

    fn empty_lines(&self, _theme: &Theme) -> Vec<Line<'static>> {
        let message = if self.pending {
            "Loading records..."
        } else if !self.query.text.is_empty() {
            "No matches. Change the search or filters."
        } else {
            "No requirements yet. Continue the interview or add a requirement with n."
        };
        wrap(message, self.list_area.width as usize)
            .into_iter()
            .map(Line::from)
            .collect::<Vec<_>>()
    }
}

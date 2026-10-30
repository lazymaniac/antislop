use super::super::comparison::panel;
use super::super::*;
use crate::lean::tui::{Theme, sanitize, visual_text_lines};
use ratatui::{Frame, layout::Rect, text::Line, widgets::Paragraph};

impl LedgerView {
    pub(super) fn draw_review(&mut self, frame: &mut Frame, rows: &[Rect], theme: &Theme) {
        if self.rejecting {
            self.draw_rejection(frame, rows[3], theme);
        } else {
            self.draw_preview(frame, rows[3], theme);
        }
        let actions = if self.rejecting {
            " Ctrl+Enter Reject with comment | Esc Back to diff"
        } else {
            " Enter Approve | r Reject with comment | Esc Close | Ctrl+R Reload"
        };
        frame.render_widget(Paragraph::new(actions).style(theme.selected()), rows[4]);
        let status = self.error.as_deref().unwrap_or(if self.pending {
            "Applying changes... Your review is preserved."
        } else {
            "Requirements approval does not start implementation."
        });
        let style = if self.error.is_some() {
            theme.error()
        } else {
            theme.dim()
        };
        frame.render_widget(Paragraph::new(sanitize(status)).style(style), rows[5]);
    }

    fn draw_rejection(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let block = panel(theme, " Rejection comment ");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.width > 0 && inner.height > 0 {
            self.draw_rejection_input(frame, inner, theme);
        }
    }

    fn draw_rejection_input(&mut self, frame: &mut Frame, inner: Rect, theme: &Theme) {
        let (column, row) = self.rejection.visual_cursor(inner.width as usize);
        self.edit_scroll = self.edit_scroll.min(row);
        if row >= self.edit_scroll + inner.height as usize {
            self.edit_scroll = row.saturating_sub(inner.height as usize - 1);
        }
        let lines = visual_text_lines(&self.rejection.text, inner.width as usize)
            .into_iter()
            .skip(self.edit_scroll)
            .take(inner.height as usize)
            .map(Line::from)
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(lines).style(theme.text()), inner);
        frame.set_cursor_position((
            inner.x.saturating_add(column).min(inner.right() - 1),
            inner.y + (row - self.edit_scroll) as u16,
        ));
    }

    fn draw_preview(&mut self, frame: &mut Frame, body: Rect, theme: &Theme) {
        let lines = self.preview_lines(theme, body.width.saturating_sub(4) as usize);
        self.scroll = self.scroll.min(
            lines
                .len()
                .saturating_sub(body.height.saturating_sub(2) as usize),
        );
        frame.render_widget(
            Paragraph::new(lines)
                .scroll((self.scroll.min(u16::MAX as usize) as u16, 0))
                .block(panel(theme, " Exact changes ")),
            body,
        );
    }
}

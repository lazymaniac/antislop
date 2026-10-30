use super::super::comparison::panel;
use super::super::*;
use crate::lean::tui::{Theme, sanitize, view::wrap};
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::Paragraph,
};

impl LedgerView {
    pub(super) fn draw_new_run_confirmation(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let text = "Create a new requirements draft?\n\nThis creates a separate empty set. It does not replace the requirements used by current planning.\n\nEnter creates the draft. Esc returns to your existing requirements.";
        let lines = wrap(text, area.width.saturating_sub(4) as usize)
            .into_iter()
            .map(Line::from)
            .collect::<Vec<_>>();
        frame.render_widget(
            Paragraph::new(lines)
                .style(theme.text())
                .block(panel(theme, " New draft set ")),
            area,
        );
    }

    pub(super) fn draw_header(&self, frame: &mut Frame, rows: &[Rect], theme: &Theme) {
        frame.render_widget(
            Paragraph::new(" Requirements ").style(theme.selected()),
            rows[1],
        );
        frame.render_widget(Paragraph::new(self.heading()).style(theme.title()), rows[0]);
        frame.render_widget(
            Paragraph::new(sanitize(&self.filter_label())).style(theme.dim()),
            rows[2],
        );
        self.draw_workflow_notice(frame, rows[2], theme);
    }

    fn heading(&self) -> String {
        self.page
            .as_ref()
            .map(|page| {
                format!(
                    " {}   /   revision {}",
                    sanitize(&page.run.goal),
                    page.run.spec_revision
                )
            })
            .unwrap_or_else(|| " Requirements workspace".into())
    }

    fn filter_label(&self) -> String {
        if self.searching {
            return format!("Search: {}", self.query.text);
        }
        if self.status_edit {
            return format!("Status: {}", self.status.text);
        }
        if self.task_filter_edit {
            return format!("Task: {}", self.task_filter.text);
        }
        let search = if self.query.text.is_empty() {
            "/ Search".into()
        } else {
            format!("Search: {}", self.query.text)
        };
        let coverage = if self.missing_only {
            "Missing coverage"
        } else {
            "All coverage"
        };
        format!(
            " {} records  |  {search}  |  {coverage}",
            self.page.as_ref().map_or(0, |page| page.total)
        )
    }

    fn draw_workflow_notice(&self, frame: &mut Frame, row: Rect, theme: &Theme) {
        let Some((failed, message)) = &self.workflow_notice else {
            return;
        };
        let style = if *failed {
            theme.error()
        } else {
            theme.accent()
        };
        let area = Rect::new(
            row.x,
            row.y.saturating_add(1),
            row.width,
            row.height.saturating_sub(1),
        );
        frame.render_widget(Paragraph::new(sanitize(message)).style(style), area);
    }

    pub(super) fn draw_browse_footer(
        &self,
        frame: &mut Frame,
        actions: Rect,
        status_area: Rect,
        theme: &Theme,
    ) {
        let footer = Line::from(vec![
            Span::styled(" p  Review pending changes ", theme.accent()),
            Span::styled(
                " Up/Down Select  |  Left/Right Details  |  e Edit  |  Esc Close",
                theme.dim(),
            ),
        ]);
        frame.render_widget(Paragraph::new(footer), actions);
        frame.render_widget(
            Paragraph::new(self.status_label()).style(self.status_style(theme)),
            status_area,
        );
    }

    fn status_label(&self) -> String {
        if let Some(error) = &self.error {
            return format!("Error: {}. Draft preserved.", sanitize(error));
        }
        if self.pending {
            return "Loading... Your drafts are preserved.".into();
        }
        self.notice.clone().unwrap_or_else(|| "n Add  |  N New draft set  |  g Browse sets  |  / Search  |  f Status  |  t Task  |  m Gaps  |  j/k Scroll  |  r Refresh".into())
    }

    fn status_style(&self, theme: &Theme) -> ratatui::style::Style {
        if self.error.is_some() {
            theme.error()
        } else {
            theme.dim()
        }
    }
}

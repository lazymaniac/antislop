use super::model::LABELS;
use super::*;
use crate::lean::tui::{InputBuffer, sanitize};
use ratatui::{Frame, layout::Rect, text::Line, widgets::Paragraph};
impl LedgerView {
    pub(super) fn form_area(&self) -> Rect {
        let left = if self.area.width >= 60 { 26 } else { 1 };
        Rect::new(
            self.area.x.saturating_add(left),
            self.area.y.saturating_add(4),
            self.area.width.saturating_sub(left + 1).max(1),
            self.area.height.saturating_sub(7).max(1),
        )
    }
    pub(super) fn draw_form(&mut self, frame: &mut Frame, area: Rect) {
        let body = self.form_area();
        let Some(form) = self.editing.as_ref().and_then(|key| self.drafts.get(key)) else {
            return;
        };
        draw_form_heading(frame, area, form);
        if area.width >= 60 {
            draw_form_labels(frame, area, body, form);
        }
        let input = &form.fields[form.field as usize];
        draw_form_input(frame, body, &mut self.edit_scroll, input);
        self.draw_form_status(frame, area);
    }

    fn draw_form_status(&self, frame: &mut Frame, area: Rect) {
        let status = self.error.as_deref().unwrap_or(if self.pending {
            "Waiting for daemon; draft preserved"
        } else {
            "Original source is immutable. Add a user correction in the New source field."
        });
        frame.render_widget(
            Paragraph::new(sanitize(status)),
            Rect::new(area.x, area.bottom().saturating_sub(2), area.width, 1),
        );
        frame.render_widget(
            Paragraph::new(
                "Criteria: ID | required/optional | integration/check | text. Enter adds a line.",
            ),
            Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
        );
    }
}

fn draw_form_heading(frame: &mut Frame, area: Rect, form: &super::Form) {
    let heading = format!("Edit {} · revision {}", form.base.id, form.revision);
    frame.render_widget(
        Paragraph::new(heading),
        Rect::new(area.x, area.y, area.width, 1),
    );
    frame.render_widget(
        Paragraph::new("Tab/Shift+Tab field · Ctrl+S preview · Ctrl+R reload · Esc keep draft"),
        Rect::new(area.x, area.y + 1, area.width, 1),
    );
    frame.render_widget(
        Paragraph::new(format!(
            "{}: {}",
            form.field as usize + 1,
            LABELS[form.field as usize]
        )),
        Rect::new(area.x, area.y + 2, area.width, 1),
    );
}

fn draw_form_labels(frame: &mut Frame, area: Rect, body: Rect, form: &super::Form) {
    let start = (form.field as usize).saturating_sub(body.height as usize / 2);
    let labels = LABELS
        .iter()
        .enumerate()
        .skip(start)
        .take(body.height as usize)
        .map(|(i, label)| {
            Line::from(format!(
                "{} {}",
                if i == form.field as usize { ">" } else { " " },
                label
            ))
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(labels),
        Rect::new(area.x, body.y, 24, body.height),
    );
}

fn draw_form_input(frame: &mut Frame, body: Rect, scroll: &mut usize, input: &InputBuffer) {
    let (column, row) = input.visual_cursor(body.width as usize);
    if row < *scroll {
        *scroll = row;
    } else if row >= *scroll + body.height as usize {
        *scroll = row.saturating_sub(body.height as usize - 1);
    }
    let lines = crate::lean::tui::visual_text_lines(&input.text, body.width as usize)
        .into_iter()
        .skip(*scroll)
        .take(body.height as usize)
        .map(Line::from)
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines), body);
    let cursor_y = body
        .y
        .saturating_add(row.saturating_sub(*scroll).min(u16::MAX as usize) as u16);
    if cursor_y < body.bottom() {
        frame.set_cursor_position((
            body.x
                .saturating_add(column)
                .min(body.right().saturating_sub(1)),
            cursor_y,
        ));
    }
}

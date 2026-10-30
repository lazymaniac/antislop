use super::model::{LABELS, TABS};
use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
impl LedgerView {
    pub fn mouse(&mut self, mouse: MouseEvent) -> Action {
        if self.preview.is_some() {
            self.scroll_mouse(mouse.kind);
            return Action::None;
        }
        if self.editing.is_some() {
            self.edit_mouse(mouse);
            return Action::None;
        }
        self.browse_mouse(mouse)
    }

    fn edit_mouse(&mut self, mouse: MouseEvent) {
        let body = self.form_area();
        if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
            self.edit_click(mouse, body);
        } else {
            self.edit_scroll(mouse.kind, body.width as usize);
        }
    }

    fn edit_click(&mut self, mouse: MouseEvent, body: ratatui::layout::Rect) {
        if self.label_click(mouse, body) {
            return;
        }
        if mouse.row == self.area.y + 2 {
            self.next_form_field();
            return;
        }
        if body.contains((mouse.column, mouse.row).into()) {
            self.place_form_cursor(mouse, body);
        }
    }

    fn label_click(&mut self, mouse: MouseEvent, body: ratatui::layout::Rect) -> bool {
        if self.area.width < 60
            || mouse.column >= body.x
            || mouse.row < body.y
            || mouse.row >= body.bottom()
        {
            return false;
        }
        let visible = body.height as usize;
        let start = self
            .form_mut()
            .map_or(0, |form| (form.field as usize).saturating_sub(visible / 2));
        if let Some(form) = self.form_mut() {
            form.field = FormField::ALL
                [(start + mouse.row.saturating_sub(body.y) as usize).min(LABELS.len() - 1)];
        }
        self.edit_scroll = 0;
        true
    }

    fn next_form_field(&mut self) {
        if let Some(form) = self.form_mut() {
            form.field = form.field.next(false);
        }
        self.edit_scroll = 0;
    }

    fn place_form_cursor(&mut self, mouse: MouseEvent, body: ratatui::layout::Rect) {
        let row = mouse.row.saturating_sub(body.y) as usize + self.edit_scroll;
        let Some(form) = self.form_mut() else {
            return;
        };
        let input = &mut form.fields[form.field as usize];
        let lines =
            crate::lean::tui::editor::compute_visual_lines(&input.text, body.width as usize);
        let Some(line) = lines.get(row) else {
            return;
        };
        let column = mouse.column.saturating_sub(body.x);
        input.cursor = line
            .chars
            .iter()
            .find(|(_, x, width)| column < x.saturating_add(*width))
            .map(|(byte, _, _)| *byte)
            .unwrap_or(line.end_byte);
        input.preferred_col = None;
    }

    fn edit_scroll(&mut self, kind: MouseEventKind, width: usize) {
        if !matches!(kind, MouseEventKind::ScrollUp | MouseEventKind::ScrollDown) {
            return;
        }
        let Some(form) = self.form_mut() else {
            return;
        };
        for _ in 0..3 {
            if kind == MouseEventKind::ScrollUp {
                form.fields[form.field as usize].move_up(width);
            } else {
                form.fields[form.field as usize].move_down(width);
            }
        }
    }

    fn browse_mouse(&mut self, mouse: MouseEvent) -> Action {
        if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
            if mouse.row == self.area.bottom().saturating_sub(2)
                && mouse.column.saturating_sub(self.area.x) < 26
            {
                return Action::Proposal;
            }
            if self.tab_click(mouse) {
                return Action::Detail;
            }
            if let Some(row) = self.record_click(mouse) {
                return self.select_record(row);
            }
        }
        if self.detail_area.contains((mouse.column, mouse.row).into()) {
            self.scroll_mouse(mouse.kind);
            return Action::None;
        }
        self.scroll_key(mouse.kind)
    }

    fn tab_click(&mut self, mouse: MouseEvent) -> bool {
        if self.detail_area.width == 0
            || mouse.row != self.detail_area.y.saturating_sub(1)
            || mouse.column < self.detail_area.x
            || mouse.column >= self.detail_area.right()
        {
            return false;
        }
        let column = mouse.column.saturating_sub(self.detail_area.x) as usize;
        let mut end = 0;
        for (index, label) in TABS.iter().enumerate() {
            end += unicode_width::UnicodeWidthStr::width(*label) + 2;
            if column < end {
                self.invalidate_read();
                self.tab = index;
                self.scroll = 0;
                self.source = None;
                return true;
            }
        }
        false
    }

    fn record_click(&self, mouse: MouseEvent) -> Option<usize> {
        if !self.list_area.contains((mouse.column, mouse.row).into()) {
            return None;
        }
        let row = self.list_start + mouse.row.saturating_sub(self.list_area.y) as usize / 3;
        (row < self.filtered().len()).then_some(row)
    }

    fn select_record(&mut self, row: usize) -> Action {
        self.invalidate_read();
        self.selected = row;
        self.scroll = 0;
        self.detail = None;
        self.source = None;
        Action::Detail
    }

    fn scroll_mouse(&mut self, kind: MouseEventKind) {
        match kind {
            MouseEventKind::ScrollDown => self.scroll = self.scroll.saturating_add(3),
            MouseEventKind::ScrollUp => self.scroll = self.scroll.saturating_sub(3),
            _ => {}
        }
    }

    fn scroll_key(&mut self, kind: MouseEventKind) -> Action {
        let code = match kind {
            MouseEventKind::ScrollDown => KeyCode::Down,
            MouseEventKind::ScrollUp => KeyCode::Up,
            _ => return Action::None,
        };
        self.key(KeyEvent::new(code, KeyModifiers::NONE), "")
    }
}

use super::*;
impl ConfigEditor {
    pub fn draw(&self, frame: &mut Frame) {
        self.draw_with_theme(
            frame,
            &Theme::from_env(),
            super::super::trust::config_dir()
                .ok()
                .map(|p| p.join("antislop.toml"))
                .as_deref(),
        );
    }

    pub fn draw_with_theme(
        &self,
        frame: &mut Frame,
        theme: &Theme,
        global: Option<&std::path::Path>,
    ) {
        let area = frame.area();
        if area.width < 12 || area.height < 5 {
            return;
        }
        frame.render_widget(Clear, area);
        let rows = usize::from(area.height.saturating_sub(5));
        let start = self.selected.saturating_sub(rows.saturating_sub(1));
        let mut lines = self.field_lines(start, rows, global, theme);
        let cursor = self.selected_line(area, theme);
        lines.push(cursor);
        lines.push(Line::raw(
            "Tab: field  Enter: cycle  Ctrl+N/D: add/delete profile/provider/route  Ctrl+S: apply  Ctrl+R: reload  Esc: close",
        ));
        lines.push(Line::raw(super::super::sanitize_line(self.status_line())));
        frame.render_widget(Paragraph::new(lines), area);
        frame.set_cursor_position((
            area.x + self.cursor_column(area),
            area.bottom().saturating_sub(3),
        ));
    }

    fn field_lines(
        &self,
        start: usize,
        rows: usize,
        global: Option<&std::path::Path>,
        theme: &Theme,
    ) -> Vec<Line<'static>> {
        let origin = if global == Some(std::path::Path::new(&self.path)) {
            "global"
        } else {
            "project"
        };
        let mut lines = vec![
            Line::styled(
                super::super::sanitize_line(&format!("Configuration ({origin}): {}", self.path)),
                theme.title(),
            ),
            Line::raw("Running turns finish with their old profile."),
        ];
        lines.extend(self.visible_fields(start, rows, theme));
        lines.resize_with(2 + rows, || Line::raw(""));
        lines
    }

    fn visible_fields(&self, start: usize, rows: usize, theme: &Theme) -> Vec<Line<'static>> {
        self.fields
            .iter()
            .enumerate()
            .skip(start)
            .take(rows)
            .map(|(index, field)| self.field_line(index, field, theme))
            .collect()
    }

    fn field_line(&self, index: usize, field: &Field, theme: &Theme) -> Line<'static> {
        let error = field
            .error
            .as_ref()
            .map(|e| format!("  ERROR: {e}"))
            .unwrap_or_default();
        let label = format!("{} = {}{error}", field.path.join("."), field.input.text);
        let style = if index == self.selected {
            theme.selected()
        } else {
            theme.text()
        };
        Line::styled(super::super::sanitize_line(&label), style)
    }

    fn selected_line(&self, area: Rect, theme: &Theme) -> Line<'static> {
        let selected = &self.fields[self.selected];
        let width = usize::from(area.width.saturating_sub(1).max(1));
        let wrapped = super::super::visual_text_lines(&selected.input.text, width);
        let (_, row) = selected.input.visual_cursor(width);
        Line::styled(
            wrapped.get(row).cloned().unwrap_or_default(),
            theme.selected(),
        )
    }

    fn status_line(&self) -> &str {
        if self.adding.is_some() {
            &self.status
        } else {
            self.fields[self.selected]
                .error
                .as_deref()
                .unwrap_or(&self.status)
        }
    }

    fn cursor_column(&self, area: Rect) -> u16 {
        let width = usize::from(area.width.saturating_sub(1).max(1));
        let (col, _) = self.fields[self.selected].input.visual_cursor(width);
        col.min(area.width.saturating_sub(1))
    }
}

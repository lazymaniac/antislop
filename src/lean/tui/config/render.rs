use super::*;
use ratatui::{layout::Rect, text::Span};
use unicode_width::UnicodeWidthStr;

impl ConfigEditor {
    pub fn draw(&self, frame: &mut Frame) {
        self.draw_with_theme(
            frame,
            &Theme::from_env(),
            crate::lean::paths::config_dir()
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
        if area.width < 40 || area.height < 9 {
            self.draw_compact(frame, area, theme);
            return;
        }
        let panel = super::super::panel_area(area, 84);
        let origin = if global == Some(std::path::Path::new(&self.path)) {
            "global"
        } else {
            "project"
        };
        let title = Line::styled(
            theme.clean(&format!("Configuration ({origin}): {}", self.path)),
            theme.title(),
        );
        let footer_y = panel.bottom().saturating_sub(2);
        let content_y = (panel.y + 2).min(footer_y);
        let content_height = footer_y.saturating_sub(content_y);
        let left_width = (panel.width / 4)
            .clamp(14, 24)
            .min(panel.width.saturating_sub(1));
        let left = Rect::new(panel.x, content_y, left_width, content_height);
        let right = Rect::new(
            left.right().saturating_add(1).min(panel.right()),
            content_y,
            panel.right().saturating_sub(left.right().saturating_add(1)),
            content_height,
        );
        frame.render_widget(
            Paragraph::new(vec![title]),
            Rect::new(panel.x, panel.y, panel.width, 1),
        );
        frame.render_widget(
            Paragraph::new(Line::raw("Running turns finish with their old profile.")),
            Rect::new(panel.x, panel.y.saturating_add(1), panel.width, 1),
        );

        let sections = self.sections();
        let active = self
            .fields
            .get(self.selected)
            .map(|field| field.path[0].as_str());
        let section_rows: Vec<Line<'static>> = sections
            .iter()
            .map(|(name, count)| {
                let selected = Some(name.as_str()) == active;
                let style = if selected {
                    theme.selected()
                } else {
                    theme.text()
                };
                let label = format!("{} {} ({count})", if selected { ">" } else { " " }, name);
                Line::styled(super::super::sanitize_line(&label), style)
            })
            .collect();
        frame.render_widget(Paragraph::new(section_rows), left);

        let active_fields: Vec<(usize, &Field)> = self
            .fields
            .iter()
            .enumerate()
            .filter(|(_, field)| Some(field.path[0].as_str()) == active)
            .collect();
        let selected_local = active_fields
            .iter()
            .position(|(index, _)| *index == self.selected)
            .unwrap_or(0);
        let viewport = usize::from(right.height.saturating_sub(1));
        let start = selected_local.saturating_sub(viewport.saturating_sub(1));
        let mut lines = Vec::new();
        if let Some(name) = active {
            lines.push(Line::styled(
                format!("{name}: {} fields", active_fields.len()),
                theme.accent(),
            ));
        }
        lines.extend(
            active_fields
                .iter()
                .skip(start)
                .take(viewport)
                .map(|(index, field)| {
                    self.field_line(*index, field, theme, usize::from(right.width))
                }),
        );
        frame.render_widget(Paragraph::new(lines), right);

        let status = self.status_line();
        let actions = if self.pending {
            "Applying..."
        } else {
            "Ctrl+S save/apply | Ctrl+R refresh | Esc close"
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(super::super::sanitize_line(actions), theme.accent()),
                Span::raw("  "),
                Span::styled(super::super::sanitize_line(status), theme.dim()),
            ])),
            Rect::new(
                panel.x,
                footer_y,
                panel.width,
                panel.bottom().saturating_sub(footer_y),
            ),
        );
        if let Some((_, field)) = active_fields
            .iter()
            .find(|(index, _)| *index == self.selected)
            && let Some(y) = self.field_cursor_row(right, selected_local, start, field)
        {
            let width = usize::from(right.width);
            let prefix = format!("{} = ", field.path.join("."));
            let prefix_width = prefix.width().min(width);
            let (column, _) = field
                .input
                .visual_cursor(width.saturating_sub(prefix_width).max(1));
            frame.set_cursor_position((
                right.x + (prefix_width + usize::from(column)).min(width.saturating_sub(1)) as u16,
                y,
            ));
        }
    }

    fn draw_compact(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let field = &self.fields[self.selected];
        let title = format!("Config: {}", self.path);
        frame.render_widget(
            Paragraph::new(Line::styled(
                fit(theme, &title, usize::from(area.width)),
                theme.title(),
            )),
            Rect::new(area.x, area.y, area.width, 1),
        );
        if area.height > 1 {
            let (value, column) = selected_field_line(field, usize::from(area.width));
            frame.render_widget(
                Paragraph::new(Line::styled(theme.clean(&value), theme.selected())),
                Rect::new(area.x, area.y + 1, area.width, 1),
            );
            frame.set_cursor_position((area.x + column.min(area.width - 1), area.y + 1));
        }
        if area.height > 3 {
            let error = field.error.as_deref().unwrap_or(self.status_line());
            frame.render_widget(
                Paragraph::new(Line::styled(
                    fit(theme, error, usize::from(area.width)),
                    theme.dim(),
                )),
                Rect::new(area.x, area.bottom() - 2, area.width, 1),
            );
        }
        if area.height > 2 {
            frame.render_widget(
                Paragraph::new(Line::raw("Ctrl+S save | Ctrl+R refresh | Esc close")),
                Rect::new(area.x, area.bottom() - 1, area.width, 1),
            );
        }
    }

    fn sections(&self) -> Vec<(String, usize)> {
        let mut sections: Vec<(String, usize)> = Vec::new();
        for field in &self.fields {
            let name = field.path[0].clone();
            if let Some((_, count)) = sections.iter_mut().find(|(section, _)| *section == name) {
                *count += 1;
            } else {
                sections.push((name, 1));
            }
        }
        sections
    }

    fn field_line(
        &self,
        index: usize,
        field: &Field,
        theme: &Theme,
        width: usize,
    ) -> Line<'static> {
        if index == self.selected {
            let (label, _) = selected_field_line(field, width);
            return Line::styled(theme.clean(&label), theme.selected());
        }
        let error = field
            .error
            .as_ref()
            .map(|error| format!(" ERROR: {error}"))
            .unwrap_or_default();
        let label = format!("{} = {}{error}", field.path.join("."), field.input.text);
        let style = if index == self.selected {
            theme.selected()
        } else if field.error.is_some() {
            theme.error()
        } else {
            theme.text()
        };
        Line::styled(fit(theme, &label, width), style)
    }

    fn field_cursor_row(
        &self,
        area: Rect,
        selected: usize,
        start: usize,
        _field: &Field,
    ) -> Option<u16> {
        let row = selected.saturating_sub(start);
        (row < usize::from(area.height)).then_some(area.y + row as u16 + 1)
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
}

fn fit(theme: &Theme, text: &str, width: usize) -> String {
    theme.fit(&theme.clean(text), width)
}

fn selected_field_line(field: &Field, width: usize) -> (String, u16) {
    let prefix = format!("{} = ", field.path.join("."));
    let prefix_width = prefix.width().min(width);
    let value_width = width.saturating_sub(prefix_width).max(1);
    let (column, row) = field.input.visual_cursor(value_width);
    let lines = super::super::visual_text_lines(&field.input.text, value_width);
    let value = lines.get(row).map(String::as_str).unwrap_or_default();
    let error = field
        .error
        .as_ref()
        .map(|error| format!(" ERROR: {error}"))
        .unwrap_or_default();
    (
        format!("{prefix}{value}{error}"),
        (prefix_width + usize::from(column)).min(width.saturating_sub(1)) as u16,
    )
}

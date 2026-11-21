use super::super::*;

/// Keep the editing surface visually centered and bounded on wide terminals.
pub(super) fn centered_area(area: Rect) -> Rect {
    let width = area.width.min(84);
    Rect::new(
        area.x.saturating_add(area.width.saturating_sub(width) / 2),
        area.y,
        width,
        area.height,
    )
}

pub(super) fn panel_editor_area(area: Rect) -> Rect {
    let area = centered_area(area);
    Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(1),
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    )
}

/// Reserve an interior cell for the insertion cursor after a full visual line.
pub(super) fn panel_wrap_width(area: Rect) -> usize {
    usize::from(panel_editor_area(area).width.saturating_sub(1)).max(1)
}

pub(super) struct Panel<'a> {
    pub title: &'a str,
    pub status: &'a str,
    pub lines: &'a [String],
    pub focused: bool,
    pub cursor: Option<(u16, usize, usize)>,
}

/// Draw the shared composer shell. Callers adapt their own draft state into
/// these display values and retain ownership of editing and delivery.
pub(super) fn draw_panel(frame: &mut Frame, area: Rect, panel: Panel<'_>, theme: &Theme) -> Rect {
    let area = centered_area(area);
    if area.width == 0 || area.height == 0 {
        return panel_editor_area(area);
    }
    let fill = if panel.focused {
        theme.raised()
    } else {
        theme.card()
    };
    let border = fill.patch(theme.faint());
    let buffer = frame.buffer_mut();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            buffer[(x, y)].set_style(fill);
        }
    }
    if area.height >= 2 {
        for x in area.left().saturating_add(1)..area.right().saturating_sub(1) {
            buffer[(x, area.y)]
                .set_symbol(if theme.ascii { "-" } else { "─" })
                .set_style(border);
            buffer[(x, area.bottom() - 1)]
                .set_symbol(if theme.ascii { "-" } else { "─" })
                .set_style(border);
        }
        for y in area.top().saturating_add(1)..area.bottom().saturating_sub(1) {
            buffer[(area.x, y)]
                .set_symbol(if theme.ascii { "|" } else { "│" })
                .set_style(border);
            buffer[(area.right() - 1, y)]
                .set_symbol(if theme.ascii { "|" } else { "│" })
                .set_style(border);
        }
        buffer[(area.x, area.y)]
            .set_symbol(if theme.ascii { "+" } else { "┌" })
            .set_style(border);
        buffer[(area.right() - 1, area.y)]
            .set_symbol(if theme.ascii { "+" } else { "┐" })
            .set_style(border);
        buffer[(area.x, area.bottom() - 1)]
            .set_symbol(if theme.ascii { "+" } else { "└" })
            .set_style(border);
        buffer[(area.right() - 1, area.bottom() - 1)]
            .set_symbol(if theme.ascii { "+" } else { "┘" })
            .set_style(border);
    }
    if area.height >= 1 && area.width > 2 {
        buffer.set_string(
            area.x + 1,
            area.y,
            theme.fit(&theme.clean(panel.title), usize::from(area.width - 2)),
            fill.patch(theme.dim()),
        );
    }
    if area.height >= 2 && area.width > 2 {
        buffer.set_string(
            area.x + 1,
            area.bottom() - 1,
            theme.fit(&theme.clean(panel.status), usize::from(area.width - 2)),
            fill.patch(theme.dim()),
        );
    }
    let editor = panel_editor_area(area);
    if editor.width > 0 && editor.height > 0 {
        frame.render_widget(
            Paragraph::new(
                panel
                    .lines
                    .iter()
                    .cloned()
                    .map(Line::raw)
                    .collect::<Vec<_>>(),
            )
            .style(fill.patch(theme.text())),
            Rect {
                width: editor.width.saturating_sub(1),
                ..editor
            },
        );
        if panel.focused
            && let Some((column, row, first_line)) = panel.cursor
        {
            let cursor_x = editor
                .x
                .saturating_add(column.min(editor.width.saturating_sub(1)))
                .min(editor.right().saturating_sub(1));
            let cursor_y = editor
                .y
                .saturating_add(row.saturating_sub(first_line) as u16)
                .min(editor.bottom().saturating_sub(1));
            frame.set_cursor_position((cursor_x, cursor_y));
        }
    }
    editor
}

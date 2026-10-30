use super::super::super::*;

pub(super) fn draw_editor(app: &mut App, frame: &mut Frame) -> bool {
    let Some(editor) = &app.editor else {
        return false;
    };
    let area = frame.area();
    let inner_width = usize::from(area.width.saturating_sub(3).max(1));
    app.editor_width = inner_width;
    let lines = visual_text_lines(&editor.input.text, inner_width);
    let height = lines.len().clamp(1, 6) as u16 + 2;
    if area.height < height + 1 {
        return true;
    }
    let visible = usize::from(height.saturating_sub(2)).max(1);
    let (column, row) = editor.input.visual_cursor(inner_width);
    let shown = row
        .saturating_sub(visible.saturating_sub(1))
        .min(lines.len().saturating_sub(visible));
    let end = (shown + visible).min(lines.len());
    let box_area = Rect::new(area.x, area.bottom() - 1 - height, area.width, height);
    frame.render_widget(Clear, box_area);
    frame.render_widget(editor_box(editor, &lines[shown..end]), box_area);
    draw_editor_cursor(app, frame, box_area, column, row, shown, visible);
    false
}

fn editor_box(editor: &Editor, lines: &[String]) -> Paragraph<'static> {
    Paragraph::new(lines.iter().cloned().map(Line::raw).collect::<Vec<_>>()).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::ayu::BLUE))
            .title(editor_title(&editor.target)),
    )
}

fn editor_title(target: &str) -> String {
    if let Some(rest) = target.strip_prefix("unblock:") {
        let task = rest.rsplit(':').next().unwrap_or_default();
        let action = rest.split(':').next().unwrap_or_default();
        format!(
            " {} {task} · optional text · Enter send · Alt+Enter newline · Esc cancel ",
            action.replace('_', " ")
        )
    } else if let Some(plan) = target.strip_prefix("reject:") {
        format!(
            " reject plan {plan} · optional reason · Enter reject · Alt+Enter newline · Esc cancel "
        )
    } else {
        format!(
            " comment on {} · Enter send · Alt+Enter newline · Esc cancel ",
            target_name(target)
        )
    }
}

fn draw_editor_cursor(
    app: &App,
    frame: &mut Frame,
    area: Rect,
    column: u16,
    row: usize,
    shown: usize,
    visible: usize,
) {
    if app.help.is_some()
        || app.picker.is_some()
        || app.trust_prompt.is_some()
        || area.width < 3
        || area.height < 3
    {
        return;
    }
    let min_x = area.x + 1;
    let max_x = area.right().saturating_sub(2);
    let min_y = area.y + 1;
    let max_y = area.bottom().saturating_sub(2);
    let x = (area.x + 1 + column).clamp(min_x, max_x);
    let y = (area.y + 1 + row.saturating_sub(shown).min(visible - 1) as u16).clamp(min_y, max_y);
    frame.set_cursor_position((x, y));
}

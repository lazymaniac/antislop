use super::super::super::*;

pub(super) fn draw_editor(app: &mut App, frame: &mut Frame) -> bool {
    let Some(editor) = &app.editor else {
        return false;
    };
    let area = frame.area();
    let inner_width = usize::from(area.width.saturating_sub(3).max(1));
    app.editor_width = inner_width;
    let lines = visual_text_lines(&editor.input.text, inner_width);
    let comments: Vec<String> = app
        .view
        .comments_on(&editor.target)
        .into_iter()
        .take(4)
        .map(|comment| format!("#{} [{}] {}", comment.id, comment.status, comment.text))
        .collect();
    let comment_rows = comments.len().min(4);
    let height = (lines.len().clamp(1, 6) + comment_rows + 1).min(9) as u16 + 2;
    if area.height < height + 1 {
        return true;
    }
    let visible = usize::from(height.saturating_sub(2))
        .saturating_sub(comment_rows)
        .saturating_sub(1)
        .max(1);
    let (column, row) = editor.input.visual_cursor(inner_width);
    let shown = row
        .saturating_sub(visible.saturating_sub(1))
        .min(lines.len().saturating_sub(visible));
    let end = (shown + visible).min(lines.len());
    let box_area = Rect::new(area.x, area.bottom() - 1 - height, area.width, height);
    frame.render_widget(Clear, box_area);
    frame.render_widget(editor_box(editor, &comments, &lines[shown..end]), box_area);
    draw_editor_cursor(
        app,
        frame,
        box_area,
        (column, row),
        (shown, visible),
        comment_rows as u16,
    );
    false
}

fn editor_box(editor: &Editor, comments: &[String], lines: &[String]) -> Paragraph<'static> {
    let rows = comments
        .iter()
        .cloned()
        .map(Line::raw)
        .chain(std::iter::once(Line::raw(editor_prompt(&editor.target))))
        .chain(lines.iter().cloned().map(Line::raw))
        .collect::<Vec<_>>();
    Paragraph::new(rows).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::ayu::BLUE))
            .title(editor_title(&editor.target)),
    )
}

fn editor_title(target: &str) -> String {
    if let Some(rest) = target.strip_prefix("unblock:") {
        let mut parts = rest.splitn(3, ':');
        let action = parts.next().unwrap_or_default();
        let plan = parts.next().unwrap_or_default();
        let task = parts.next().unwrap_or_default();
        format!(
            " {} · plan {plan} · task {task} · optional scoped text · Enter send · Alt+Enter newline · Esc cancel ",
            action.replace('_', " "),
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

fn editor_prompt(target: &str) -> String {
    if let Some(rest) = target.strip_prefix("unblock:") {
        let action = rest.split(':').next().unwrap_or_default();
        return match action {
            "retry_with_guidance" => "Hint for this worker (optional):".into(),
            "restart_agent" => "Context for the fresh agent session (optional):".into(),
            _ => "Additional context for this recovery action (optional):".into(),
        };
    }
    if let Some(plan) = target.strip_prefix("reject:") {
        return format!("Reason for rejecting plan {plan} (optional):");
    }
    format!("Comment on {}:", target_name(target))
}

fn draw_editor_cursor(
    app: &App,
    frame: &mut Frame,
    area: Rect,
    cursor: (u16, usize),
    window: (usize, usize),
    comment_rows: u16,
) {
    if app.help.is_some() || app.picker.is_some() || area.width < 3 || area.height < 3 {
        return;
    }
    let min_x = area.x + 1;
    let max_x = area.right().saturating_sub(2);
    let min_y = area.y + 1;
    let max_y = area.bottom().saturating_sub(2);
    let x = (area.x + 1 + cursor.0).clamp(min_x, max_x);
    let y = (area.y
        + 1
        + comment_rows
        + 1
        + cursor.1.saturating_sub(window.0).min(window.1 - 1) as u16)
        .clamp(min_y, max_y);
    frame.set_cursor_position((x, y));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_rejection_and_comment_prompts_name_their_target_and_purpose() {
        let target = "unblock:retry_with_guidance:plan-7:task-9";
        assert!(editor_title(target).contains("plan plan-7 · task task-9"));
        assert_eq!(editor_prompt(target), "Hint for this worker (optional):");
        assert!(editor_title("reject:plan-7").contains("reject plan plan-7"));
        assert_eq!(
            editor_prompt("reject:plan-7"),
            "Reason for rejecting plan plan-7 (optional):"
        );
        assert_eq!(
            editor_prompt("plan:plan-7:line:12"),
            "Comment on Plan plan-7 line 12:"
        );
    }
}

use super::super::super::*;

pub(super) fn draw_confirmation(app: &App, frame: &mut Frame) {
    let Some(conf) = app.confirmation.as_ref().filter(|c| {
        c.status == ConfirmationStatus::Pending
            && app.picker.is_none()
            && app.help.is_none()
            && app.trust_prompt.is_none()
    }) else {
        return;
    };
    let area = frame.area();
    let height = 5;
    if area.height < height + 2 || area.width < 20 {
        return;
    }
    let rect = Rect::new(
        area.x + 2,
        area.bottom().saturating_sub(height + 2),
        area.width.saturating_sub(4),
        height,
    );
    frame.render_widget(Clear, rect);
    frame.render_widget(confirmation_box(conf), rect);
}

fn confirmation_box(conf: &ConfirmationCard) -> Paragraph<'static> {
    let lines = vec![
        Line::from(vec![
            Span::styled(
                " Confirmation: ",
                Style::default()
                    .fg(theme::ayu::YELLOW)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(conf.title.clone()),
        ]),
        Line::from(vec![Span::styled(
            format!(" Scope: {}", conf.scope_hash),
            Style::default().fg(theme::ayu::DIM),
        )]),
        Line::from(vec![Span::styled(
            " [Enter / y] Confirm   [Esc / n] Reject",
            Style::default()
                .fg(theme::ayu::BLUE)
                .add_modifier(Modifier::BOLD),
        )]),
    ];
    Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::ayu::YELLOW))
            .title(" Confirmation required "),
    )
}

pub(super) fn draw_project_trust(app: &App, frame: &mut Frame) {
    let Some(path) = &app.project_trust else {
        return;
    };
    let area = frame.area();
    let rect = centered_dialog(area, 8, 76);
    frame.render_widget(Clear, rect);
    frame.render_widget(project_trust_box(path, rect.width), rect);
}

fn project_trust_box(path: &std::path::Path, width: u16) -> Paragraph<'static> {
    Paragraph::new(project_trust_lines(path, width)).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::ayu::YELLOW))
            .title(" trust ")
            .padding(ratatui::widgets::Padding::horizontal(1)),
    )
}

fn project_trust_lines(path: &std::path::Path, width: u16) -> Vec<Line<'static>> {
    let inner = usize::from(width.saturating_sub(4).max(1));
    vec![
        project_question(inner),
        Line::raw(""),
        project_path(path, inner),
        Line::raw(""),
        project_access(inner),
        Line::raw(""),
        project_actions(inner),
    ]
}

fn project_question(width: usize) -> Line<'static> {
    Line::styled(
        truncate("Open this directory as a project?", width),
        Style::default()
            .fg(theme::ayu::YELLOW)
            .add_modifier(Modifier::BOLD),
    )
}

fn project_path(path: &std::path::Path, width: usize) -> Line<'static> {
    Line::styled(
        truncate(&path.display().to_string(), width),
        Style::default()
            .fg(theme::ayu::BLUE)
            .add_modifier(Modifier::BOLD),
    )
}

fn project_access(width: usize) -> Line<'static> {
    Line::styled(
        truncate(
            "Its agents can read files, run commands and edit code there.",
            width,
        ),
        dim(),
    )
}

fn project_actions(width: usize) -> Line<'static> {
    Line::styled(
        truncate("[Enter / y] Trust and open   [Esc / n] Cancel", width),
        Style::default().fg(theme::ayu::GREEN),
    )
}

pub(super) fn draw_unblock(app: &mut App, frame: &mut Frame) {
    if let Some(modal) = &mut app.unblock {
        modal.draw(frame);
    }
}

pub(super) fn draw_graph_filters(app: &mut App, frame: &mut Frame) {
    let providers = graph::providers(&app.view);
    if let Some(menu) = &mut app.graph_filters {
        menu.draw(frame, &providers, app.relations, &app.graph_scope);
    }
}

pub(super) fn draw_trust_prompt(app: &App, frame: &mut Frame) {
    let Some(dir) = &app.trust_prompt else {
        return;
    };
    let area = frame.area();
    let rect = centered_dialog(area, 9, 76);
    frame.render_widget(Clear, rect);
    frame.render_widget(trust_prompt_box(dir, rect.width), rect);
}

fn trust_prompt_box(dir: &str, width: u16) -> Paragraph<'static> {
    Paragraph::new(trust_prompt_lines(dir, width)).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::ayu::YELLOW))
            .title(" Workspace trust required "),
    )
}

fn trust_prompt_lines(dir: &str, width: u16) -> Vec<Line<'static>> {
    let inner = usize::from(width.saturating_sub(4).max(1));
    vec![
        trust_question(inner),
        Line::raw(""),
        trust_directory(dir, inner),
        Line::raw(""),
        trust_access(inner),
        Line::raw(""),
        trust_actions(inner),
    ]
}

fn trust_question(width: usize) -> Line<'static> {
    trust_line(
        "Do you trust the contents of this directory?",
        width,
        theme::ayu::YELLOW,
        true,
    )
}

fn trust_directory(dir: &str, width: usize) -> Line<'static> {
    trust_line(
        &format!("  {}", truncate(dir, width.saturating_sub(2))),
        width,
        theme::ayu::BLUE,
        true,
    )
}

fn trust_access(width: usize) -> Line<'static> {
    Line::styled(
        truncate(
            "Agents can read files, run commands, and edit code in this folder.",
            width,
        ),
        dim(),
    )
}

fn trust_actions(width: usize) -> Line<'static> {
    trust_line(
        "[Enter / y] Trust this directory   [Esc / n / q] Quit",
        width,
        theme::ayu::GREEN,
        true,
    )
}

fn trust_line(text: &str, width: usize, color: Color, bold: bool) -> Line<'static> {
    let style = Style::default().fg(color);
    let style = if bold {
        style.add_modifier(Modifier::BOLD)
    } else {
        style
    };
    Line::from(vec![Span::styled(truncate(text, width), style)])
}

fn centered_dialog(area: Rect, height: u16, max_width: u16) -> Rect {
    let width = area.width.saturating_sub(4).clamp(12, max_width);
    let height = height.min(area.height);
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

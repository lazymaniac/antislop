//! Shared spacing and flat surfaces from the approved Canvas/Workbench references.
use super::super::*;
use unicode_width::UnicodeWidthStr;

pub(super) fn inset(area: Rect) -> Rect {
    area.inner(ratatui::layout::Margin::new(
        if area.width >= 100 {
            2
        } else {
            u16::from(area.width >= 40)
        },
        u16::from(area.height >= 30),
    ))
}

pub(super) fn brand(frame: &mut Frame, area: Rect, project: &str, status: &str, theme: &Theme) {
    if area.is_empty() {
        return;
    }
    let project = std::path::Path::new(project)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(project);
    let spans = vec![
        Span::styled(
            format!("{} ", if theme.ascii { ">" } else { "›" }),
            theme.accent(),
        ),
        Span::styled("antislop", theme.text()),
        Span::styled(
            format!("    {} / session", theme.clean(project)),
            theme.dim(),
        ),
    ];
    frame.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect::new(area.x, area.y, area.width, 1),
    );
    if area.width >= 80 {
        let width = (status.width() as u16).min(area.width / 3);
        frame.render_widget(
            Paragraph::new(theme.fit(status, usize::from(width))).style(theme.accent()),
            Rect::new(area.right() - width, area.y, width, 1),
        );
    }
}

pub(super) fn direction(frame: &mut Frame, area: Rect, target: &str, theme: &Theme) {
    if area.is_empty() {
        return;
    }
    frame.render_widget(Block::default().style(theme.card()), area);
    let y = area.y + u16::from(area.height >= 3);
    let padding = u16::from(area.width >= 20) * 2;
    let width = area.width.saturating_sub(padding * 2);
    let label = if target == "session" {
        "Give direction, change scope, ask a question…"
    } else {
        "Give direction to this task…"
    };
    let line = Line::from(vec![
        Span::styled(":  ", theme.accent()),
        Span::styled(
            theme.fit(&theme.chrome(label), usize::from(width.saturating_sub(12))),
            theme.dim(),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(line),
        Rect::new(area.x + padding, y, width, 1),
    );
    if width >= 24 {
        frame.render_widget(
            Paragraph::new("ctrl k").style(theme.text()),
            Rect::new(area.right().saturating_sub(padding + 6), y, 6, 1),
        );
    }
}

pub(super) fn keybar(
    app: &mut App,
    frame: &mut Frame,
    area: Rect,
    keys: &[(&str, KeyCode, KeyModifiers)],
    theme: &Theme,
) {
    let mut x = area.x;
    for (label, code, modifiers) in keys {
        let label = theme.chrome(label);
        let width = label.width() as u16;
        if x.saturating_add(width) > area.right() {
            break;
        }
        let hit = Rect::new(x, area.y, width, 1);
        frame.render_widget(Paragraph::new(label).style(theme.dim()), hit);
        app.chrome_keys.push((hit, *code, *modifiers));
        x = x.saturating_add(width + 4);
    }
}

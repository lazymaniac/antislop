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

pub(super) fn brand(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) -> Option<Rect> {
    if area.is_empty() {
        return None;
    }
    let branch = app
        .view
        .session_branch
        .as_deref()
        .or_else(|| {
            app.projects
                .get(app.active_project)
                .and_then(|project| project.branch.as_deref())
        })
        .map(|branch| theme.clean(branch))
        .unwrap_or_else(|| theme.chrome("—"));
    let state = if app.view.paused {
        "paused"
    } else if app.view.idle {
        "ready"
    } else {
        "running"
    };
    let status = theme.chrome(&format!(
        "{state} · {}",
        super::super::flow_editor::format_clock_mm_ss(
            app.view
                .session_time
                .elapsed_ms(crate::lean::store::now_ms())
        )
    ));
    let width = status.width() as u16;
    let right = (area.width >= width.saturating_add(13))
        .then(|| Rect::new(area.right() - width, area.y, width, 1));
    let left_width = right.map_or(area.width, |right| right.x.saturating_sub(area.x + 2));
    let spans = vec![
        Span::styled("> ", theme.accent()),
        Span::styled("antislop", theme.text()),
        Span::styled(
            format!(
                "  {}",
                theme.fit(&branch, usize::from(left_width.saturating_sub(12)))
            ),
            theme.dim(),
        ),
    ];
    frame.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect::new(area.x, area.y, left_width, 1),
    );
    if let Some(right) = right {
        frame.render_widget(Paragraph::new(status).style(theme.accent()), right);
    }
    right
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

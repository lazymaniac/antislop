use super::super::*;

pub(super) fn draw(app: &mut App, frame: &mut Frame, theme: &Theme) {
    if app.editor.is_some() || app.flow_editor.is_some() || app.plan_view.is_some() {
        return;
    }
    let agent = app.details.as_ref().is_some_and(|details| details.editing);
    if app.navigation.focus != NavigationFocus::Composer && !agent {
        return;
    }
    let screen = frame.area();
    let width = if screen.width <= 40 {
        screen.width
    } else {
        screen.width.saturating_sub(4).min(84)
    }
    .max(1);
    let flat = screen.width >= 60 && screen.height >= 20;
    let height = if flat {
        18.min(screen.height)
    } else if screen.height <= 15 {
        screen.height
    } else {
        13
    }
    .max(1);
    let area = Rect::new(
        screen.x + (screen.width - width) / 2,
        screen.y
            + if flat && screen.height >= 30 {
                screen.height / 6
            } else {
                (screen.height - height) / 2
            },
        width,
        height,
    );
    frame.render_widget(Clear, area);
    if flat {
        for cell in &mut frame.buffer_mut().content {
            cell.modifier.insert(Modifier::DIM);
        }
    }
    let inner = if flat {
        Rect::new(
            area.x + 2,
            area.y + 5,
            area.width.saturating_sub(4),
            area.height.saturating_sub(9),
        )
    } else {
        super::composer::panel_editor_area(area)
    };
    let wrap_width = if flat {
        usize::from(inner.width.saturating_sub(1)).max(1)
    } else {
        super::composer::panel_wrap_width(area)
    };
    let (input, recipient, mode, status) = if agent {
        let details = app.details.as_mut().unwrap();
        details.input_width[details.active_panel] = wrap_width;
        let key = details.composer_key();
        let drafts = details.drafts.borrow();
        let composer = drafts.get(&key).cloned().unwrap_or_default();
        (
            composer.input,
            composer.address.unwrap_or(key.1),
            composer.mode,
            composer.error.or(composer.delivery).unwrap_or_default(),
        )
    } else {
        app.input_width = wrap_width;
        let recipient = app
            .input
            .text
            .strip_prefix('@')
            .and_then(|rest| {
                rest.split_once(char::is_whitespace)
                    .map(|(recipient, _)| recipient)
            })
            .unwrap_or(ORCHESTRATOR)
            .to_owned();
        let attachments = app.image_drafts.get(&app.conversation).map_or(0, Vec::len);
        (
            app.input.clone(),
            recipient,
            app.navigation.session_delivery,
            app.notice
                .clone()
                .unwrap_or_else(|| format!("{attachments} images · paste/drop to attach")),
        )
    };
    let lines = visual_text_lines(&input.text, wrap_width);
    let (column, row) = input.visual_cursor(wrap_width);
    let first = row.saturating_sub(usize::from(inner.height.saturating_sub(1)));
    if !agent {
        app.input_shown = first;
    }
    let lines: Vec<_> = lines
        .into_iter()
        .skip(first)
        .take(usize::from(inner.height))
        .collect();
    let title = theme.chrome(&format!(
        " Give direction · To: {recipient} · {} · Ctrl+R recipient · Ctrl+T mode ",
        mode.name()
    ));
    let hint = theme.chrome(&format!(
        " Enter send · Alt+Enter/Ctrl+J newline · Esc retain draft · {status}"
    ));
    if flat {
        frame.render_widget(Clear, area);
        frame.render_widget(Block::default().style(theme.raised()), area);
        frame.render_widget(
            Paragraph::new("Give direction").style(theme.text()),
            Rect::new(area.x + 2, area.y + 1, area.width.saturating_sub(4), 1),
        );
        let target = if agent { "this task" } else { "this session" };
        frame.render_widget(
            Paragraph::new(theme.chrome(&format!(
                "To: {recipient} · {target} · {} · Ctrl+R recipient · Ctrl+T mode",
                mode.name()
            )))
            .style(theme.dim()),
            Rect::new(area.x + 2, area.y + 3, area.width.saturating_sub(4), 1),
        );
        frame.render_widget(Block::default().style(theme.overlay()), inner);
        frame.render_widget(
            Paragraph::new(lines.into_iter().map(Line::raw).collect::<Vec<_>>())
                .style(theme.text()),
            inner,
        );
        let send = Rect::new(area.x + 2, area.bottom() - 3, 22, 1);
        let cancel = Rect::new(send.right() + 3, send.y, 10, 1);
        frame.render_widget(
            Paragraph::new(theme.chrome(" Send instruction ↵ ")).style(theme.button()),
            send,
        );
        frame.render_widget(Paragraph::new("Cancel").style(theme.dim()), cancel);
        let hint = if status.is_empty() {
            "Alt+Enter/Ctrl+J newline · Esc retain draft".to_owned()
        } else {
            status
        };
        frame.render_widget(
            Paragraph::new(theme.chrome(&hint)).style(theme.dim()),
            Rect::new(
                area.x + 2,
                area.bottom() - 1,
                area.width.saturating_sub(4),
                1,
            ),
        );
        if app.picker.is_none() && app.help.is_none() {
            app.direction_actions = Some((send, cancel));
            frame.set_cursor_position((inner.x + column, inner.y + (row - first) as u16));
        }
    } else {
        super::composer::draw_panel(
            frame,
            area,
            super::composer::Panel {
                title: &title,
                status: &hint,
                lines: &lines,
                focused: app.picker.is_none() && app.help.is_none(),
                cursor: Some((column, row, first)),
            },
            theme,
        );
    }
}

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
    let height = if screen.height <= 15 {
        screen.height
    } else {
        13
    }
    .max(1);
    let area = Rect::new(
        screen.x + (screen.width - width) / 2,
        screen.y + (screen.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, area);
    let inner = super::composer::panel_editor_area(area);
    let wrap_width = super::composer::panel_wrap_width(area);
    let (input, recipient, mode, status) = if agent {
        let details = app.details.as_mut().unwrap();
        details.input_width[details.active_panel] = wrap_width;
        let key = details.composer_key();
        let drafts = details.drafts.borrow();
        let composer = drafts.get(&key).cloned().unwrap_or_default();
        (
            composer.input,
            key.1,
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

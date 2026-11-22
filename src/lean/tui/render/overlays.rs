use super::super::*;

mod editor;
mod modal;

pub(super) fn draw_remaining(app: &mut App, frame: &mut Frame, theme: &Theme) {
    draw_selection(app, frame);
    if editor::draw_editor(app, frame, theme) {
        return;
    }
    if let Some(flow_editor) = &app.flow_editor {
        flow_editor.draw(frame, theme);
        return;
    }
    super::direction::draw(app, frame, theme);
    draw_interview(app, frame, theme);
    draw_picker(app, frame, theme);
    draw_help(app, frame);
    modal::draw_unblock(app, frame);
    modal::draw_graph_filters(app, frame);
    app.draw_decision_inbox(frame, theme);
}

fn draw_selection(app: &mut App, frame: &mut Frame) {
    let area = (!app.selection_blocked())
        .then_some(app.text_area)
        .flatten();
    app.text_selection.draw_scrolled(
        frame.buffer_mut(),
        area,
        app.selection_context(),
        app.text_origin,
    );
}

fn draw_interview(app: &mut App, frame: &mut Frame, theme: &Theme) {
    if !app.direction_area.is_empty() {
        return;
    }
    if app.editor.is_some()
        || app.help.is_some()
        || app.picker.is_some()
        || !app.interview_visible()
    {
        return;
    }
    let area = if app.view.scope_confirmation_summary.is_some() {
        let screen = frame.area();
        let horizontal = (screen.width / 20)
            .max(1)
            .min(screen.width.saturating_sub(view::MIN_TERM_WIDTH) / 2);
        let vertical = (screen.height / 20)
            .max(1)
            .min(screen.height.saturating_sub(view::MIN_TERM_HEIGHT) / 2);
        screen.inner(ratatui::layout::Margin::new(horizontal, vertical))
    } else {
        app.chat_area
    };
    if let Some(form) = app.interview_forms.get_mut(&app.conversation) {
        form.draw(frame, area, theme);
    }
}

fn draw_picker(app: &mut App, frame: &mut Frame, theme: &Theme) {
    if let Some(picker) = &mut app.picker {
        picker.draw_themed(frame, &app.view, app.help.is_none(), theme);
    }
}

fn draw_help(app: &mut App, frame: &mut Frame) {
    if let Some(help) = app.help {
        app.help_scroll = app.help_scroll.min(help.max_scroll(frame.area()));
        help.draw_scroll(frame, app.help_scroll);
    }
}

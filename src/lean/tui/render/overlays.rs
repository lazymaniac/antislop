use super::super::*;

mod editor;
mod modal;

pub(super) fn draw_remaining(app: &mut App, frame: &mut Frame) {
    draw_selection(app, frame);
    if editor::draw_editor(app, frame) {
        return;
    }
    draw_interview(app, frame);
    draw_picker(app, frame);
    draw_help(app, frame);
    modal::draw_confirmation(app, frame);
    modal::draw_project_trust(app, frame);
    modal::draw_unblock(app, frame);
    modal::draw_graph_filters(app, frame);
    modal::draw_trust_prompt(app, frame);
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

fn draw_interview(app: &mut App, frame: &mut Frame) {
    if app.editor.is_some()
        || app.help.is_some()
        || app.picker.is_some()
        || app.trust_prompt.is_some()
        || app.project_trust.is_some()
        || app.confirmation.is_some()
        || !app.interview_visible()
    {
        return;
    }
    if let Some(form) = app.interview_forms.get_mut(&app.conversation) {
        form.draw(frame, app.chat_area, &Theme::from_env());
    }
}

fn draw_picker(app: &mut App, frame: &mut Frame) {
    if let Some(picker) = &mut app.picker {
        picker.draw_with_cursor(
            frame,
            &app.view,
            app.help.is_none() && app.trust_prompt.is_none(),
        );
    }
}

fn draw_help(app: &mut App, frame: &mut Frame) {
    if let Some(help) = app.help {
        app.help_scroll = app.help_scroll.min(help.max_scroll(frame.area()));
        help.draw_scroll(frame, app.help_scroll);
    }
}

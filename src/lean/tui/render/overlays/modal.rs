use super::super::super::*;

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

pub(super) fn draw_approval(app: &mut App, frame: &mut Frame) {
    app.sync_approval();
    if let Some(modal) = &app.approval {
        modal.draw(frame);
    }
}

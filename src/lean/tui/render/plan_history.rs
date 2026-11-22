use super::super::*;

pub(super) fn draw_plan_history(app: &mut App, frame: &mut Frame, theme: &Theme) -> bool {
    if app.plan_history.is_none() {
        return false;
    }
    let area = super::chrome::branded_area(app, frame, theme);
    app.plan_history
        .as_mut()
        .unwrap()
        .draw_with_theme(frame, area, theme);
    true
}

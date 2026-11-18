use super::super::*;

pub(super) fn draw_plan_history(app: &mut App, frame: &mut Frame) -> bool {
    if let Some(browser) = app.plan_history.as_mut() {
        browser.draw(frame);
        return true;
    }
    false
}

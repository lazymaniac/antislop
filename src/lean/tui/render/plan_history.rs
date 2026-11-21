use super::super::*;

pub(super) fn draw_plan_history(app: &mut App, frame: &mut Frame, theme: &Theme) -> bool {
    if let Some(browser) = app.plan_history.as_mut() {
        browser.draw_with_theme(frame, theme);
        return true;
    }
    false
}

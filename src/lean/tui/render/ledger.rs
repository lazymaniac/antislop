use super::super::*;

pub(super) fn draw_history_or_ledger(app: &mut App, frame: &mut Frame) -> bool {
    if let Some(browser) = app.plan_history.as_mut() {
        browser.draw(frame);
        return true;
    }
    let Some(ledger) = app.ledger_view.as_mut().filter(|view| view.visible) else {
        return false;
    };
    ledger.workflow_notice = app.view.requirements_progress.clone();
    ledger.draw(frame, frame.area());
    if let Some(picker) = &mut app.picker {
        picker.draw(frame, &app.view);
    }
    true
}

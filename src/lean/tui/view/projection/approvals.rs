use super::*;
use serde_json::Value;

pub(super) fn on_approval_requested(
    view: &mut View,
    _event: &Value,
    payload: &Value,
    _key: String,
    _turn: &str,
) {
    let text = |name: &str| payload[name].as_str().unwrap_or_default().to_owned();
    let options = payload["options"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|option| {
            let field = |name: &str| option[name].as_str().unwrap_or_default().to_owned();
            (field("id"), field("label"), field("decision"))
        })
        .collect();
    let request = text("request");
    view.approvals.retain(|pending| pending.request != request);
    view.approvals.push(PendingApproval {
        request,
        role: text("role"),
        provider: text("provider"),
        summary: text("summary"),
        orchestrator: payload["decider"] == "orchestrator",
        options,
    });
}

pub(super) fn on_approval_resolved(
    view: &mut View,
    _event: &Value,
    payload: &Value,
    _key: String,
    _turn: &str,
) {
    let request = payload["request"].as_str().unwrap_or_default();
    view.approvals.retain(|pending| pending.request != request);
}

pub(super) fn on_approval_escalated(
    view: &mut View,
    _event: &Value,
    payload: &Value,
    _key: String,
    _turn: &str,
) {
    let request = payload["request"].as_str().unwrap_or_default();
    for pending in &mut view.approvals {
        if pending.request == request {
            pending.orchestrator = false;
        }
    }
}

pub(super) fn on_approval_mode_changed(
    view: &mut View,
    _event: &Value,
    payload: &Value,
    _key: String,
    _turn: &str,
) {
    view.autonomous = payload["mode"] == "autonomous";
}

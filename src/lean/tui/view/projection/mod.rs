use super::*;
use serde_json::Value;
mod agents;
mod conversation;
mod graph;
mod mail;
mod plan;
mod reviews;
mod tasks;

pub(super) fn apply(view: &mut View, event: &Value) {
    view.layout_revision = view.layout_revision.wrapping_add(1);
    view.after = event["seq"].as_u64().unwrap_or(view.after);
    let payload = &event["payload"];
    let turn = event["turn_id"].as_str().unwrap_or_default();
    let key = view.roles.get(turn).cloned().unwrap_or_default();
    let kind = event["kind"].as_str().unwrap_or_default();
    if !event_is_visible(kind, payload, &key) {
        return;
    }
    let handler = if kind == "message" && payload["internal"] == true {
        Some(conversation::on_message as Handler)
    } else if kind == "message" {
        Some(conversation::on_message_2 as Handler)
    } else {
        HANDLERS
            .iter()
            .find(|(name, _)| *name == kind)
            .map(|(_, handler)| *handler)
    };
    if let Some(handler) = handler {
        handler(view, event, payload, key, turn);
    }
}

fn event_is_visible(kind: &str, payload: &Value, key: &str) -> bool {
    (kind != "agent_final" || matches!(key, crate::lean::PLANNER | ORCHESTRATOR))
        && (kind != "mail" || payload["kind"] != "copy")
}

type Handler = fn(&mut View, &Value, &Value, String, &str);

const HANDLERS: &[(&str, Handler)] = &[
    (
        "ledger_approval_required",
        conversation::on_ledger_approval_required,
    ),
    ("ledger_changed", conversation::on_ledger_changed),
    ("ledger_proposal_rejected", conversation::on_ledger_changed),
    (
        "user_decision_requested",
        conversation::on_user_decision_requested,
    ),
    ("interview_question", conversation::on_interview_question),
    ("session_started", conversation::on_session_started),
    ("agent_final", agents::on_agent_final),
    ("planning_progress", conversation::on_planning_progress),
    ("workflow", conversation::on_workflow),
    ("turn_started", agents::on_turn_started),
    ("agent_tool_rejected", agents::on_agent_tool_rejected),
    ("turn_finished", agents::on_turn_finished),
    ("graph_task", graph_task),
    ("graph_updated", graph_updated),
    ("plan_proposed", plan::on_plan_proposed),
    ("plan_rejected", plan::on_plan_rejected),
    ("plan_review", plan::on_plan_review),
    ("requirements_failed", reviews::on_requirements_failed),
    ("plan_invalid", plan::on_plan_invalid),
    ("task", task),
    ("comment_replied", mail::on_comment_replied),
    ("paused", reviews::on_paused),
    ("notice", reviews::on_notice),
    ("mode", reviews::on_mode),
    ("check", tasks::on_check),
    ("integration", integration),
    ("task_recovery", task_recovery),
    ("mail", mail_message),
    ("mail_delivered", mail::on_mail_delivered),
    ("mail_rejected", mail_rejected),
    ("audit", reviews::on_audit),
    ("code_review", reviews::on_code_review),
    ("comment", comment),
];

fn graph_task(view: &mut View, _: &Value, payload: &Value, _: String, _: &str) {
    graph::on_graph_task(view, payload);
}
fn graph_updated(view: &mut View, _: &Value, payload: &Value, _: String, _: &str) {
    graph::on_graph_updated(view, payload);
}
fn task(view: &mut View, _: &Value, payload: &Value, _: String, _: &str) {
    tasks::on_task(view, payload);
}
fn integration(view: &mut View, _: &Value, payload: &Value, _: String, _: &str) {
    tasks::on_integration(view, payload);
}
fn task_recovery(view: &mut View, _: &Value, payload: &Value, _: String, _: &str) {
    tasks::on_task_recovery(view, payload);
}
fn mail_message(view: &mut View, _: &Value, payload: &Value, _: String, _: &str) {
    mail::on_mail(view, payload);
}
fn mail_rejected(view: &mut View, _: &Value, payload: &Value, _: String, _: &str) {
    mail::on_mail_rejected(view, payload);
}
fn comment(view: &mut View, _: &Value, payload: &Value, _: String, _: &str) {
    mail::on_comment(view, payload);
}

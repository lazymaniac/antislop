use super::*;
use serde_json::Value;

pub(super) fn on_task(view: &mut View, payload: &Value) {
    let id = clean_line(payload["id"].as_str().unwrap_or_default());
    let index = view.tasks.iter().position(|row| row.id == id);
    let Some(index) = index else {
        return;
    };
    let task_uuid = payload["task"].as_str().unwrap_or_default().to_owned();
    view.check_events += 1;
    view.task_check_events.insert(id.clone(), view.check_events);
    view.task_rows.insert(task_uuid.clone(), index);
    let progress = update_task_row(view, index, payload, &task_uuid);
    report_task_reason(view, &id, &progress);
    view.report_progress(
        &id,
        &progress.title,
        (&progress.previous_status, progress.previous_round),
        (&progress.status, progress.round),
        progress.reason,
    );
}

struct TaskProgress {
    title: String,
    previous_status: String,
    previous_round: u64,
    status: String,
    round: u64,
    reason: Option<String>,
}

fn update_task_row(
    view: &mut View,
    index: usize,
    payload: &Value,
    task_uuid: &str,
) -> TaskProgress {
    let row = &mut view.tasks[index];
    let (previous_status, previous_round) = update_task_identity(row, payload, task_uuid);
    update_task_feedback(row, &payload["feedback"]);
    if let Some(scope) = payload["scope_hash"].as_str() {
        row.scope_hash = Some(scope.to_owned());
        view.conversation_scope_hash = Some(scope.to_owned());
    }
    clear_finished_activity(row);
    TaskProgress {
        title: row.title.clone(),
        previous_status,
        previous_round,
        status: row.status.clone(),
        round: row.round,
        reason: row.reason.clone(),
    }
}

fn update_task_identity(row: &mut TaskRow, payload: &Value, task_uuid: &str) -> (String, u64) {
    let previous = (row.status.clone(), row.round);
    row.uuid = Some(task_uuid.to_owned());
    row.status = clean_line(payload["status"].as_str().unwrap_or_default());
    row.round = payload["round"].as_u64().unwrap_or_default();
    row.reason = payload["reason"]
        .as_str()
        .map(|reason| bound(&clean(reason), DETAIL_LIMIT));
    previous
}

fn clear_finished_activity(row: &mut TaskRow) {
    if status::finished(&row.status) {
        row.activity.clear();
    }
}

fn update_task_feedback(row: &mut TaskRow, feedback: &Value) {
    let fresh = feedback_is_current(feedback, row.round);
    let projected = if fresh {
        feedback["effect"]
            .as_str()
            .map(|effect| crate::lean::tasks::bound_cut(&clean_line(effect), EFFECT_LIMIT))
    } else {
        None
    };
    row.effect = projected.as_ref().map(|(text, _)| text.clone());
    row.effect_cut = projected.map(|(_, cut)| cut || feedback["effect_cut"] == true);
    row.obligations = if fresh && feedback["coverage"].is_object() {
        Some(bounded_coverage(&feedback["coverage"]))
    } else {
        None
    };
    row.integration = if fresh && feedback["integration"].is_object() {
        Some(bounded_landing(&feedback["integration"]))
    } else {
        None
    };
}

fn report_task_reason(view: &mut View, id: &str, progress: &TaskProgress) {
    if let Some(reason) = &progress.reason {
        let line = format!(
            "! {} {}: {} — {reason}",
            id,
            progress.title,
            status_label(&progress.status)
        );
        view.push(
            Style::default().fg(crate::lean::tui::theme::ayu::YELLOW),
            line,
        );
    }
}

pub(super) fn on_check(view: &mut View, event: &Value, payload: &Value, key: String, turn: &str) {
    let text = |name: &str| payload[name].as_str().unwrap_or_default().to_owned();
    let _ = (event, &key, turn, &text);
    view.check_events += 1;
    if let Some(row) = view
        .task_rows
        .get(&text("task"))
        .and_then(|&index| view.tasks.get(index))
    {
        view.task_check_events
            .insert(row.id.clone(), view.check_events);
    }
}

pub(super) fn on_integration(view: &mut View, payload: &Value) {
    let approved = payload["status"] == "approved";
    let title = clean_line(&format!(
        "[{}] {}",
        event_text(payload, "id"),
        event_text(payload, "title")
    ));
    // The landing line the row keeps is the bounded projection;
    // the full text lives in the task's report and in the mail,
    // so a cut here never loses evidence. Events written before
    // the daemon bounded details fold through the same cut.
    let detail = bound(&clean_line(&event_text(payload, "detail")), DETAIL_LIMIT);
    report_integration(view, &title, &detail, approved);
    update_integration_row(view, payload, &detail);
}

fn report_integration(view: &mut View, title: &str, detail: &str, approved: bool) {
    view.cards.push(TranscriptCard::System {
        title: title.to_owned(),
        meta: if approved {
            "approved".into()
        } else {
            "verification".into()
        },
        text: detail.to_owned(),
        tone: if approved {
            crate::lean::tui::theme::Tone::Done
        } else {
            crate::lean::tui::theme::Tone::Waiting
        },
    });
    view.push(
        if approved {
            dim()
        } else {
            Style::default().fg(crate::lean::tui::theme::ayu::YELLOW)
        },
        format!("· {title}: {detail}"),
    );
}

fn update_integration_row(view: &mut View, payload: &Value, detail: &str) {
    // The same landing reaches the row: a stopped integration
    // keeps its raw reason, and the detail rides along with any
    // outcome the task event reported.
    let id = clean_line(&event_text(payload, "id"));
    if let Some(row) = view.tasks.iter_mut().find(|row| row.id == id) {
        if payload["status"] == "needs_user" {
            row.reason = Some(detail.to_owned());
        }
        match row.integration.as_mut().filter(|v| v.is_object()) {
            Some(existing) => {
                existing["detail"] = serde_json::json!(detail);
                existing["source"] = serde_json::json!("integration");
            }
            None => {
                row.integration = Some(serde_json::json!({
                    "detail": detail,
                    "source": "integration",
                    "status": event_text(payload, "status"),
                }));
            }
        }
    }
}

pub(super) fn on_task_recovery(view: &mut View, payload: &Value) {
    // A recovery is announced as a `task` event with its feedback
    // plus this event with the action's own raw detail. The row
    // takes the bounded line; a retry also clears what the earlier
    // round showed, so nothing stale survives the new round.
    // Events written before the daemon bounded recovery details
    // fold through the same cut; the whole text stays in mail.
    let action = clean_line(&event_text(payload, "action"));
    let detail = bound(&clean_line(&event_text(payload, "detail")), DETAIL_LIMIT);
    let line = format!("· recovery · {action} · {detail}");
    view.push(dim(), line);
    let task = event_text(payload, "task");
    if let Some(&index) = view.task_rows.get(&task)
        && let Some(row) = view.tasks.get_mut(index)
    {
        update_recovery_row(row, &action, &detail, payload);
    }
}

fn update_recovery_row(row: &mut TaskRow, action: &str, detail: &str, payload: &Value) {
    let status = clean_line(&event_text(payload, "status"));
    if !status.is_empty() {
        row.status = status;
    }
    if status::finished(&row.status) {
        row.activity.clear();
    }
    if !detail.is_empty() {
        row.reason = Some(detail.to_owned());
    }
    if resets_card(action) {
        row.effect = None;
        row.effect_cut = None;
        row.obligations = None;
        row.integration = None;
    }
    apply_recovery_outcome(row, action, payload);
}

fn apply_recovery_outcome(row: &mut TaskRow, action: &str, payload: &Value) {
    let Some(outcome) = recovery_outcome(action) else {
        return;
    };
    let mut integration = row
        .integration
        .take()
        .filter(|v| v.is_object())
        .unwrap_or_else(|| serde_json::json!({}));
    integration["outcome"] = serde_json::json!(outcome);
    integration["source"] = serde_json::json!("recovery");
    if let Some(branch) = payload["branch"].as_str() {
        integration["branch"] = serde_json::json!(bound(&clean_line(branch), DETAIL_LIMIT));
    }
    row.integration = Some(integration);
}

fn event_text(payload: &Value, field: &str) -> String {
    payload[field].as_str().unwrap_or_default().to_owned()
}

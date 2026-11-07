use super::*;
use serde_json::Value;

pub(super) fn on_plan_proposed(
    view: &mut View,
    _event: &Value,
    payload: &Value,
    _key: String,
    _turn: &str,
) {
    view.plan_preview = None;
    view.requirements_progress = Some(if payload["automatic"] == true {
        (false, "Reviewed plan starts automatically.".into())
    } else if payload["review"]["approved"] == true {
        (false, "Plan reviewed; waiting for your acceptance.".into())
    } else if let Some(error) = payload["review"]["error"].as_str() {
        (true, format!("Plan review failed: {error}"))
    } else if let Some(reason) = payload["waiting"].as_str() {
        (true, format!("Plan could not start: {reason}"))
    } else {
        (false, "Plan received.".into())
    });
    view.plan = payload["plan"].as_str().map(str::to_owned);
    view.plan_doc = payload["document"].as_str().unwrap_or_default().to_owned();
    view.tasks = proposed_tasks(payload);
    fill_parallel_fallback(view, payload);
    reset_plan_agents(view);
    append_plan_review(view, payload);
    publish_plan_summary(view, payload);
}

fn proposed_tasks(payload: &Value) -> Vec<TaskRow> {
    payload["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|task| {
            let id = task["id"].as_str().unwrap_or_default();
            proposed_task(task, &payload["parallel"][id])
        })
        .collect()
}

fn proposed_task(task: &Value, peers: &Value) -> TaskRow {
    let line = |key| clean_line(task[key].as_str().unwrap_or_default());
    let optional = |key| task[key].as_str().map(clean_line);
    TaskRow {
        id: line("id"),
        uuid: None,
        title: line("title"),
        summary: optional("summary"),
        reviewer_summary: task["reviewer"]["summary"].as_str().map(clean_line),
        section: optional("section"),
        kind: line("kind"),
        goal: line("goal"),
        depends_on: strings(&task["depends_on"]),
        parallel_with: strings(peers),
        change_kind: serde_json::from_value(task["change_kind"].clone())
            .expect("plan task has a validated change_kind"),
        focus: clean_line(task["reviewer"]["focus"].as_str().unwrap_or_default()),
        risks: strings(&task["reviewer"]["risks"]),
        status: "proposed".into(),
        round: 0,
        activity: proposed_activity(task["change_kind"] != "none"),
        reason: None,
        revision_token: optional("revision_token"),
        effect: None,
        effect_cut: None,
        obligations: None,
        integration: None,
    }
}

fn proposed_activity(writes: bool) -> String {
    if writes {
        "modifies files".into()
    } else {
        String::new()
    }
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item.as_str().map(clean_line))
        .collect()
}

fn fill_parallel_fallback(view: &mut View, payload: &Value) {
    if !payload["parallel"].is_null() || view.tasks.len() < 2 {
        return;
    }
    let peers = crate::lean::tasks::peers_of(
        &view
            .tasks
            .iter()
            .map(|row| crate::lean::tasks::PeerNode {
                id: &row.id,
                depends_on: &row.depends_on,
            })
            .collect::<Vec<_>>(),
    );
    for row in &mut view.tasks {
        row.parallel_with = peers.get(&row.id).cloned().unwrap_or_default();
    }
}

fn reset_plan_agents(view: &mut View) {
    view.task_rows.clear();
    view.task_check_events.clear();
    view.agents
        .retain(|key, _| !key.starts_with("worker:") && !key.starts_with("reviewer:"));
    view.links.clear();
    view.mailbox.clear();
}

fn append_plan_review(view: &mut View, payload: &Value) {
    for line in crate::lean::cli::review_lines("plan reviewer", &payload["review"]) {
        view.push(
            Style::default().fg(crate::lean::tui::theme::ayu::YELLOW),
            line,
        );
    }
    let review = &payload["review"];
    if !review.is_null() {
        view.review(PLAN_REVIEWER, review);
    }
}

fn publish_plan_summary(view: &mut View, payload: &Value) {
    view.plan_revision_token = payload["revision_token"].as_str().map(str::to_owned);
    if let Some(scope) = &view.plan_revision_token {
        view.conversation_revision_token = Some(scope.clone());
    }
    let next = view
        .requirements_progress
        .as_ref()
        .map(|(_, text)| text.as_str())
        .unwrap_or_default();
    let summary = format!("Plan: {} tasks. {next}", view.tasks.len());
    view.cards.push(TranscriptCard::Plan {
        plan: view.plan.clone().unwrap_or_default(),
        tasks: view.tasks.len(),
        text: summary.clone(),
    });
    view.push(
        Style::default().fg(crate::lean::tui::theme::ayu::YELLOW),
        summary,
    );
    view.push(Style::default(), "");
}

pub(super) fn on_plan_rejected(
    view: &mut View,
    event: &Value,
    payload: &Value,
    key: String,
    turn: &str,
) {
    let text = |name: &str| payload[name].as_str().unwrap_or_default().to_owned();
    let _ = (event, &key, turn, &text);
    for task in &mut view.tasks {
        if task.status == "proposed" {
            task.status = "cancelled".into();
        }
    }
    let reason = text("reason");
    view.cards.push(TranscriptCard::System {
        title: "Plan rejected".into(),
        meta: String::new(),
        text: format!("✗ {reason}"),
        tone: crate::lean::tui::theme::Tone::Failed,
    });
    view.push(
        Style::default().fg(crate::lean::tui::theme::ayu::RED),
        format!("✗ plan rejected: {reason}"),
    );
}

pub(super) fn on_plan_review(
    view: &mut View,
    event: &Value,
    payload: &Value,
    key: String,
    turn: &str,
) {
    let text = |name: &str| payload[name].as_str().unwrap_or_default().to_owned();
    let _ = (event, &key, turn, &text);
    view.review(PLAN_REVIEWER, payload);
}

pub(super) fn on_plan_invalid(
    view: &mut View,
    event: &Value,
    payload: &Value,
    key: String,
    turn: &str,
) {
    let text = |name: &str| payload[name].as_str().unwrap_or_default().to_owned();
    let _ = (event, &key, turn, &text);
    let err = text("error");
    if payload["retrying"] == true {
        view.requirements_progress =
            Some((false, "Correcting the plan format automatically.".into()));
        return;
    }
    report_plan_invalid(view, payload, err);
}

fn report_plan_invalid(view: &mut View, payload: &Value, err: String) {
    let err = if payload["retry"].as_str().is_some() {
        format!("{err}\n{}", payload["retry"].as_str().unwrap_or_default())
    } else {
        err
    };
    view.requirements_progress = Some((true, err.clone()));
    view.cards.push(TranscriptCard::System {
        title: "Plan rejected".into(),
        meta: String::new(),
        text: format!("✗ {err}"),
        tone: crate::lean::tui::theme::Tone::Failed,
    });
    view.push(
        Style::default().fg(crate::lean::tui::theme::ayu::RED),
        format!("✗ plan rejected: {err}"),
    );
}

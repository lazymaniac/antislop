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
    view.requirements_progress = Some((
        false,
        "Plan ready for review. Esc closes requirements so you can review the plan.".into(),
    ));
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
        verifier_summary: task["verifier"]["summary"].as_str().map(clean_line),
        section: optional("section"),
        kind: line("kind"),
        goal: line("goal"),
        scope: strings(&task["scope"]),
        depends_on: strings(&task["depends_on"]),
        parallel_with: strings(peers),
        writes: task["writes"] == true,
        focus: clean_line(task["verifier"]["focus"].as_str().unwrap_or_default()),
        risks: strings(&task["verifier"]["risks"]),
        status: "proposed".into(),
        round: 0,
        activity: proposed_activity(task["writes"] == true),
        reason: None,
        scope_hash: optional("scope_hash"),
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
                writes: row.writes,
                scope: &row.scope,
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
        .retain(|key, _| !key.starts_with("worker:") && !key.starts_with("verifier:"));
    view.links.clear();
    view.mailbox.clear();
}

fn append_plan_review(view: &mut View, payload: &Value) {
    for line in crate::lean::cli::review_lines("plan verifier", &payload["review"]) {
        view.push(
            Style::default().fg(crate::lean::tui::theme::ayu::YELLOW),
            line,
        );
    }
    let review = &payload["review"];
    if !review.is_null() {
        view.review(PLAN_VERIFIER, review);
    }
}

fn publish_plan_summary(view: &mut View, payload: &Value) {
    view.plan_scope_hash = payload["scope_hash"].as_str().map(str::to_owned);
    if let Some(scope) = &view.plan_scope_hash {
        view.conversation_scope_hash = Some(scope.clone());
    }
    let next = if payload["automatic"] == true {
        "running (autonomous mode).".to_owned()
    } else {
        let waiting = payload["waiting"]
            .as_str()
            .map(|reason| format!("Waiting for you: {reason}. "))
            .unwrap_or_default();
        format!("{waiting}Ctrl+A runs it, or type what to change.")
    };
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
    view.review(PLAN_VERIFIER, payload);
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

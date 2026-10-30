use super::*;
use serde_json::Value;

pub(super) fn on_agent_final(
    view: &mut View,
    event: &Value,
    payload: &Value,
    key: String,
    turn: &str,
) {
    let text = |name: &str| payload[name].as_str().unwrap_or_default().to_owned();
    let _ = (event, &key, turn, &text);
    if let crate::lean::turn_result::TurnResult::Plan { document, .. } =
        crate::lean::turn_result::TurnResult::decode(&text("text"))
    {
        view.plan_preview = Some(clean(&document));
    }
}

pub(super) fn on_turn_started(
    view: &mut View,
    event: &Value,
    payload: &Value,
    key: String,
    turn: &str,
) {
    let text = |name: &str| payload[name].as_str().unwrap_or_default().to_owned();
    let _ = (event, &key, turn, &text);
    let key = view.agent_key(&text("role"));
    let provider = text("provider");
    start_role_activity(view, &key, &provider, payload, turn);
    record_agent_start(view, &key, &provider, event, payload, turn);
}

fn start_role_activity(view: &mut View, key: &str, provider: &str, payload: &Value, turn: &str) {
    if key == ORCHESTRATOR {
        let comments: Vec<_> = payload["comments"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_u64)
            .collect();
        for id in &comments {
            view.comment_progress.insert(
                *id,
                CommentProgress {
                    state: "addressing".into(),
                    error: None,
                },
            );
        }
        view.comment_turns.insert(turn.to_owned(), comments);
        view.activity = Some(format!("orchestrator ({provider}) is working…"));
        view.advance_messages("sent", "delivered");
    } else if let Some(row) = view.row_of(key) {
        let who = key.split(':').next().unwrap_or_default();
        row.activity = format!("{who} ({provider})");
    }
}

fn record_agent_start(
    view: &mut View,
    key: &str,
    provider: &str,
    event: &Value,
    payload: &Value,
    turn: &str,
) {
    let agent = view.agents.entry(key.to_owned()).or_default();
    agent.branch = payload["branch"].as_str().map(clean_line);
    agent.provider = provider.to_owned();
    agent.model = payload["model"].as_str().map(str::to_owned);
    agent.effort = payload["effort"].as_str().map(str::to_owned);
    agent.running = true;
    agent.started_ms = event["created_ms"].as_i64().filter(|ms| *ms > 0);
    agent.activity = "starting turn".into();
    view.roles.insert(turn.to_owned(), key.to_owned());
}

pub(super) fn on_agent_tool_rejected(
    view: &mut View,
    event: &Value,
    payload: &Value,
    key: String,
    turn: &str,
) {
    let text = |name: &str| payload[name].as_str().unwrap_or_default().to_owned();
    let _ = (event, &key, turn, &text);
    let who = if key.is_empty() { "agent" } else { &key };
    let line = format!("{who}: {} rejected: {}", text("tool"), text("error"));
    if let Some(agent) = view.agents.get_mut(&key) {
        agent.activity = line.clone();
    }
    if let Some(row) = view.row_of(&key) {
        row.activity = line.clone();
    }
    view.push(
        Style::default().fg(crate::lean::tui::theme::ayu::RED),
        line.clone(),
    );
    view.system_card(
        "Agent tool rejected",
        line,
        crate::lean::tui::theme::Tone::Waiting,
    );
}

pub(super) fn on_turn_finished(
    view: &mut View,
    event: &Value,
    payload: &Value,
    key: String,
    turn: &str,
) {
    let text = |name: &str| payload[name].as_str().unwrap_or_default().to_owned();
    let _ = (event, &key, turn, &text);
    let failed = payload["status"] != "done";
    if key == ORCHESTRATOR {
        finish_orchestrator(view, payload, turn, failed);
    } else if key == crate::lean::PLANNER && failed {
        report_planner_failure(view, payload);
    } else if let Some(row) = view.row_of(&key) {
        row.activity = if failed {
            format!("✗ {}", text("error"))
        } else {
            String::new()
        };
    }
    finish_agent_turn(view, &key, event, payload, failed);
}

fn finish_orchestrator(view: &mut View, payload: &Value, turn: &str, failed: bool) {
    view.activity = None;
    if !failed {
        view.advance_messages("delivered", "answered");
    }
    if let Some(comments) = view.comment_turns.remove(turn) {
        finish_comment_turn(view, comments, payload, failed);
    }
    if failed {
        report_orchestrator_failure(view, payload);
    }
}

fn finish_comment_turn(view: &mut View, comments: Vec<u64>, payload: &Value, failed: bool) {
    for id in comments {
        if view
            .comment_progress
            .get(&id)
            .is_some_and(|progress| progress.state == "queued")
        {
            continue;
        }
        view.comment_progress.insert(
            id,
            CommentProgress {
                state: if failed {
                    payload["status"].as_str().unwrap_or_default().to_owned()
                } else {
                    "addressed".into()
                },
                error: payload["error"].as_str().map(str::to_owned),
            },
        );
    }
}

fn report_orchestrator_failure(view: &mut View, payload: &Value) {
    let line = format!(
        "✗ orchestrator: {} — {}",
        payload["status"].as_str().unwrap_or_default(),
        payload["error"].as_str().unwrap_or_default()
    );
    view.push(
        Style::default().fg(crate::lean::tui::theme::ayu::RED),
        line.clone(),
    );
    view.system_card("Orchestrator", line, crate::lean::tui::theme::Tone::Failed);
}

fn report_planner_failure(view: &mut View, payload: &Value) {
    let line = format!(
        "✗ planner: {} — {}",
        payload["status"].as_str().unwrap_or_default(),
        payload["error"].as_str().unwrap_or_default()
    );
    view.push(
        Style::default().fg(crate::lean::tui::theme::ayu::RED),
        line.clone(),
    );
    view.system_card("Planner", line, crate::lean::tui::theme::Tone::Failed);
}

fn finish_agent_turn(view: &mut View, key: &str, event: &Value, payload: &Value, failed: bool) {
    if !key.is_empty() {
        let outcome = turn_outcome(payload, failed);
        let agent = view.agents.entry(key.to_owned()).or_default();
        agent.running = false;
        agent.turns += 1;
        let ended = event["created_ms"].as_i64().filter(|ms| *ms > 0);
        if let (Some(start), Some(end)) = (agent.started_ms.take(), ended) {
            let took = end.saturating_sub(start).max(0) as u64;
            agent.busy_ms += took;
            agent.last_ms = Some(took);
        }
        agent.activity = outcome.clone();
    }
}

fn turn_outcome(payload: &Value, failed: bool) -> String {
    if failed {
        format!(
            "✗ {}: {}",
            payload["status"].as_str().unwrap_or_default(),
            payload["error"].as_str().unwrap_or_default()
        )
    } else {
        "✓ turn finished".into()
    }
}

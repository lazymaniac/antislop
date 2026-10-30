use super::*;
use serde_json::Value;

pub(super) fn on_ledger_approval_required(
    view: &mut View,
    event: &Value,
    payload: &Value,
    key: String,
    turn: &str,
) {
    let text = |name: &str| payload[name].as_str().unwrap_or_default().to_owned();
    let _ = (event, &key, turn, &text);
    view.requirements_approval_event = view.after;
    view.requirements_approval_run = payload["run"].as_str().map(str::to_owned);
}

pub(super) fn on_ledger_changed(
    view: &mut View,
    event: &Value,
    payload: &Value,
    key: String,
    turn: &str,
) {
    let text = |name: &str| payload[name].as_str().unwrap_or_default().to_owned();
    let _ = (event, &key, turn, &text);
    if view.requirements_approval_run.as_deref() == payload["run"].as_str() {
        view.requirements_approval_run = None;
    }
    if event["kind"] == "ledger_proposal_rejected" {
        view.requirements_progress = Some((
            false,
            "Requirements rejected. Preparing a revised proposal for review.".into(),
        ));
    }
}

pub(super) fn on_user_decision_requested(
    view: &mut View,
    event: &Value,
    payload: &Value,
    key: String,
    turn: &str,
) {
    let text = |name: &str| payload[name].as_str().unwrap_or_default().to_owned();
    let _ = (event, &key, turn, &text);
    if let Ok(question) =
        serde_json::from_value::<crate::lean::interview::Question>(payload.clone())
    {
        view.user_decision = true;
        view.system_card(
            "Decision required",
            question.transcript(),
            crate::lean::tui::theme::Tone::Waiting,
        );
        view.interview_question = Some((view.after, question));
    }
}

pub(super) fn on_interview_question(
    view: &mut View,
    event: &Value,
    payload: &Value,
    key: String,
    turn: &str,
) {
    let text = |name: &str| payload[name].as_str().unwrap_or_default().to_owned();
    let _ = (event, &key, turn, &text);
    if let Ok(question) =
        serde_json::from_value::<crate::lean::interview::Question>(payload.clone())
        && (2..=6).contains(&question.options.len())
    {
        view.interview_question = Some((view.after, question));
        view.user_decision = false;
        view.requirements_summary.clear();
    }
}

pub(super) fn on_session_started(
    view: &mut View,
    event: &Value,
    payload: &Value,
    key: String,
    turn: &str,
) {
    let text = |name: &str| payload[name].as_str().unwrap_or_default().to_owned();
    let _ = (event, &key, turn, &text);
    view.session_branch = payload["branch"].as_str().map(clean_line);
}

pub(super) fn on_planning_progress(
    view: &mut View,
    event: &Value,
    payload: &Value,
    key: String,
    turn: &str,
) {
    let text = |name: &str| payload[name].as_str().unwrap_or_default().to_owned();
    let _ = (event, &key, turn, &text);
    view.requirements_progress = Some((false, text("text")));
    view.cards.push(TranscriptCard::System {
        title: "Planning".into(),
        meta: String::new(),
        tone: crate::lean::tui::theme::Tone::Active,
        text: text("text"),
    });
    view.push(dim(), text("text"));
}

pub(super) fn on_workflow(
    view: &mut View,
    event: &Value,
    payload: &Value,
    key: String,
    turn: &str,
) {
    let text = |name: &str| payload[name].as_str().unwrap_or_default().to_owned();
    let _ = (event, &key, turn, &text);
    view.workflow_phase = Some(text("phase"));
    if view.workflow_phase.as_deref() != Some("interview") && !view.user_decision {
        view.interview_question = None;
    }
    if payload["summary"].is_string() {
        view.requirements_summary = if payload["upto_message"]
            .as_u64()
            .is_some_and(|upto| upto < view.latest_user_message)
        {
            String::new()
        } else {
            text("summary")
        };
    }
}

pub(super) fn on_message(
    _view: &mut View,
    event: &Value,
    payload: &Value,
    key: String,
    turn: &str,
) {
    let text = |name: &str| payload[name].as_str().unwrap_or_default().to_owned();
    let _ = (event, &key, turn, &text);
    {};
}

pub(super) fn on_message_2(
    view: &mut View,
    event: &Value,
    payload: &Value,
    key: String,
    turn: &str,
) {
    let _ = (event, &key, turn);
    match payload["author"].as_str().unwrap_or_default() {
        // Shown by their `comment` and `code_review` events.
        "user" if message_text(payload).starts_with(crate::lean::COMMENT_PREFIX) => {}
        "code-review" => {}
        "user" => record_user_message(view, payload),
        "runtime" => record_runtime_message(view),
        ORCHESTRATOR | PLAN_VERIFIER | crate::lean::PLANNER | "requirements-planner"
            if view.workflow_phase.as_deref() == Some("planning") => {}
        author => record_agent_message(view, author, payload),
    };
}

fn message_text(payload: &Value) -> String {
    payload["text"].as_str().unwrap_or_default().to_owned()
}

fn record_user_message(view: &mut View, payload: &Value) {
    let text = message_text(payload);
    update_user_message_state(view, payload, &text);
    let message = MessageEntry {
        id: format!("m{}", view.after),
        sender: "You".into(),
        target: "orchestrator".into(),
        text: text.clone(),
        state: "sent".into(),
        kind: "user".into(),
    };
    append_user_message(view, message, text);
}

fn update_user_message_state(view: &mut View, payload: &Value, text: &str) {
    view.latest_user_message = payload["seq"].as_u64().unwrap_or(view.latest_user_message);
    if view.workflow_phase.as_deref() == Some("interview") {
        view.requirements_summary.clear();
    }
    if view.user_decision
        && view
            .interview_question
            .as_ref()
            .is_some_and(|(_, question)| {
                text.starts_with(&format!("Interview answer ({})\n", question.id))
            })
    {
        view.interview_question = None;
        view.user_decision = false;
    }
}

fn append_user_message(view: &mut View, message: MessageEntry, text: String) {
    view.messages.push(message.clone());
    view.cards.push(TranscriptCard::Message(message));
    view.push(
        Style::default()
            .fg(crate::lean::tui::theme::ayu::BLUE)
            .add_modifier(Modifier::BOLD),
        "You",
    );
    view.push(Style::default(), text);
    view.push(Style::default(), "");
}

fn record_runtime_message(view: &mut View) {
    view.cards.push(TranscriptCard::System {
        title: "Runtime".into(),
        meta: String::new(),
        text: "· task results handed to orchestrator".into(),
        tone: crate::lean::tui::Tone::Queued,
    });
    view.push(dim(), "· task results handed to orchestrator");
    view.push(Style::default(), "");
}

fn record_agent_message(view: &mut View, author: &str, payload: &Value) {
    let display = match author {
        ORCHESTRATOR => "orchestrator",
        PLAN_VERIFIER => "plan verifier",
        other => other,
    };
    let message = MessageEntry {
        id: format!("m{}", view.after),
        sender: display.to_owned(),
        target: "You".into(),
        text: message_text(payload),
        state: "delivered".into(),
        kind: "agent".into(),
    };
    view.messages.push(message.clone());
    view.cards.push(TranscriptCard::Message(message));
    view.push(
        Style::default()
            .fg(crate::lean::tui::theme::ayu::GREEN)
            .add_modifier(Modifier::BOLD),
        display,
    );
    view.push(Style::default(), message_text(payload));
    view.push(Style::default(), "");
}

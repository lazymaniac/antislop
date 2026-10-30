use super::*;
use serde_json::Value;

pub(super) fn on_requirements_failed(
    view: &mut View,
    event: &Value,
    payload: &Value,
    key: String,
    turn: &str,
) {
    let text = |name: &str| payload[name].as_str().unwrap_or_default().to_owned();
    let _ = (event, &key, turn, &text);
    if payload["retrying"] == true {
        view.requirements_progress = Some((false, text("retry").to_owned()));
        view.push(
            Style::default(),
            format!("{}\n{}", text("error"), text("retry")),
        );
        return;
    }
    view.requirements_progress = Some((true, format!("{} — {}", text("error"), text("retry"))));
    let message = format!("{}\n{}", text("error"), text("retry"));
    view.cards.push(TranscriptCard::System {
        title: "Requirements preparation failed".into(),
        meta: String::new(),
        text: message.clone(),
        tone: crate::lean::tui::theme::Tone::Failed,
    });
    view.push(
        Style::default().fg(crate::lean::tui::theme::ayu::RED),
        message,
    );
}

pub(super) fn on_paused(view: &mut View, event: &Value, payload: &Value, key: String, turn: &str) {
    let text = |name: &str| payload[name].as_str().unwrap_or_default().to_owned();
    let _ = (event, &key, turn, &text);
    view.paused = payload["paused"] == true;
    let txt = if view.paused {
        "⏸ paused: new steps wait (F6 continues)"
    } else {
        "▶ resumed"
    };
    view.cards.push(TranscriptCard::System {
        title: "Pause".into(),
        meta: String::new(),
        text: txt.into(),
        tone: crate::lean::tui::theme::Tone::Waiting,
    });
    view.push(
        Style::default().fg(crate::lean::tui::theme::ayu::YELLOW),
        txt,
    );
}

pub(super) fn on_notice(view: &mut View, event: &Value, payload: &Value, key: String, turn: &str) {
    let text = |name: &str| payload[name].as_str().unwrap_or_default().to_owned();
    let _ = (event, &key, turn, &text);
    let txt = text("text");
    view.cards.push(TranscriptCard::System {
        title: "Notice".into(),
        meta: String::new(),
        text: txt.clone(),
        tone: crate::lean::tui::theme::Tone::Waiting,
    });
    view.push(
        Style::default().fg(crate::lean::tui::theme::ayu::YELLOW),
        format!("⚠ {txt}"),
    );
}

pub(super) fn on_mode(view: &mut View, event: &Value, payload: &Value, key: String, turn: &str) {
    let text = |name: &str| payload[name].as_str().unwrap_or_default().to_owned();
    let _ = (event, &key, turn, &text);
    view.autonomous = Some(payload["autonomous"] == true);
    let text = if payload["autonomous"] == true {
        "⚙ autonomous mode: next plans and rounds start automatically"
    } else {
        "⚙ supervised mode: next plans wait for approval (Ctrl+A)"
    };
    view.push(
        Style::default().fg(crate::lean::tui::theme::ayu::YELLOW),
        text,
    );
    view.system_card("Mode", text.into(), crate::lean::tui::theme::Tone::Queued);
}

pub(super) fn on_audit(view: &mut View, event: &Value, payload: &Value, key: String, turn: &str) {
    let text = |name: &str| payload[name].as_str().unwrap_or_default().to_owned();
    let _ = (event, &key, turn, &text);
    let lines = crate::lean::cli::review_lines("audit", payload);
    for line in &lines {
        view.push(
            Style::default().fg(crate::lean::tui::theme::ayu::YELLOW),
            line.clone(),
        );
    }
    let tone = if payload["approved"] == true {
        crate::lean::tui::theme::Tone::Done
    } else {
        crate::lean::tui::theme::Tone::Waiting
    };
    view.system_card("Audit", lines.join("\n"), tone);
    view.review(AUDITOR, payload);
}

pub(super) fn on_code_review(
    view: &mut View,
    event: &Value,
    payload: &Value,
    key: String,
    turn: &str,
) {
    let text = |name: &str| payload[name].as_str().unwrap_or_default().to_owned();
    let _ = (event, &key, turn, &text);
    let task = text("id");
    if payload["status"] == "running" {
        view.push(dim(), format!("· code review {task} in progress"));
    } else {
        let who = format!("code review {task}");
        let lines = crate::lean::cli::review_lines(&who, payload);
        for line in &lines {
            view.push(
                Style::default().fg(crate::lean::tui::theme::ayu::YELLOW),
                line.clone(),
            );
        }
        view.system_card(
            &format!("Code review {task}"),
            lines.join("\n"),
            crate::lean::tui::theme::Tone::Waiting,
        );
        view.reviews.insert(format!("worker:{task}"), lines);
    }
}

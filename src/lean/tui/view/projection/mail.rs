use super::*;
use serde_json::Value;

pub(super) fn on_comment_replied(
    view: &mut View,
    event: &Value,
    payload: &Value,
    key: String,
    turn: &str,
) {
    let text = |name: &str| payload[name].as_str().unwrap_or_default().to_owned();
    let _ = (event, &key, turn, &text);
    // Answered in words: the comment stays open for the user.
    for id in payload["comments"].as_array().into_iter().flatten() {
        if let Some(id) = id.as_u64()
            && let Some(progress) = view.comment_progress.get_mut(&id)
            && progress.state == "addressed"
        {
            progress.state = "answered".into();
        }
    }
}

pub(super) fn on_mail(view: &mut View, payload: &Value) {
    let recipient = event_text(payload, "recipient");
    let mail_id = payload["id"].as_u64().unwrap_or_default();
    view.mail.insert(mail_id, recipient.clone());
    let sender = mail_sender(&event_text(payload, "sender"));
    let id_str = if mail_id > 0 {
        format!("{mail_id}")
    } else {
        format!("m{}", view.after)
    };
    let message = mail_message_entry(&id_str, &sender, &recipient, payload);
    view.messages.push(message.clone());
    view.cards.push(TranscriptCard::Message(message));
    let line = mail_line(&sender, &recipient, payload);
    view.push(
        Style::default().fg(crate::lean::tui::theme::ayu::PURPLE),
        format!("✉ {line} · waiting for recipient's next turn"),
    );
    let from = View::mail_key(&event_text(payload, "sender"));
    let to = View::mail_key(&recipient);
    record_mail_link(view, from, to, &line);
}

fn mail_message_entry(id: &str, sender: &str, recipient: &str, payload: &Value) -> MessageEntry {
    MessageEntry {
        id: id.to_owned(),
        sender: sender.to_owned(),
        target: recipient.to_owned(),
        text: event_text(payload, "text"),
        state: "waiting for turn".to_owned(),
        kind: event_text(payload, "kind"),
    }
}

fn mail_line(sender: &str, recipient: &str, payload: &Value) -> String {
    format!(
        "{sender} → {recipient} ({}): {}",
        event_text(payload, "kind"),
        event_text(payload, "text")
    )
}

fn record_mail_link(view: &mut View, from: String, to: String, line: &str) {
    for key in [&from, &to] {
        view.mailbox
            .entry(key.clone())
            .or_default()
            .push(clean(line));
    }
    let link = view.links.entry((from, to)).or_insert(Link {
        count: 0,
        last: Instant::now(),
    });
    link.count += 1;
    link.last = Instant::now();
}

pub(super) fn on_mail_delivered(
    view: &mut View,
    event: &Value,
    payload: &Value,
    key: String,
    turn: &str,
) {
    let text = |name: &str| payload[name].as_str().unwrap_or_default().to_owned();
    let _ = (event, &key, turn, &text);
    for id in payload["ids"].as_array().into_iter().flatten() {
        if let Some(mail_id) = id.as_u64() {
            let id_str = format!("{mail_id}");
            for msg in &mut view.messages {
                if msg.id == id_str {
                    msg.state = "delivered".to_owned();
                }
            }
            for card in &mut view.cards {
                if let TranscriptCard::Message(msg) = card
                    && msg.id == id_str
                {
                    msg.state = "delivered".to_owned();
                }
            }
            if let Some(recipient) = view.mail.remove(&mail_id) {
                view.push(dim(), format!("✓ {recipient} received message"));
            }
        }
    }
}

pub(super) fn on_mail_rejected(view: &mut View, payload: &Value) {
    let sender = event_text(payload, "sender");
    let recipient = event_text(payload, "recipient");
    mark_rejected_messages(view, &sender, &recipient);
    view.push(
        Style::default().fg(crate::lean::tui::theme::ayu::RED),
        format!(
            "✗ message from {} to {} undeliverable: {}",
            sender,
            recipient,
            event_text(payload, "reason")
        ),
    );
}

fn mark_rejected_messages(view: &mut View, sender: &str, recipient: &str) {
    let normalized = mail_sender(sender);
    for msg in view.messages.iter_mut().rev() {
        if (msg.sender == sender || msg.sender == normalized) && msg.target == recipient {
            msg.state = "rejected".to_owned();
            break;
        }
    }
    for card in view.cards.iter_mut().rev() {
        if let TranscriptCard::Message(msg) = card
            && (msg.sender == sender || msg.sender == normalized)
            && msg.target == recipient
        {
            msg.state = "rejected".to_owned();
            break;
        }
    }
}

pub(super) fn on_comment(view: &mut View, payload: &Value) {
    let Ok(comment) = serde_json::from_value::<Comment>(payload.clone()) else {
        return;
    };
    store_comment_scope(view, &comment, payload);
    mark_comment_queued(view, &comment);
    let name = target_name(&comment.target);
    let was_draft = view
        .comments
        .get(&comment.id)
        .is_some_and(|old| old.status == "draft");
    let (line, card_status) = comment_event_text(
        &comment,
        &name,
        was_draft,
        view.comments.contains_key(&comment.id),
    );
    view.cards.push(TranscriptCard::Comment {
        id: comment.id,
        target: name,
        text: comment.text.clone(),
        status: card_status,
    });
    view.push(
        Style::default().fg(crate::lean::tui::theme::ayu::BLUE),
        line,
    );
    view.comments.insert(comment.id, comment);
}

fn store_comment_scope(view: &mut View, comment: &Comment, payload: &Value) {
    if payload.get("round").is_some() || payload.get("run").is_some() {
        view.comment_scopes.insert(
            comment.id,
            (
                payload["round"].as_u64(),
                payload["run"].as_str().map(str::to_owned),
            ),
        );
    }
}

fn mark_comment_queued(view: &mut View, comment: &Comment) {
    if comment.status == "open" {
        view.comment_progress.insert(
            comment.id,
            CommentProgress {
                state: "queued".into(),
                error: None,
            },
        );
    }
}

fn comment_event_text(
    comment: &Comment,
    name: &str,
    was_draft: bool,
    exists: bool,
) -> (String, String) {
    match comment.status.as_str() {
        "draft" => (
            format!("✎ #{} draft to {name}: {}", comment.id, comment.text),
            "draft".into(),
        ),
        "open" if was_draft => (format!("✎ #{} sent for review", comment.id), "sent".into()),
        "open" if !exists => (
            format!("✎ #{} to {name}: {}", comment.id, comment.text),
            "open".into(),
        ),
        "open" => (format!("✎ #{} reopened", comment.id), "reopened".into()),
        "resolved" => (format!("✎ #{} resolved", comment.id), "resolved".into()),
        _ => (format!("✎ #{} deleted", comment.id), "deleted".into()),
    }
}

fn event_text(payload: &Value, name: &str) -> String {
    payload[name].as_str().unwrap_or_default().to_owned()
}

fn mail_sender(sender: &str) -> String {
    match sender {
        "user" => "You".into(),
        ORCHESTRATOR => "orchestrator".into(),
        other => other.into(),
    }
}

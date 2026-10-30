use super::*;

pub(super) fn messages(details: &mut Details, view: &View, records: &mut Vec<Rc<DetailRecord>>) {
    let messages = details
        .task
        .as_ref()
        .and_then(|task| task["messages"].as_array());
    match messages {
        Some(messages) => append_messages(messages, records),
        None => super::mailbox::mailbox(details, view, records),
    }
}

fn append_messages(messages: &[Value], records: &mut Vec<Rc<DetailRecord>>) {
    records.extend(
        messages
            .iter()
            .map(|message| record(vec![plain(message_text(message))], " ")),
    );
    if records.is_empty() {
        records.push(no_messages());
    }
}

fn message_text(message: &Value) -> String {
    let delivery = if message["delivered_turn"].is_string() {
        "delivered"
    } else {
        "queued"
    };
    format!(
        "{} -> {} · {delivery}\n{}",
        message["sender"].as_str().unwrap_or_default(),
        message["recipient"].as_str().unwrap_or_default(),
        message["text"].as_str().unwrap_or_default()
    )
}

fn no_messages() -> Rc<DetailRecord> {
    Rc::new(DetailRecord {
        lines: vec![(dim(), "No messages in this round.".into())],
        search_text: String::new(),
    })
}

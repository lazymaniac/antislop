pub(in crate::lean) fn fenced<'a>(text: &'a str, tag: &str) -> Option<(String, &'a str)> {
    let open = format!("```{tag}");
    let start = text.rfind(&open)?;
    let body_start = start + open.len();
    let rest = &text[body_start..];
    if !rest.starts_with(['\n', '\r', ' ']) {
        return None;
    }
    // The closing fence stands on its own line; a ``` inside the body (in a
    // JSON string, say) does not end it. A one-line block closes at the first.
    let end = rest
        .find("\n```")
        .map_or_else(|| rest.find("```"), |at| Some(at + 1))?;
    let mut remaining = text[..start].trim_end().to_owned();
    let after = rest[end + 3..].trim();
    if !after.is_empty() {
        remaining.push_str("\n\n");
        remaining.push_str(after);
    }
    Some((remaining, rest[..end].trim()))
}

/// Messages delivered into a turn, before its own prompt.
pub fn inbox_text(mail: &[crate::lean::store::Mail]) -> String {
    inbox_text_from(mail.iter())
}

pub(crate) fn inbox_text_from<'a>(
    mail: impl IntoIterator<Item = &'a crate::lean::store::Mail>,
) -> String {
    let mut text = String::from("Messages for you:\n");
    for message in mail {
        use std::fmt::Write as _;
        let _ = writeln!(
            text,
            "- from {} ({}): {}",
            message.sender, message.kind, message.text
        );
    }
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inbox_text_from_keeps_format_while_borrowing_filtered_messages() {
        let messages = [
            crate::lean::store::Mail {
                id: 1,
                created_ms: 0,
                delivered_turn: None,
                delivered_ms: None,
                sender: "worker:a".into(),
                recipient: "worker:b".into(),
                kind: "agent_work".into(),
                text: "first".into(),
                state: "queued".into(),
            },
            crate::lean::store::Mail {
                id: 2,
                created_ms: 0,
                delivered_turn: None,
                delivered_ms: None,
                sender: "user".into(),
                recipient: "worker:b".into(),
                kind: "guidance".into(),
                text: "second".into(),
                state: "queued".into(),
            },
        ];

        let expected = "Messages for you:\n- from worker:a (agent_work): first\n- from user (guidance): second\n\n";
        assert_eq!(inbox_text(&messages), expected);
        assert_eq!(
            inbox_text_from(messages.iter().skip(1)),
            "Messages for you:\n- from user (guidance): second\n\n"
        );
    }
}

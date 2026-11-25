use crate::lean::prompts;
/// Messages delivered into a turn, before its own prompt.
pub fn inbox_text(mail: &[crate::lean::store::Mail]) -> String {
    inbox_text_from(mail.iter())
}

pub(crate) fn inbox_text_from<'a>(
    mail: impl IntoIterator<Item = &'a crate::lean::store::Mail>,
) -> String {
    let mut text = String::from(prompts::INBOX_HEADING);
    for message in mail {
        use std::fmt::Write as _;
        let _ = writeln!(
            text,
            prompts::template!(INBOX_MESSAGE),
            message.sender,
            message.kind,
            if message.sender == crate::lean::RUNTIME && message.kind == "agent_work" {
                message.text.clone()
            } else {
                crate::lean::prompt_context::untrusted(prompts::INBOX_MESSAGE_LABEL, &message.text)
            }
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

        let all = inbox_text(&messages);
        assert!(all.contains("worker:a (agent_work)"));
        assert!(all.contains("user (guidance)"));
        assert_eq!(all.matches("untrusted reference data").count(), 2);
        assert!(all.contains("first") && all.contains("second"));
        let filtered = inbox_text_from(messages.iter().skip(1));
        assert!(!filtered.contains("first") && filtered.contains("second"));
    }
}

use super::*;
mod checks;
mod comments;
mod overview;
mod plan;
mod results;
mod review;

impl Details {
    pub(in crate::lean::tui) fn records(&mut self, view: &View) -> Vec<Rc<DetailRecord>> {
        let mut records = Vec::new();
        match self.tab {
            PLAN => plan::plan(self, view, &mut records),
            0 | REVIEWER => overview::overview(self, view, &mut records),
            REVIEW => review::review(self, view, &mut records),
            COMMENTS => comments::comments(self, view, &mut records),
            CHECKS => checks::checks(self, view, &mut records),
            _ => results::results(self, view, &mut records),
        }
        records
    }

    pub(in crate::lean::tui) fn project_dialogue_entries(
        &self,
        view: &View,
    ) -> Vec<Rc<DetailRecord>> {
        conversation::records(self, view)
    }
}

mod conversation {
    use super::*;
    use crate::lean::tui::view::MessageEntry;
    use crate::lean::tui::view::TranscriptCard;
    use std::collections::HashSet;
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;

    pub(super) fn records(details: &Details, view: &View) -> Vec<Rc<DetailRecord>> {
        let mut records = Vec::new();
        let mut messages = view.messages.clone();
        let mut retained_ids = messages
            .iter()
            .map(|message| message.id.clone())
            .collect::<HashSet<_>>();
        for card in &view.cards {
            if let TranscriptCard::Message(message) = card
                && retained_ids.insert(message.id.clone())
            {
                messages.push(message.clone());
            }
        }
        for message in messages {
            if details.key != "conversation"
                && normalize_identity(view, &message.sender) != details.key
                && normalize_identity(view, &message.target) != details.key
            {
                continue;
            }
            push_message(&mut records, details, message);
        }
        if details.key == "conversation" {
            for card in &view.cards {
                if matches!(card, TranscriptCard::Message(_)) {
                    continue;
                }
                push_event_card(&mut records, details, card);
            }
        } else if let Some((role, task_id)) = details.key.split_once(':')
            && matches!(role, "worker" | "reviewer")
        {
            let prefix = format!("{task_id} ");
            for card in &view.cards {
                let TranscriptCard::System {
                    title,
                    text,
                    sent_at,
                    ..
                } = card
                else {
                    continue;
                };
                if title != "Progress" {
                    continue;
                }
                let lines = text
                    .lines()
                    .filter(|line| line.starts_with(&prefix))
                    .collect::<Vec<_>>();
                if !lines.is_empty() {
                    push_record(
                        &mut records,
                        details,
                        format!("Task progress · {task_id}"),
                        lines.join("\n"),
                        *sent_at,
                    );
                }
            }
        }
        if records.is_empty() {
            records.push(record(
                vec![plain(
                    "No retained conversation history is available for this target.".into(),
                )],
                " ",
            ));
        }
        records
    }

    fn normalize_identity(view: &View, identity: &str) -> String {
        if matches!(
            identity.split_once(':').map(|(role, _)| role),
            Some("worker" | "reviewer")
        ) {
            view.agent_key(identity)
        } else {
            view.agent_key(&View::mail_key(identity))
        }
    }

    fn push_message(records: &mut Vec<Rc<DetailRecord>>, details: &Details, message: MessageEntry) {
        let title = format!(
            "{} → {} · {}",
            message.sender, message.target, message.state
        );
        push_record(records, details, title, message.text, message.sent_at);
    }

    fn push_event_card(
        records: &mut Vec<Rc<DetailRecord>>,
        details: &Details,
        card: &TranscriptCard,
    ) {
        let (title, body, sent_at) = match card {
            TranscriptCard::Message(_) => return,
            TranscriptCard::Plan {
                plan,
                text,
                sent_at,
                ..
            } => (format!("Plan {plan}"), text.clone(), *sent_at),
            TranscriptCard::System {
                title,
                meta,
                text,
                sent_at,
                ..
            } => (format!("{title} · {meta}"), text.clone(), *sent_at),
            TranscriptCard::Comment {
                id,
                target,
                text,
                status,
                sent_at,
            } => (
                format!("Comment #{id} · {target} · {status}"),
                text.clone(),
                *sent_at,
            ),
            TranscriptCard::Completion {
                summary,
                latest_result,
                sent_at,
                ..
            } => (
                "Session completion".into(),
                latest_result.as_deref().unwrap_or(summary).to_owned(),
                *sent_at,
            ),
        };
        push_record(records, details, title, body, sent_at);
    }

    fn push_record(
        records: &mut Vec<Rc<DetailRecord>>,
        details: &Details,
        title: String,
        body: String,
        sent_at: Option<i64>,
    ) {
        let date = sent_at.and_then(message_datetime).unwrap_or_default();
        let date_width = UnicodeWidthStr::width(date.as_str());
        let width = details.text_area.width as usize;
        let title = fit_header(&title, width.saturating_sub(date_width + 1));
        let title_width = UnicodeWidthStr::width(title.as_str());
        let gap = width.saturating_sub(title_width + date_width).max(1);
        let mut lines = vec![(
            Style::default().add_modifier(Modifier::BOLD),
            format!("{title}{}{date}", " ".repeat(gap)),
        )];
        lines.extend(body.lines().map(|line| plain(line.to_owned())));
        records.push(record(lines, "\n"));
    }

    fn fit_header(text: &str, width: usize) -> String {
        if UnicodeWidthStr::width(text) <= width {
            return text.to_owned();
        }
        let available = width.saturating_sub(1);
        let mut clipped = String::new();
        let mut used = 0;
        for grapheme in text.graphemes(true) {
            let next = UnicodeWidthStr::width(grapheme);
            if used + next > available {
                break;
            }
            clipped.push_str(grapheme);
            used += next;
        }
        if width > 0 {
            clipped.push('…');
        }
        clipped
    }

    fn message_datetime(milliseconds: i64) -> Option<String> {
        let seconds = milliseconds.div_euclid(1000) as libc::time_t;
        let mut calendar = std::mem::MaybeUninit::<libc::tm>::uninit();
        // SAFETY: localtime_r writes to the initialized output buffer.
        if unsafe { libc::localtime_r(&seconds, calendar.as_mut_ptr()) }.is_null() {
            return None;
        }
        // SAFETY: localtime_r succeeded and initialized the structure.
        let calendar = unsafe { calendar.assume_init() };
        Some(format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            calendar.tm_year + 1900,
            calendar.tm_mon + 1,
            calendar.tm_mday,
            calendar.tm_hour,
            calendar.tm_min,
            calendar.tm_sec
        ))
    }
}

fn record(lines: Vec<(Style, String)>, separator: &str) -> Rc<DetailRecord> {
    let search_text = lines
        .iter()
        .map(|(_, text)| text.as_str())
        .collect::<Vec<_>>()
        .join(separator);
    Rc::new(DetailRecord { lines, search_text })
}

fn plain(text: String) -> (Style, String) {
    (Style::default(), text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lean::tui::view::MessageEntry;
    use crate::lean::tui::view::TranscriptCard;

    #[test]
    fn record_search_text_joins_lines_with_the_screen_separator() {
        let lines = vec![plain("first".into()), plain("second".into())];
        assert_eq!(record(lines.clone(), " ").search_text, "first second");
        assert_eq!(record(lines, "\n").search_text, "first\nsecond");
    }

    #[test]
    fn workbench_conversation_uses_retained_messages_and_marks_missing_history() {
        let mut details = Details::new("conversation".into());
        details.text_area.width = 80;
        let mut view = View::default();
        view.cards.push(TranscriptCard::Message(MessageEntry {
            id: "m1".into(),
            sent_at: Some(1_700_000_000_000),
            sender: "You".into(),
            target: "Session".into(),
            text: "Retained answer".into(),
            state: "sent".into(),
            kind: "user".into(),
        }));
        let lines = details
            .project_dialogue_entries(&view)
            .into_iter()
            .flat_map(|record| record.lines.clone())
            .map(|(_, line)| line)
            .collect::<Vec<_>>();
        assert!(lines.iter().any(|line| line.contains("Retained answer")));
        let header = lines
            .iter()
            .find(|line| line.contains("You → Session"))
            .unwrap();
        assert!(header.contains("2023-11-14"), "{header}");
        assert_eq!(unicode_width::UnicodeWidthStr::width(header.as_str()), 80);

        let empty = details.project_dialogue_entries(&View::default());
        assert!(
            empty[0]
                .search_text
                .contains("No retained conversation history")
        );
    }

    #[test]
    fn agent_conversation_excludes_unscoped_session_cards_and_other_targets() {
        let mut view = View::default();
        view.cards.extend([
            TranscriptCard::Message(MessageEntry {
                id: "other".into(),
                sent_at: None,
                sender: "worker:t2".into(),
                target: "orchestrator".into(),
                text: "other task message".into(),
                state: "sent".into(),
                kind: "agent".into(),
            }),
            TranscriptCard::Message(MessageEntry {
                id: "owned".into(),
                sent_at: None,
                sender: "worker:t1".into(),
                target: "orchestrator".into(),
                text: "target task message".into(),
                state: "sent".into(),
                kind: "agent".into(),
            }),
            TranscriptCard::Plan {
                sent_at: None,
                plan: "p2".into(),
                tasks: 1,
                text: "other task plan".into(),
            },
            TranscriptCard::System {
                sent_at: None,
                title: "Progress".into(),
                meta: String::new(),
                text: "t1 Owned task is working\nt2 Foreign task is working\nt10 Prefix collision"
                    .into(),
                tone: crate::lean::tui::theme::Tone::Queued,
            },
            TranscriptCard::Comment {
                sent_at: None,
                id: 4,
                target: "task t2".into(),
                text: "other task comment".into(),
                status: "open".into(),
            },
            TranscriptCard::Completion {
                sent_at: None,
                branch: None,
                summary: "session completed".into(),
                actions: Vec::new(),
                selected: 0,
                latest_result: None,
            },
        ]);

        let agent = Details::new("worker:t1".into());
        let agent_records = agent.project_dialogue_entries(&view);
        assert_eq!(agent_records.len(), 2);
        assert!(agent_records[0].search_text.contains("target task message"));
        assert!(
            agent_records[1]
                .search_text
                .contains("t1 Owned task is working")
        );
        for unrelated in [
            "other task message",
            "other task plan",
            "other task event",
            "other task comment",
            "session completed",
            "t2 Foreign task is working",
            "t10 Prefix collision",
        ] {
            assert!(
                agent_records
                    .iter()
                    .all(|record| !record.search_text.contains(unrelated))
            );
        }

        let session = Details::new("conversation".into()).project_dialogue_entries(&view);
        assert_eq!(session.len(), view.cards.len());
        assert!(
            session
                .iter()
                .any(|record| record.search_text.contains("other task plan"))
        );
        assert!(
            session
                .iter()
                .any(|record| record.search_text.contains("session completed"))
        );
    }

    #[test]
    fn agent_conversation_includes_mail_without_cards_and_normalizes_task_uuids() {
        let mut view = View::default();
        view.apply(&serde_json::json!({
            "seq": 1,
            "kind": "plan_proposed",
            "payload": {"plan":"p1", "tasks":[{"id":"t1", "title":"Task", "change_kind":"none"}]}
        }));
        view.apply(&serde_json::json!({
            "seq": 2,
            "kind": "task",
            "payload": {"id":"t1", "task":"task-uuid", "status":"working", "round":1}
        }));
        view.apply(&serde_json::json!({
            "seq": 3,
            "created_ms": 1_700_000_000_000_i64,
            "kind": "mail",
            "payload": {"id":7, "sender":"task-uuid", "recipient":"orchestrator", "text":"agent-only mail", "kind":"agent"}
        }));
        assert!(
            view.cards
                .iter()
                .all(|card| !matches!(card, TranscriptCard::Message(_))),
            "agent-to-agent mail has no message card"
        );
        assert_eq!(view.messages.len(), 1);

        let details = Details::new("worker:t1".into());
        let records = details.project_dialogue_entries(&view);
        assert_eq!(records.len(), 1);
        assert!(records[0].search_text.contains("agent-only mail"));
        let session_records = Details::new("conversation".into()).project_dialogue_entries(&view);
        assert!(
            session_records
                .iter()
                .any(|record| record.search_text.contains("agent-only mail"))
        );

        let mut card_view = view;
        card_view.messages.clear();
        card_view.cards.push(TranscriptCard::Message(MessageEntry {
            id: "card-uuid".into(),
            sent_at: None,
            sender: "worker:task-uuid".into(),
            target: "orchestrator".into(),
            text: "role-shaped mail".into(),
            state: "sent".into(),
            kind: "agent".into(),
        }));
        let records = details.project_dialogue_entries(&card_view);
        assert!(records[0].search_text.contains("role-shaped mail"));
    }
}

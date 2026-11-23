use super::*;
use crate::lean::tui::view;
mod checks;
mod comments;
mod overview;
mod plan;
mod results;
mod review;

#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct ConversationCache {
    key: String,
    width: u16,
    messages: std::collections::HashMap<String, (view::MessageEntry, Rc<DetailRecord>)>,
    cards: Vec<(view::TranscriptCard, Vec<Rc<DetailRecord>>)>,
    empty: Option<Rc<DetailRecord>>,
}

impl Details {
    pub(in crate::lean::tui) fn records(&mut self, view: &View) -> Vec<Rc<DetailRecord>> {
        let mut records = Vec::new();
        use crate::lean::tui::WorkbenchTab;
        match self.content_tab() {
            Some(WorkbenchTab::Summary) if self.shows_checks() => {
                checks::checks(self, view, &mut records)
            }
            Some(WorkbenchTab::Summary | WorkbenchTab::Diff | WorkbenchTab::Pty) => {
                overview::overview(self, view, &mut records)
            }
            Some(WorkbenchTab::Conversation) => return self.project_dialogue_entries(view),
            Some(WorkbenchTab::Results) => results::results(self, view, &mut records),
            Some(WorkbenchTab::Comments) => comments::comments(self, view, &mut records),
            Some(WorkbenchTab::Checks) => checks::checks(self, view, &mut records),
            None if self.tab == PLAN => plan::plan(self, view, &mut records),
            None if self.tab == REVIEW => review::review(self, view, &mut records),
            None => results::results(self, view, &mut records),
        }
        records
    }

    pub(in crate::lean::tui) fn project_dialogue_entries(
        &mut self,
        view: &View,
    ) -> Vec<Rc<DetailRecord>> {
        let mut cache = std::mem::take(&mut self.conversation_cache);
        if cache.key != self.key || cache.width != self.text_area.width {
            cache = ConversationCache {
                key: self.key.clone(),
                width: self.text_area.width,
                ..Default::default()
            };
        }
        let records = conversation::records(self, view, &mut cache);
        self.conversation_cache = cache;
        records
    }
}

mod conversation {
    use super::*;
    use crate::lean::tui::view::MessageEntry;
    use crate::lean::tui::view::TranscriptCard;
    use std::collections::HashSet;
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;

    pub(super) fn records(
        details: &Details,
        view: &View,
        cache: &mut ConversationCache,
    ) -> Vec<Rc<DetailRecord>> {
        let mut records = Vec::new();
        let mut retained_ids = HashSet::new();
        let prompts: Vec<MessageEntry> = details
            .task
            .as_ref()
            .and_then(|task| task["_conversation_prompts"].as_array())
            .into_iter()
            .flatten()
            .filter_map(|prompt| serde_json::from_value(prompt.clone()).ok())
            .collect();
        let mut messages: Vec<_> = prompts
            .iter()
            .chain(view.messages.iter())
            .chain(view.cards.iter().filter_map(|card| {
                if let TranscriptCard::Message(message) = card {
                    Some(message)
                } else {
                    None
                }
            }))
            .collect();
        messages.sort_by_key(|message| message.sent_at.unwrap_or(i64::MAX));
        for message in messages {
            if !retained_ids.insert(message.id.as_str()) {
                continue;
            }
            if details.key != "conversation" && !view.message_involves(message, &details.key) {
                continue;
            }
            if let Some((old, projected)) = cache.messages.get(&message.id)
                && old == message
            {
                records.push(Rc::clone(projected));
            } else {
                let mut projected = Vec::new();
                push_message(&mut projected, details, message.clone());
                let projected = projected.pop().unwrap();
                cache
                    .messages
                    .insert(message.id.clone(), (message.clone(), Rc::clone(&projected)));
                records.push(projected);
            }
        }
        cache
            .messages
            .retain(|id, _| retained_ids.contains(id.as_str()));
        for (index, card) in view.cards.iter().enumerate() {
            if let Some((old, projected)) = cache.cards.get(index)
                && old == card
            {
                records.extend(projected.iter().map(Rc::clone));
                continue;
            }
            let mut projected = Vec::new();
            if details.key == "conversation" {
                push_event_card(&mut projected, details, card);
            } else if let Some((role, task_id)) = details.key.split_once(':')
                && matches!(role, "worker" | "reviewer")
                && let TranscriptCard::System {
                    title,
                    text,
                    sent_at,
                    ..
                } = card
                && title == "Progress"
            {
                let prefix = format!("{task_id} ");
                let lines = text
                    .lines()
                    .filter(|line| line.starts_with(&prefix))
                    .collect::<Vec<_>>();
                if !lines.is_empty() {
                    push_record(
                        &mut projected,
                        details,
                        format!("Task progress · {task_id}"),
                        lines.join("\n"),
                        *sent_at,
                    );
                }
            }
            records.extend(projected.iter().map(Rc::clone));
            if index < cache.cards.len() {
                cache.cards[index] = (card.clone(), projected);
            } else {
                cache.cards.push((card.clone(), projected));
            }
        }
        cache.cards.truncate(view.cards.len());
        if records.is_empty() {
            records.push(Rc::clone(cache.empty.get_or_insert_with(|| {
                record(
                    vec![plain(
                        "No retained conversation history is available for this target.".into(),
                    )],
                    " ",
                )
            })));
        }
        records
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
        let width = usize::from(details.text_area.width.saturating_sub(3));
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
    fn conversation_projection_reuses_unchanged_records_and_refreshes_changed_inputs() {
        let mut details = Details::new("conversation".into());
        details.text_area.width = 80;
        let mut view = View::default();
        let message = MessageEntry {
            id: "a".into(),
            sent_at: Some(1),
            sender: "You".into(),
            target: "orchestrator".into(),
            text: "retained 👩‍💻 é".repeat(1000),
            state: "sent".into(),
            kind: "user".into(),
        };
        view.messages.push(message.clone());
        let first = details.project_dialogue_entries(&view);
        let unchanged = details.project_dialogue_entries(&view);
        assert!(Rc::ptr_eq(&first[0], &unchanged[0]));
        let mut next = message;
        next.id = "b".into();
        view.messages.push(next);
        let appended = details.project_dialogue_entries(&view);
        assert!(Rc::ptr_eq(&first[0], &appended[0]));
        assert_eq!(appended.len(), 2);
        let mut layout = super::super::model::LayoutCache::default();
        layout.update(first, "", 80);
        let incrementally_wrapped = layout.update(appended, "", 80).to_vec();
        let mut fresh = super::super::model::LayoutCache::default();
        assert_eq!(
            incrementally_wrapped,
            fresh.update(details.project_dialogue_entries(&view), "", 80)
        );
        view.messages[0].state = "delivered".into();
        let changed = details.project_dialogue_entries(&view);
        assert!(!Rc::ptr_eq(&unchanged[0], &changed[0]));
        details.text_area.width = 40;
        let resized = details.project_dialogue_entries(&view);
        assert!(!Rc::ptr_eq(&changed[0], &resized[0]));
        view.messages.clear();
        assert!(
            details.project_dialogue_entries(&view)[0]
                .search_text
                .contains("No retained")
        );
    }

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
        assert_eq!(unicode_width::UnicodeWidthStr::width(header.as_str()), 77);

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

        let mut agent = Details::new("worker:t1".into());
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

        let mut details = Details::new("worker:t1".into());
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

use super::super::*;

struct CommentCardRef<'a> {
    id: u64,
    target: &'a str,
    text: &'a str,
    status: &'a str,
}

impl App {
    pub(super) fn transcript_card_rows(
        &self,
        entry: &TranscriptCard,
        found: bool,
        width: u16,
        theme: &Theme,
    ) -> Vec<Row> {
        let mut rows = if let Some(comment) = comment_card_ref(entry) {
            self.comment_rows(comment, found, width, theme)
        } else {
            standard_card_rows(entry, found, width, theme)
        };
        if let Some(datetime) = entry.sent_at().and_then(message_datetime)
            && let Some(header) = rows.first()
        {
            let fill = header.fill;
            let timestamps = theme::wrap(&datetime, usize::from(width.saturating_sub(2)))
                .into_iter()
                .map(|line| {
                    Row::new(
                        fill,
                        vec![
                            Span::styled(theme.glyphs().bar, fill.patch(theme.dim())),
                            Span::styled(format!(" {line}"), fill.patch(theme.dim())),
                        ],
                    )
                });
            rows.splice(1..1, timestamps);
        }
        rows
    }

    fn comment_rows(
        &self,
        comment: CommentCardRef<'_>,
        found: bool,
        width: u16,
        theme: &Theme,
    ) -> Vec<Row> {
        let (state, error) = comment_status(self, comment.id, comment.status);
        let body = comment_body(comment.text, error.as_deref());
        comment_card(
            comment.id,
            comment.target,
            &body,
            &state,
            found,
            width,
            theme,
        )
    }
}

pub(super) fn comment_status(app: &App, id: u64, status: &str) -> (String, Option<String>) {
    let state = app
        .view
        .comments
        .get(&id)
        .map_or(status, |_| app.view.comment_state(id))
        .to_owned();
    let error = app.view.comment_error(id).map(str::to_owned);
    (state, error)
}

fn comment_card_ref(entry: &TranscriptCard) -> Option<CommentCardRef<'_>> {
    let TranscriptCard::Comment {
        id,
        target,
        text,
        status,
        ..
    } = entry
    else {
        return None;
    };
    Some(CommentCardRef {
        id: *id,
        target,
        text,
        status,
    })
}

fn standard_card_rows(entry: &TranscriptCard, found: bool, width: u16, theme: &Theme) -> Vec<Row> {
    match entry {
        TranscriptCard::Message(msg) => message_rows(msg, found, width, theme),
        TranscriptCard::Plan {
            plan, tasks, text, ..
        } => plan_rows(plan, *tasks, text, found, width, theme),
        TranscriptCard::System {
            title,
            meta,
            text,
            tone,
            ..
        } => system_rows(title, meta, text, tone, found, width, theme),
        TranscriptCard::Completion {
            branch,
            summary,
            actions,
            selected,
            latest_result,
            ..
        } => {
            let branch_line = branch.as_ref().map_or_else(|| "Final branch: unavailable".to_owned(), |branch| format!("Final branch: {branch}\nReview: git diff main...{branch}\nContinue with the result: git switch {branch}"));
            let mut text = format!(
                "{branch_line}\n\nWork summary: {summary}\nReports and retained evidence remain available in session history.\n\nNext action (press 1, 2 or 3):\n"
            );
            for (index, action) in actions.iter().enumerate() {
                let label = match action.kind {
                    crate::lean::daemon::SessionActionKind::CreatePr => "Create PR",
                    crate::lean::daemon::SessionActionKind::MergeWithMain => "Merge with main",
                    crate::lean::daemon::SessionActionKind::ContinueWork => "Continue work",
                };
                text.push_str(&format!(
                    "{} [{}] {}",
                    if index == *selected { ">" } else { " " },
                    index + 1,
                    label
                ));
                if let Some(result) = &action.result {
                    text.push_str(&format!(
                        " — {}",
                        match result {
                            crate::lean::daemon::SessionActionResult::Queued { .. } =>
                                "queued".to_owned(),
                            crate::lean::daemon::SessionActionResult::Succeeded { summary } =>
                                format!("succeeded: {summary}"),
                            crate::lean::daemon::SessionActionResult::Failed { details } =>
                                format!("failed: {details}"),
                            crate::lean::daemon::SessionActionResult::MissingInputs { inputs } =>
                                format!("missing inputs: {}", inputs.join(", ")),
                        }
                    ));
                }
                text.push('\n');
            }
            if let Some(result) = latest_result {
                text.push_str(&format!("\n{result}"));
            }
            system_rows(
                "Session completed",
                "",
                &text,
                &Tone::Done,
                found,
                width,
                theme,
            )
        }
        TranscriptCard::Comment { .. } => unreachable!("comments handled before standard rows"),
    }
}

fn card_fill(found: bool, theme: &Theme) -> Style {
    if found {
        theme.card().add_modifier(Modifier::REVERSED)
    } else {
        theme.card()
    }
}

fn message_rows(msg: &MessageEntry, found: bool, width: u16, theme: &Theme) -> Vec<Row> {
    card(theme, message_card_props(msg, found, width, theme))
}

fn message_card_props<'a>(
    msg: &'a MessageEntry,
    found: bool,
    width: u16,
    theme: &Theme,
) -> CardProps<'a> {
    let fill = if msg.sender == "You" || msg.sender == "user" {
        theme.raised()
    } else {
        theme.card()
    };
    let fill = if found {
        fill.add_modifier(Modifier::REVERSED)
    } else {
        fill
    };
    let person = theme.person(&msg.sender);
    let head = message_heading(msg, person, theme);
    let right = message_right(msg, theme);
    CardProps {
        fill,
        accent: person,
        head,
        right,
        body: &msg.text,
        max_body: usize::MAX,
        width,
    }
}

fn message_heading(msg: &MessageEntry, person: Style, theme: &Theme) -> Vec<Span<'static>> {
    vec![
        Span::styled(theme.clean(&msg.sender), person.patch(theme.title())),
        Span::styled(format!(" {} ", theme.glyphs().to), theme.faint()),
        Span::styled(theme.clean(&msg.target), theme.person(&msg.target)),
    ]
}

fn message_right(msg: &MessageEntry, theme: &Theme) -> String {
    theme.clean(&msg.state)
}

fn message_datetime(milliseconds: i64) -> Option<String> {
    let seconds = milliseconds.div_euclid(1000) as libc::time_t;
    let mut calendar = std::mem::MaybeUninit::<libc::tm>::uninit();
    // SAFETY: localtime_r writes to a valid tm buffer and reads the supplied timestamp.
    if unsafe { libc::localtime_r(&seconds, calendar.as_mut_ptr()) }.is_null() {
        return None;
    }
    // SAFETY: a successful localtime_r initialized the entire tm structure.
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

fn comment_card(
    id: u64,
    target: &str,
    body: &str,
    state: &str,
    found: bool,
    width: u16,
    theme: &Theme,
) -> Vec<Row> {
    card(
        theme,
        CardProps {
            fill: card_fill(found, theme),
            accent: theme.person("user"),
            head: vec![Span::styled(
                format!("Comment #{id} on {}", theme.clean(target)),
                theme.accent().patch(theme.title()),
            )],
            right: theme.clean(state),
            body,
            max_body: usize::MAX,
            width,
        },
    )
}

fn plan_rows(
    plan: &str,
    tasks: usize,
    text: &str,
    found: bool,
    width: u16,
    theme: &Theme,
) -> Vec<Row> {
    card(
        theme,
        CardProps {
            fill: card_fill(found, theme),
            accent: theme.accent(),
            head: vec![Span::styled(
                format!("Plan {}", theme.clean(plan)),
                theme.accent().patch(theme.title()),
            )],
            right: format!("{tasks} tasks"),
            body: text,
            max_body: usize::MAX,
            width,
        },
    )
}

fn system_rows(
    title: &str,
    meta: &str,
    text: &str,
    tone: &Tone,
    found: bool,
    width: u16,
    theme: &Theme,
) -> Vec<Row> {
    let accent = tone_style(tone, theme);
    card(
        theme,
        CardProps {
            fill: card_fill(found, theme),
            accent,
            head: vec![Span::styled(
                theme.clean(title),
                accent.patch(theme.title()),
            )],
            right: theme.clean(meta),
            body: text,
            max_body: usize::MAX,
            width,
        },
    )
}

fn tone_style(tone: &Tone, theme: &Theme) -> Style {
    match tone {
        Tone::Active => theme.accent(),
        Tone::Done => theme.status("completed"),
        Tone::Failed => theme.error(),
        Tone::Waiting => theme.warning(),
        Tone::Queued => theme.dim(),
    }
}

fn comment_body(text: &str, error: Option<&str>) -> String {
    match error {
        Some(error) => format!("{text}\nError: {error}"),
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn completion_card_renders_branch_summary_actions_and_raw_failure() {
        let mut view = View::default();
        view.apply(&json!({"seq":1,"kind":"session_completed","payload":{"branch":"antislop/session/x","summary":"Added the completion flow"}}));
        view.apply(&json!({"seq":2,"kind":"session_action_result","payload":{"action":"create_pr","result":{"status":"failed","details":"gh: authentication failed"}}}));
        let card = view.cards.last().unwrap();
        let rows = standard_card_rows(card, false, 120, &Theme::new(false, false));
        let text = rows
            .iter()
            .flat_map(|row| &row.line.spans)
            .map(|span| span.content.as_ref())
            .collect::<String>();
        for expected in [
            "antislop/session/x",
            "Added the completion flow",
            "Create PR",
            "Merge with main",
            "Continue work",
            "failed: gh: authentication failed",
            "Reports and retained evidence",
        ] {
            assert!(
                text.contains(expected),
                "missing {expected:?} from {text:?}"
            );
        }
    }

    #[test]
    fn every_card_kind_keeps_its_event_time_after_replay_and_at_narrow_widths() {
        let events = [
            ("message", json!({"author":"user","text":"Continue"})),
            (
                "plan_proposed",
                json!({"plan":"p","tasks":[{"id":"t1","title":"Task","change_kind":"code"},{"id":"t1-options","title":"Options","change_kind":"none"}]}),
            ),
            ("notice", json!({"text":"Preserve 👩‍💻e\u{301}"})),
            (
                "comment",
                json!({"id":1,"target":"plan:p","text":"Check scope","status":"open","label":"Plan","message":1}),
            ),
            (
                "task",
                json!({"id":"t1","task":"task","status":"needs_user","round":1}),
            ),
            (
                "task",
                json!({"id":"t1","task":"task","status":"approved","round":1,"feedback":{"round":1,"integration":{"commit_message":"fix: preserve result"}}}),
            ),
            (
                "task",
                json!({"id":"t1-options","task":"options","status":"approved","round":1,"feedback":{"round":1,"selection":{"approach":"Keep order","rationale":"Preserve work","tradeoffs":"One queue"}}}),
            ),
        ];
        let mut app = App::test_app();
        let mut replay = View::default();
        for (index, (kind, payload)) in events.into_iter().enumerate() {
            let sent_at = 1_700_000_000_000_i64 + index as i64 * 1000;
            let event = json!({"seq":index+1,"created_ms":sent_at,"kind":kind,"payload":payload});
            app.view.apply(&event);
            replay.apply(&event);
            assert_eq!(
                app.view.cards.last().unwrap().sent_at(),
                Some(sent_at),
                "{kind}"
            );
        }
        assert_eq!(app.view.cards, replay.cards);
        for card in &app.view.cards {
            let datetime = message_datetime(card.sent_at().unwrap()).unwrap();
            for width in [120, 50, 34, 12] {
                let rows = app.transcript_card_rows(card, false, width, &Theme::new(false, false));
                let text = rows
                    .iter()
                    .flat_map(|row| &row.line.spans)
                    .map(|span| span.content.as_ref())
                    .collect::<String>();
                let compact = text
                    .replace(['▌', '|'], "")
                    .split_whitespace()
                    .collect::<String>();
                assert!(
                    compact.contains(&datetime.replace(' ', "")),
                    "{width}: {text}"
                );
            }
        }
        for title in ["Progress", "Completed: Task", "Selected approach: Options"] {
            assert!(app.view.cards.iter().any(|card| matches!(card, TranscriptCard::System {title: actual, ..} if actual == title)), "{title}");
        }
        let before = app.view.cards.len();
        app.view.apply(
            &json!({"seq":100,"kind":"notice","payload":{"text":"Legacy event has no timestamp"}}),
        );
        assert_eq!(app.view.cards[before].sent_at(), None);
    }

    #[test]
    fn coalesced_progress_uses_its_last_update_time() {
        let mut view = View::default();
        view.tasks.push(view::TaskRow {
            id: "t1".into(),
            title: "Task".into(),
            ..Default::default()
        });
        for (seq, status, time) in [(1, "needs_user", 1000), (2, "waiting", 2000)] {
            view.apply(
                &json!({"seq":seq,"created_ms":time,"kind":"task","payload":{
                    "id":"t1","task":"task","round":1,"status":status
                }}),
            );
        }
        assert_eq!(view.cards.len(), 1);
        assert_eq!(view.cards[0].sent_at(), Some(2000));
    }

    #[test]
    fn the_live_phase_card_uses_the_workflow_event_time() {
        use ratatui::{Terminal, backend::TestBackend};
        let sent_at = 1_700_000_000_000_i64;
        let mut app = App::test_app();
        app.view.apply(&json!({"seq":1,"created_ms":sent_at,"kind":"workflow","payload":{"phase":"implementation"}}));
        assert_eq!(app.view.workflow_sent_at, Some(sent_at));
        let mut terminal = Terminal::new(TestBackend::new(70, 14)).unwrap();
        terminal
            .draw(|frame| app.draw_transcript(frame, frame.area(), &Theme::new(false, false)))
            .unwrap();
        let screen = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(
            screen.contains(&message_datetime(sent_at).unwrap()),
            "{screen}"
        );
        assert!(screen.contains("Phase 3/3"), "{screen}");
    }
}

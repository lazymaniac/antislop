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
        if let Some(comment) = comment_card_ref(entry) {
            return self.comment_rows(comment, found, width, theme);
        }
        standard_card_rows(entry, found, width, theme)
    }

    fn comment_rows(
        &self,
        comment: CommentCardRef<'_>,
        found: bool,
        width: u16,
        theme: &Theme,
    ) -> Vec<Row> {
        let state = self.comment_card_state(comment.id, comment.status);
        let body = comment_body(self, comment.id, comment.text);
        comment_card(
            comment.id,
            comment.target,
            &body,
            state,
            found,
            width,
            theme,
        )
    }

    fn comment_card_state<'a>(&'a self, id: u64, status: &'a str) -> &'a str {
        self.view
            .comments
            .get(&id)
            .map_or(status, |_| self.view.comment_state(id))
    }
}

fn comment_card_ref(entry: &TranscriptCard) -> Option<CommentCardRef<'_>> {
    let TranscriptCard::Comment {
        id,
        target,
        text,
        status,
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
        TranscriptCard::Plan { plan, tasks, text } => {
            plan_rows(plan, *tasks, text, found, width, theme)
        }
        TranscriptCard::System {
            title,
            meta,
            text,
            tone,
        } => system_rows(title, meta, text, tone, found, width, theme),
        TranscriptCard::Comment { .. } => unreachable!("comments handled before standard rows"),
        TranscriptCard::Confirmation(conf) => confirmation_rows(conf, found, width, theme),
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
    format!(
        "{} {} {}",
        theme.clean(&msg.state),
        theme.glyphs().dot,
        theme.clean(&msg.id)
    )
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

fn comment_body(app: &App, id: u64, text: &str) -> String {
    match app.view.comment_error(id) {
        Some(error) => format!("{text}\nError: {error}"),
        None => text.to_owned(),
    }
}

fn confirmation_rows(conf: &ConfirmationCard, found: bool, width: u16, theme: &Theme) -> Vec<Row> {
    let (accent, status) = confirmation_style(conf.status, theme);
    let head = confirmation_heading(conf, accent, theme);
    let body = confirmation_body(conf);
    card(
        theme,
        CardProps {
            fill: card_fill(found, theme),
            accent,
            head,
            right: status.into(),
            body: &body,
            max_body: usize::MAX,
            width,
        },
    )
}

fn confirmation_style(status: ConfirmationStatus, theme: &Theme) -> (Style, &'static str) {
    match status {
        ConfirmationStatus::Pending => (theme.warning(), "[awaiting confirmation]"),
        ConfirmationStatus::Confirmed => (theme.status("completed"), "[confirmed]"),
        ConfirmationStatus::Stale => (theme.error(), "[stale: stale_scope]"),
        ConfirmationStatus::Rejected => (theme.dim(), "[rejected]"),
    }
}

fn confirmation_heading(
    conf: &ConfirmationCard,
    accent: Style,
    theme: &Theme,
) -> Vec<Span<'static>> {
    vec![
        Span::styled("Confirmation: ", theme.faint()),
        Span::styled(theme.clean(&conf.title), accent.patch(theme.title())),
    ]
}

fn confirmation_body(conf: &ConfirmationCard) -> String {
    let mut body = conf.detail.clone();
    append_optional_line(&mut body, "Plan", conf.plan_id.as_deref());
    append_optional_line(&mut body, "Task", conf.task_id.as_deref());
    append_optional_line(&mut body, "Diff candidate", conf.candidate.as_deref());
    if !conf.scope_hash.is_empty() {
        append_optional_line(&mut body, "Scope (hash)", Some(&conf.scope_hash));
    }
    body
}

fn append_optional_line(body: &mut String, label: &str, value: Option<&str>) {
    if let Some(value) = value {
        body.push_str(&format!("\n{label}: {value}"));
    }
}

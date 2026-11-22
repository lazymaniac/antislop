//! One bottom dock for direction, editing, questions and contextual actions.
use super::super::*;

#[derive(Debug, Clone, Copy)]
pub(in crate::lean::tui) enum Action {
    Answer(usize),
    Review,
    Decisions,
    Session(usize),
}

fn question(app: &App) -> Option<&interview::Form> {
    let (event, _) = app.view.interview_question.as_ref()?;
    app.interview_forms
        .get(&app.conversation)
        .filter(|form| form.event == *event && !form.sent)
}

fn editing(app: &App) -> bool {
    app.interview_visible()
        || app.navigation.focus == NavigationFocus::Composer
        || app.details.as_ref().is_some_and(|details| details.editing)
}

fn extension_height(app: &App, viewport: Rect) -> u16 {
    if let Some(form) = question(app) {
        let preferred = if app.view.scope_confirmation_summary.is_some() {
            16
        } else if app.interview_visible() {
            (form.question.options.len() as u16 * 2 + 6).min(16)
        } else {
            form.question.options.len() as u16 + 4
        };
        preferred.min((viewport.height / 2).max(1))
    } else if !app.view.approvals.is_empty() {
        2
    } else if completion(app).is_some() && viewport.height >= 24 {
        4.min(viewport.height / 3)
    } else {
        0
    }
}

pub(in crate::lean::tui) fn footer_height(app: &App, viewport: Rect) -> u16 {
    let lines = if app.interview_visible() {
        question(app).map_or(1, |form| form.input.text.lines().take(5).count().max(1))
    } else if let Some(details) = app.details.as_ref().filter(|details| details.editing) {
        details
            .drafts
            .borrow()
            .get(&details.composer_key())
            .map_or(1, |draft| draft.input.text.lines().take(5).count().max(1))
    } else {
        app.input.text.lines().take(5).count().max(1)
    };
    let input = if editing(app) {
        lines as u16 + 2
    } else {
        if viewport.height < 20 { 1 } else { 2 }
    };
    (input + extension_height(app, viewport) + 1).min(viewport.height.saturating_sub(1).max(1))
}

pub(super) fn keybar(app: &mut App, frame: &mut Frame, area: Rect, theme: &Theme) -> bool {
    if !editing(app) {
        return false;
    }
    let keys = if app.interview_visible() {
        vec![
            ("enter answer", KeyCode::Enter, KeyModifiers::NONE),
            ("tab choices", KeyCode::Tab, KeyModifiers::NONE),
            ("PgDn review", KeyCode::PageDown, KeyModifiers::NONE),
            ("esc back", KeyCode::Esc, KeyModifiers::NONE),
            ("F1 help", KeyCode::F(1), KeyModifiers::NONE),
        ]
    } else {
        vec![
            ("enter send", KeyCode::Enter, KeyModifiers::NONE),
            ("alt enter newline", KeyCode::Enter, KeyModifiers::ALT),
            (
                "ctrl r recipient",
                KeyCode::Char('r'),
                KeyModifiers::CONTROL,
            ),
            ("ctrl t mode", KeyCode::Char('t'), KeyModifiers::CONTROL),
            ("esc back", KeyCode::Esc, KeyModifiers::NONE),
            ("F1 help", KeyCode::F(1), KeyModifiers::NONE),
        ]
    };
    super::chrome::keybar(app, frame, area, &keys, theme);
    true
}

fn completion(app: &App) -> Option<&[view::CompletionActionState]> {
    app.view.cards.iter().rev().find_map(|card| match card {
        view::TranscriptCard::Completion { actions, .. } => Some(actions.as_slice()),
        _ => None,
    })
}

pub(super) fn draw(app: &mut App, frame: &mut Frame, theme: &Theme) {
    if app.editor.is_some()
        || app.flow_editor.is_some()
        || app.plan_view.is_some()
        || app.direction_area.is_empty()
    {
        return;
    }
    let area = app.direction_area;
    let extra = extension_height(app, frame.area()).min(area.height.saturating_sub(2));
    let [extension, input_area] =
        Layout::vertical([Constraint::Length(extra), Constraint::Min(1)]).areas(area);
    draw_extension(app, frame, extension, theme);
    if !editing(app) {
        super::chrome::direction(
            frame,
            input_area,
            if matches!(app.navigation.target, WorkbenchTarget::Session)
                || app.navigation.screen == Screen::Graph
            {
                "session"
            } else {
                "task"
            },
            theme,
        );
        app.chrome_keys
            .push((input_area, KeyCode::Char('k'), KeyModifiers::CONTROL));
        return;
    }
    let answering = app.interview_visible();
    let agent = !answering && app.details.as_ref().is_some_and(|details| details.editing);
    let meta = u16::from(input_area.height >= 3);
    let inner = Rect::new(
        input_area.x + u16::from(input_area.width >= 6) * 2,
        input_area.y + meta,
        input_area
            .width
            .saturating_sub(u16::from(input_area.width >= 6) * 4),
        input_area.height.saturating_sub(meta),
    );
    if inner.is_empty() {
        return;
    }
    app.direction_input_area = inner;
    let wrap_width = usize::from(inner.width.saturating_sub(1)).max(1);
    let (input, recipient, mode, status) = if answering {
        let form = question(app).unwrap();
        (
            form.input.clone(),
            "Answer to antislop".to_owned(),
            String::new(),
            form.error.clone().unwrap_or_else(|| {
                if form.sending {
                    "Sending answer…".into()
                } else {
                    "Tab choices · type your own answer".into()
                }
            }),
        )
    } else if agent {
        let details = app.details.as_mut().unwrap();
        details.input_width[details.active_panel] = wrap_width;
        let key = details.composer_key();
        let composer = details
            .drafts
            .borrow()
            .get(&key)
            .cloned()
            .unwrap_or_default();
        (
            composer.input,
            composer.address.unwrap_or(key.1),
            composer.mode.name().to_owned(),
            composer.error.or(composer.delivery).unwrap_or_default(),
        )
    } else {
        app.input_width = wrap_width;
        let recipient = app
            .input
            .text
            .strip_prefix('@')
            .and_then(|rest| {
                rest.split_once(char::is_whitespace)
                    .map(|(recipient, _)| recipient)
            })
            .unwrap_or(ORCHESTRATOR)
            .to_owned();
        let images = app.image_drafts.get(&app.conversation).map_or(0, Vec::len);
        (
            app.input.clone(),
            recipient,
            app.navigation.session_delivery.name().to_owned(),
            app.failed_send
                .clone()
                .or_else(|| app.notice.clone())
                .unwrap_or_else(|| {
                    if images > 0 {
                        format!("{images} images attached")
                    } else {
                        String::new()
                    }
                }),
        )
    };
    frame.render_widget(Block::default().style(theme.raised()), input_area);
    if meta > 0 {
        let buttons = input_area.width >= 45;
        let reserve = if buttons { 20 } else { 0 };
        let label = if status.is_empty() {
            format!("To: {recipient} {} {mode}", theme.glyphs().dot)
        } else {
            status.clone()
        };
        frame.render_widget(
            Paragraph::new(theme.fit(
                &theme.clean(&label),
                usize::from(input_area.width.saturating_sub(4 + reserve)),
            ))
            .style(theme.dim()),
            Rect::new(
                input_area.x + 2,
                input_area.y,
                input_area.width.saturating_sub(4 + reserve),
                1,
            ),
        );
        if buttons {
            let send = Rect::new(input_area.right() - 19, input_area.y, 9, 1);
            let cancel = Rect::new(input_area.right() - 9, input_area.y, 7, 1);
            frame.render_widget(
                Paragraph::new(theme.chrome("Send ↵")).style(theme.accent()),
                send,
            );
            frame.render_widget(Paragraph::new("Cancel").style(theme.dim()), cancel);
            if app.picker.is_none() && app.help.is_none() && app.decision_inbox.is_none() {
                app.direction_actions = Some((send, cancel));
            }
        }
    }
    let (column, row) = input.visual_cursor(wrap_width);
    let first = row.saturating_sub(usize::from(inner.height.saturating_sub(1)));
    if !agent && !answering {
        app.input_shown = first;
    }
    let lines = visual_text_lines(&input.text, wrap_width)
        .into_iter()
        .skip(first)
        .take(usize::from(inner.height))
        .map(Line::raw)
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines).style(theme.text()), inner);
    if app.picker.is_none() && app.help.is_none() && app.decision_inbox.is_none() {
        frame.set_cursor_position((
            inner.x + column.min(inner.width.saturating_sub(1)),
            inner.y + (row - first) as u16,
        ));
    }
}

fn draw_extension(app: &mut App, frame: &mut Frame, area: Rect, theme: &Theme) {
    if area.is_empty() {
        return;
    }
    frame.render_widget(Block::default().style(theme.card()), area);
    if app.interview_visible()
        || (question(app).is_some() && app.view.scope_confirmation_summary.is_some())
    {
        if let Some(form) = app.interview_forms.get_mut(&app.conversation) {
            form.draw_inline(frame, area, theme);
            app.direction_choices.extend(
                form.choice_areas
                    .iter()
                    .map(|(rect, index)| (*rect, Action::Answer(*index))),
            );
            app.direction_choices.push((area, Action::Review));
        }
        return;
    }
    let mut choices = Vec::new();
    if let Some(form) = question(app) {
        choices.push(("QUESTION FOR YOU".to_owned(), None));
        choices.push((form.question.question.clone(), None));
        choices.extend(
            form.question
                .options
                .iter()
                .enumerate()
                .map(|(index, choice)| {
                    (
                        format!("{}. {}", index + 1, choice.label),
                        Some(Action::Answer(index)),
                    )
                }),
        );
        choices.push((
            theme.chrome("Other / own answer…"),
            Some(Action::Answer(form.question.options.len())),
        ));
    } else if !app.view.approvals.is_empty() {
        choices.push((
            format!("{} pending decision(s)", app.view.approvals.len()),
            None,
        ));
        choices.push((
            theme.chrome("Review available actions · d"),
            Some(Action::Decisions),
        ));
    } else if let Some(actions) = completion(app) {
        choices.push(("AVAILABLE ACTIONS".to_owned(), None));
        choices.extend(actions.iter().enumerate().map(|(index, action)| {
            let label = match action.kind {
                crate::lean::daemon::SessionActionKind::CreatePr => "Create PR",
                crate::lean::daemon::SessionActionKind::MergeWithMain => "Merge with main",
                crate::lean::daemon::SessionActionKind::ContinueWork => "Continue work",
            };
            let state = match &action.result {
                None => String::new(),
                Some(crate::lean::daemon::SessionActionResult::Queued { .. }) => "queued".into(),
                Some(crate::lean::daemon::SessionActionResult::Succeeded { summary }) => {
                    summary.clone()
                }
                Some(crate::lean::daemon::SessionActionResult::Failed { details }) => {
                    format!("failed: {details}")
                }
                Some(crate::lean::daemon::SessionActionResult::MissingInputs { inputs }) => {
                    format!("missing: {}", inputs.join(", "))
                }
            };
            let label = if state.is_empty() {
                label.to_owned()
            } else {
                format!("{label} {} {state}", theme.glyphs().dot)
            };
            (label, Some(Action::Session(index)))
        }));
    }
    for (index, (label, action)) in choices
        .into_iter()
        .take(usize::from(area.height))
        .enumerate()
    {
        let row = Rect::new(
            area.x + 1,
            area.y + index as u16,
            area.width.saturating_sub(2),
            1,
        );
        frame.render_widget(
            Paragraph::new(theme.fit(&theme.clean(&label), usize::from(row.width))).style(
                if action.is_some() {
                    theme.accent()
                } else {
                    theme.dim()
                },
            ),
            row,
        );
        if let Some(action) = action {
            app.direction_choices.push((row, action));
        }
    }
}

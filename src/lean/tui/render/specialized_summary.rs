use super::super::workbench_summary::{Completion, Recovery, Summary};
use super::super::*;

pub(super) fn draw(frame: &mut Frame, area: Rect, summary: Summary, scroll: &mut usize) -> bool {
    let title = match &summary {
        Summary::Recovery(_) => " Task status · recovery actions ",
        Summary::Completion(_) => " Session completion ",
        Summary::Empty => return false,
    };
    let theme = Theme::from_env();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.faint())
        .title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.is_empty() {
        return true;
    }
    let width = usize::from(inner.width.saturating_sub(1)).max(1);
    let lines = match &summary {
        Summary::Recovery(recovery) => recovery_lines(recovery, width, &theme),
        Summary::Completion(completion) => completion_lines(completion, width, &theme),
        Summary::Empty => return false,
    };
    let max_scroll = lines.len().saturating_sub(usize::from(inner.height));
    *scroll = (*scroll).min(max_scroll);
    let end = scroll
        .saturating_add(usize::from(inner.height))
        .min(lines.len());
    frame.render_widget(Paragraph::new(lines[*scroll..end].to_vec()), inner);
    super::specialized_details_layout::draw_record_scrollbar(
        frame,
        inner,
        lines.len(),
        *scroll,
        &theme,
    );
    true
}

fn recovery_lines(recovery: &Recovery, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(Span::styled(
        format!(
            "{} · {} · {} · F5 / u for actions",
            recovery.task,
            recovery.title,
            recovery.status.replace('_', " ")
        ),
        theme.title(),
    ))];
    lines.push(Line::raw("Available actions:"));
    lines.extend(
        recovery
            .choices
            .iter()
            .enumerate()
            .flat_map(|(index, choice)| {
                let text = format!(
                    "{}. {}{}",
                    index + 1,
                    choice.label,
                    if choice.asks_text {
                        " · opens a scoped draft"
                    } else {
                        ""
                    }
                );
                crate::lean::tui::view::wrap(&text, width)
                    .into_iter()
                    .map(Line::raw)
            }),
    );
    lines.push(Line::raw(""));
    lines.push(Line::raw("Raw stop reason:"));
    lines.extend(recovery.reason.split('\n').flat_map(|paragraph| {
        crate::lean::tui::view::wrap(paragraph, width)
            .into_iter()
            .map(Line::raw)
    }));
    lines
}

fn completion_lines(completion: &Completion, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let branch = completion
        .branch
        .as_deref()
        .map_or("Final branch: unavailable".to_owned(), |branch| {
            format!("Final branch: {branch}")
        });
    let mut lines = Vec::new();
    append_wrapped(&mut lines, &branch, width, "", Some(theme.title()));
    lines.push(Line::raw(""));
    for (index, action) in completion.actions.iter().enumerate() {
        let label = match action.kind {
            crate::lean::daemon::SessionActionKind::CreatePr => "Create PR",
            crate::lean::daemon::SessionActionKind::MergeWithMain => "Merge with main",
            crate::lean::daemon::SessionActionKind::ContinueWork => "Continue work",
        };
        let state = match &action.result {
            None => "available".to_owned(),
            Some(crate::lean::daemon::SessionActionResult::Queued { .. }) => {
                "queued · awaiting result".into()
            }
            Some(crate::lean::daemon::SessionActionResult::Succeeded { summary }) => {
                format!("succeeded: {summary}")
            }
            Some(crate::lean::daemon::SessionActionResult::Failed { details }) => {
                format!("failed: {details}")
            }
            Some(crate::lean::daemon::SessionActionResult::MissingInputs { inputs }) => {
                format!("missing inputs: {}", inputs.join(", "))
            }
        };
        let marker = if index == completion.selected {
            "▸"
        } else {
            " "
        };
        append_wrapped(
            &mut lines,
            &format!("{}. {label} · {state}", index + 1),
            width,
            marker,
            None,
        );
    }
    lines.push(Line::raw(""));
    lines.push(Line::raw("Completion result:"));
    lines.extend(
        crate::lean::tui::view::wrap(&completion.summary, width)
            .into_iter()
            .map(Line::raw),
    );
    if let Some(result) = &completion.latest_result {
        lines.push(Line::raw(""));
        append_wrapped(
            &mut lines,
            &format!("Latest action result: {result}"),
            width,
            "",
            None,
        );
    }
    lines.push(Line::raw(
        "Keys 1–3 run the matching action for this session.",
    ));
    lines
}

fn append_wrapped(
    lines: &mut Vec<Line<'static>>,
    text: &str,
    width: usize,
    prefix: &str,
    style: Option<Style>,
) {
    let prefix_width = unicode_width::UnicodeWidthStr::width(prefix);
    let continuation = " ".repeat(prefix_width);
    let wrapped = crate::lean::tui::view::wrap(text, width.saturating_sub(prefix_width).max(1));
    for (index, line) in wrapped.into_iter().enumerate() {
        let line = format!(
            "{}{}",
            if index == 0 { prefix } else { &continuation },
            line
        );
        lines.push(match style {
            Some(style) => Line::from(Span::styled(line, style)),
            None => Line::raw(line),
        });
    }
}

use super::super::workbench_summary::{Completion, Recovery, Summary};
use super::super::*;

#[derive(Debug, Clone, Default, PartialEq)]
pub(in crate::lean::tui) struct SummaryCache {
    key: String,
    projection: Option<Summary>,
    width: usize,
    mode: Option<(bool, bool)>,
    lines: Vec<Line<'static>>,
}

pub(super) fn draw(
    frame: &mut Frame,
    area: Rect,
    key: &str,
    view: &View,
    cache: &mut SummaryCache,
    scroll: &mut usize,
    theme: &Theme,
) -> bool {
    let changed = cache.key != key
        || !cache
            .projection
            .as_ref()
            .is_some_and(|summary| super::super::workbench_summary::unchanged(summary, view, key));
    if changed {
        cache.projection = Some(super::super::workbench_summary::project(view, key));
        cache.key = key.to_owned();
    }
    let summary = cache.projection.as_ref().unwrap();
    let title = match summary {
        Summary::Recovery(_) => " Task status · recovery actions ",
        Summary::Completion(_) => " Session completion ",
        Summary::Empty => return false,
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(theme.borders())
        .border_style(theme.faint())
        .title(theme.chrome(title));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.is_empty() {
        return true;
    }
    let width = usize::from(inner.width.saturating_sub(1)).max(1);
    if changed || cache.width != width || cache.mode != Some((theme.color, theme.ascii)) {
        cache.lines = match summary {
            Summary::Recovery(recovery) => recovery_lines(recovery, width, theme),
            Summary::Completion(completion) => completion_lines(completion, width, theme),
            Summary::Empty => return false,
        };
        cache.width = width;
        cache.mode = Some((theme.color, theme.ascii));
    }
    let lines = &cache.lines;
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
        theme,
    );
    true
}

fn recovery_lines(recovery: &Recovery, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let dot = theme.glyphs().dot;
    let mut lines = vec![Line::from(Span::styled(
        format!(
            "{} {dot} {} {dot} {} {dot} F5 / u for actions",
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
                        theme.chrome(" · opens a scoped draft")
                    } else {
                        String::new()
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
                theme.chrome("queued · awaiting result")
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
            theme.glyphs().pointer
        } else {
            " "
        };
        append_wrapped(
            &mut lines,
            &format!("{}. {label} {} {state}", index + 1, theme.glyphs().dot),
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
        theme.chrome("Keys 1–3 run the matching action for this session."),
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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn summary_reuses_full_raw_projection_and_wrap_until_inputs_or_geometry_change() {
        let mut app = App::test_app();
        app.navigation.screen = Screen::Workbench;
        app.navigation.target = WorkbenchTarget::Agent("worker:t1".into());
        app.navigation.update_presentation(|state| {
            state.tab = WorkbenchTab::Summary;
            state.split = false;
            state.list_visible = false;
        });
        app.details = Some(Details::new("worker:t1".into()));
        app.view.tasks.push(view::TaskRow {
            id: "t1".into(),
            status: "needs_user".into(),
            reason: Some(format!("{}END-OF-RAW-CAUSE", "👩‍💻 é ".repeat(10000))),
            ..Default::default()
        });
        let mut terminal = Terminal::new(TestBackend::new(80, 23)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let cache = &app.details.as_ref().unwrap().summary_cache;
        let Summary::Recovery(recovery) = cache.projection.as_ref().unwrap() else {
            panic!()
        };
        let original_reason = recovery.reason.as_ptr();
        let original_lines = cache.lines.as_ptr();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let cache = &app.details.as_ref().unwrap().summary_cache;
        let Summary::Recovery(recovery) = cache.projection.as_ref().unwrap() else {
            panic!()
        };
        assert_eq!(original_reason, recovery.reason.as_ptr());
        assert_eq!(original_lines, cache.lines.as_ptr());
        assert!(recovery.reason.ends_with("END-OF-RAW-CAUSE"));
        app.view.tasks[0].reason = Some("Changed raw reason 👩‍💻".into());
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let cache = &app.details.as_ref().unwrap().summary_cache;
        assert!(
            cache
                .lines
                .iter()
                .any(|line| line.to_string().contains("Changed raw reason"))
        );
        terminal.backend_mut().resize(40, 12);
        terminal.autoresize().unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let cache = &app.details.as_ref().unwrap().summary_cache;
        assert!(cache.width < 40);
        assert!(
            cache
                .lines
                .iter()
                .any(|line| line.to_string().contains("Changed raw reason"))
        );
    }
}

use super::super::super::super::theme::Theme;
use super::super::super::super::view::{TaskRow, View, clean};
use super::super::super::Health;
use super::super::super::health::{card_lines, fit_lines};
use ratatui::style::Style;

pub(super) fn status_style(theme: &Theme, health: Health, tick: u64) -> (String, Style) {
    let (icon, tone) = match health {
        Health::Working => (theme.spinner(tick), theme.status("working")),
        Health::Problem => (theme.glyphs().problem, theme.error()),
        Health::Waiting => (theme.glyphs().waiting, theme.warning()),
        Health::Done => (theme.glyphs().done, theme.status("approved")),
        Health::Idle => (theme.glyphs().queued, theme.dim()),
    };
    (icon.to_owned(), tone)
}

pub(super) fn card_title(name: &str, inner: usize, collapsed: bool) -> String {
    if collapsed {
        format!(
            "{} [+]",
            super::super::super::super::view::truncate(name, inner.saturating_sub(7))
        )
    } else {
        super::super::super::super::view::truncate(name, inner.saturating_sub(2))
    }
}

pub(super) fn summary_lines(
    view: &View,
    key: &str,
    task: Option<&TaskRow>,
    width: usize,
    max_lines: usize,
) -> Vec<String> {
    if let Some(task) = task.filter(|task| {
        task.verifier_summary
            .as_ref()
            .is_some_and(|summary| !clean(summary).trim().is_empty())
    }) {
        verifier_summary(task, width, max_lines)
    } else {
        card_lines(view, key, width, max_lines)
    }
}

fn verifier_summary(task: &TaskRow, width: usize, max_lines: usize) -> Vec<String> {
    let summary = task
        .summary
        .as_deref()
        .filter(|text| !clean(text).trim().is_empty())
        .unwrap_or(&task.title);
    let half = (max_lines / 2).max(1);
    let mut lines = fit_lines(&normalize(summary), width, half);
    lines.extend(fit_lines(
        &normalize(task.verifier_summary.as_deref().unwrap_or_default()),
        width,
        half,
    ));
    lines.truncate(max_lines);
    lines
}

fn normalize(text: &str) -> String {
    clean(text).split_whitespace().collect::<Vec<_>>().join(" ")
}

use super::*;
use crate::lean::tui::theme::Theme;

#[allow(clippy::too_many_arguments)]
pub fn diff_header_with_reviewer(
    task: &str,
    range: &str,
    file_count: usize,
    added: usize,
    removed: usize,
    truncated: bool,
    reviewer: Option<&ReviewerProfile>,
    width: usize,
) -> Line<'static> {
    diff_header_themed(
        task,
        range,
        file_count,
        added,
        removed,
        truncated,
        reviewer,
        width,
        &Theme::from_env(),
    )
}

#[allow(clippy::too_many_arguments)]
pub(in crate::lean::tui) fn diff_header_themed(
    task: &str,
    range: &str,
    file_count: usize,
    added: usize,
    removed: usize,
    truncated: bool,
    reviewer: Option<&ReviewerProfile>,
    width: usize,
    theme: &Theme,
) -> Line<'static> {
    render_diff_header(
        task,
        HeaderSummary {
            range,
            file_count,
            added,
            removed,
            truncated,
            reviewer,
            width,
            theme,
        },
    )
}

struct HeaderSummary<'a> {
    range: &'a str,
    file_count: usize,
    added: usize,
    removed: usize,
    truncated: bool,
    reviewer: Option<&'a ReviewerProfile>,
    width: usize,
    theme: &'a Theme,
}

fn render_diff_header(task: &str, summary: HeaderSummary<'_>) -> Line<'static> {
    let task = Span::styled(
        format!(" {task} diff "),
        Style::default().add_modifier(Modifier::BOLD),
    );
    let details = if summary.width >= 85 && !summary.range.is_empty() {
        range_detail(&summary, true)
    } else if summary.width >= 50 && !summary.range.is_empty() {
        range_detail(&summary, false)
    } else if summary.width >= 50 {
        count_detail(&summary)
    } else {
        summary
            .theme
            .chrome(&format!("+{} −{}", summary.added, summary.removed))
    };
    Line::from(vec![task, Span::styled(details, summary.theme.dim())])
}

fn range_detail(summary: &HeaderSummary<'_>, include_files: bool) -> String {
    let sep = summary.theme.glyphs().dot;
    let minus = if summary.theme.ascii { "-" } else { "−" };
    let files = if include_files {
        format!(" {sep} files: {}", summary.file_count)
    } else {
        String::new()
    };
    format!(
        "{}{files} {sep} +{} {minus}{}{}{}",
        summary.range,
        summary.added,
        summary.removed,
        summary.theme.chrome(truncation(summary.truncated)),
        reviewer_label(summary.reviewer, summary.theme)
    )
}

fn count_detail(summary: &HeaderSummary<'_>) -> String {
    let sep = summary.theme.glyphs().dot;
    let minus = if summary.theme.ascii { "-" } else { "−" };
    format!(
        "+{} {minus}{} {sep} {} file(s){}",
        summary.added,
        summary.removed,
        summary.file_count,
        truncation(summary.truncated)
    )
}

fn truncation(truncated: bool) -> &'static str {
    if truncated { " · truncated" } else { "" }
}

fn reviewer_label(reviewer: Option<&ReviewerProfile>, theme: &Theme) -> String {
    let sep = theme.glyphs().dot;
    match reviewer {
        Some(reviewer) => match &reviewer.model {
            Some(model) => format!(" {sep} reviewer: {} ({model})", reviewer.provider),
            None => format!(" {sep} reviewer: {}", reviewer.provider),
        },
        None => String::new(),
    }
}

pub fn diff_header(
    task: &str,
    range: &str,
    file_count: usize,
    added: usize,
    removed: usize,
    truncated: bool,
    width: usize,
) -> Line<'static> {
    diff_header_with_reviewer(
        task, range, file_count, added, removed, truncated, None, width,
    )
}

/// The widest hint that fits: each threshold is its hint's own width.
pub fn diff_footer_hint(width: usize) -> &'static str {
    if width >= 113 {
        "↑/↓ select · [/] file · Tab files/lines · f file list · c comment · R code review · r refresh · ? help · Esc back"
    } else if width >= 99 {
        "↑/↓ select · [/] file · Tab files/lines · c comment · R code review · r refresh · ? help · Esc back"
    } else if width >= 68 {
        "↑/↓ select · [/] file · Tab switch · c comment · R review · Esc back"
    } else if width >= 49 {
        "↑/↓ · [/] file · Tab · c comment · R review · Esc"
    } else if width >= 39 {
        "↑/↓ · [/] file · Tab · c com · Esc back"
    } else {
        "Tab · c · Esc"
    }
}

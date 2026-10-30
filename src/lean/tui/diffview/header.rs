use super::*;

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
        format!("+{} −{}", summary.added, summary.removed)
    };
    Line::from(vec![task, Span::styled(details, dim())])
}

fn range_detail(summary: &HeaderSummary<'_>, include_files: bool) -> String {
    let files = if include_files {
        format!(" · files: {}", summary.file_count)
    } else {
        String::new()
    };
    format!(
        "{}{files} · +{} −{}{}{}",
        summary.range,
        summary.added,
        summary.removed,
        truncation(summary.truncated),
        reviewer_label(summary.reviewer)
    )
}

fn count_detail(summary: &HeaderSummary<'_>) -> String {
    format!(
        "+{} −{} · {} file(s){}",
        summary.added,
        summary.removed,
        summary.file_count,
        truncation(summary.truncated)
    )
}

fn truncation(truncated: bool) -> &'static str {
    if truncated { " · truncated" } else { "" }
}

fn reviewer_label(reviewer: Option<&ReviewerProfile>) -> String {
    match reviewer {
        Some(reviewer) => match &reviewer.model {
            Some(model) => format!(" · reviewer: {} ({model})", reviewer.provider),
            None => format!(" · reviewer: {}", reviewer.provider),
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

pub fn diff_footer_hint(width: usize) -> &'static str {
    if width >= 95 {
        "↑/↓ select · ←/→ file · Tab files/lines · c comment · R code review · r refresh · ? help · Esc back"
    } else if width >= 65 {
        "↑/↓ select · ←/→ file · Tab switch · c com. · R review · r refresh · Esc back"
    } else if width >= 45 {
        "↑/↓ · ←/→ · Tab switch · c com · Esc back"
    } else {
        "Tab · c · Esc"
    }
}

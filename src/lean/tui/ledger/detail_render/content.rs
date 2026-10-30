use crate::lean::tui::{Theme, sanitize, view::wrap};
use ratatui::text::{Line, Span};

pub(super) fn overview_lines(
    entry: &crate::lean::ledger::LedgerEntry,
    theme: &Theme,
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    push_section(
        &mut lines,
        "Intent",
        &entry.spec.interpretation,
        theme,
        width,
    );
    let qualifiers = qualifiers_text(&entry.spec.qualifiers);
    push_section(
        &mut lines,
        "Constraints & qualifiers",
        &qualifiers,
        theme,
        width,
    );
    append_dependencies(&mut lines, entry, theme, width);
    append_source_reference(&mut lines, entry, theme, width);
    lines
}

pub(super) fn criteria_lines(
    entry: &crate::lean::ledger::LedgerEntry,
    theme: &Theme,
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    append_criteria(&mut lines, entry, theme, width);
    lines
}

pub(super) fn task_owner_lines(
    entry: &crate::lean::ledger::LedgerEntry,
    theme: &Theme,
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    push_section(
        &mut lines,
        "Assigned task IDs",
        &entry.spec.task_ids.join(", "),
        theme,
        width,
    );
    push_section(
        &mut lines,
        "Implementation owners",
        &entry.spec.implementation_owners.join(", "),
        theme,
        width,
    );
    push_section(
        &mut lines,
        "Verification owners",
        &entry.spec.verification_owners.join(", "),
        theme,
        width,
    );
    lines
}

pub(super) fn append_linked_tasks(
    lines: &mut Vec<Line<'static>>,
    tasks: &[serde_json::Value],
    theme: &Theme,
    width: usize,
) {
    if tasks.is_empty() {
        push_section(
            lines,
            "Linked tasks",
            "No task mapping yet. Tasks are connected when a verified plan is prepared.",
            theme,
            width,
        );
    }
    for task in tasks {
        append_task(lines, task, theme, width);
    }
}

pub(super) fn history_entries(
    history: &[serde_json::Value],
    theme: &Theme,
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for version in history {
        append_history_entry(&mut lines, version, theme, width);
    }
    lines
}

fn append_history_entry(
    lines: &mut Vec<Line<'static>>,
    version: &serde_json::Value,
    theme: &Theme,
    width: usize,
) {
    let title = format!(
        "Version {} · revision {}",
        version["version"], version["spec_revision"]
    );
    let text = version["spec"]["interpretation"].as_str().unwrap_or("");
    push_section(lines, &title, text, theme, width);
}

fn qualifiers_text(qualifiers: &[String]) -> String {
    if qualifiers.is_empty() {
        return "No additional qualifiers.".into();
    }
    qualifiers
        .iter()
        .map(|qualifier| format!("- {qualifier}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn append_dependencies(
    lines: &mut Vec<Line<'static>>,
    entry: &crate::lean::ledger::LedgerEntry,
    theme: &Theme,
    width: usize,
) {
    if !entry.spec.depends_on.is_empty() {
        push_section(
            lines,
            "Dependencies",
            &entry.spec.depends_on.join(", "),
            theme,
            width,
        );
    }
    push_section(lines, "Original wording", &entry.original, theme, width);
}

fn append_source_reference(
    lines: &mut Vec<Line<'static>>,
    entry: &crate::lean::ledger::LedgerEntry,
    theme: &Theme,
    width: usize,
) {
    let source = format!(
        "{} · bytes {}..{} · entry v{}",
        entry.spec.source.source_id, entry.spec.source.start, entry.spec.source.end, entry.version
    );
    push_section(lines, "Source reference", &source, theme, width);
}

fn append_criteria(
    lines: &mut Vec<Line<'static>>,
    entry: &crate::lean::ledger::LedgerEntry,
    theme: &Theme,
    width: usize,
) {
    if entry.spec.criteria.is_empty() {
        push_section(
            lines,
            "Acceptance criteria",
            "No criteria have been recorded yet.",
            theme,
            width,
        );
    }
    for criterion in &entry.spec.criteria {
        append_criterion(lines, criterion, theme, width);
    }
}

fn append_criterion(
    lines: &mut Vec<Line<'static>>,
    criterion: &crate::lean::ledger::Criterion,
    theme: &Theme,
    width: usize,
) {
    let required = if criterion.mandatory {
        "Required"
    } else {
        "Optional"
    };
    let integration = if criterion.integration {
        " · Integration"
    } else {
        ""
    };
    let title = format!("{} · {}{}", criterion.id, required, integration);
    push_section(lines, &title, &criterion.text, theme, width);
}

pub(super) fn append_task(
    lines: &mut Vec<Line<'static>>,
    task: &serde_json::Value,
    theme: &Theme,
    width: usize,
) {
    let title = format!(
        "{} · {}",
        task["title"].as_str().unwrap_or("Task"),
        task["status"].as_str().unwrap_or("pending")
    );
    let detail = format!(
        "{} · input v{} · generation {}",
        task["id"].as_str().unwrap_or(""),
        task["entry_version"],
        task["generation"]
    );
    push_section(lines, &title, &detail, theme, width);
}

pub(super) fn append_evidence(
    lines: &mut Vec<Line<'static>>,
    item: &serde_json::Value,
    theme: &Theme,
    width: usize,
) {
    let title = if item["approved"].as_bool() == Some(true) {
        "Verified evidence"
    } else {
        "Evidence needs review"
    };
    let detail = format!(
        "{}\n{}\n{}",
        item["proposal"]["criterion_id"].as_str().unwrap_or(""),
        item["proposal"]["result"].as_str().unwrap_or(""),
        item["assessment"].as_str().unwrap_or("")
    );
    push_section(lines, title, &detail, theme, width);
}

pub(super) fn section_lines(
    title: &str,
    text: &str,
    theme: &Theme,
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    push_section(&mut lines, title, text, theme, width);
    lines
}

pub(super) fn push_section(
    lines: &mut Vec<Line<'static>>,
    title: &str,
    text: &str,
    theme: &Theme,
    width: usize,
) {
    lines.push(Line::from(Span::styled(title.to_owned(), theme.title())));
    for line in text.lines() {
        lines.extend(wrap(&sanitize(line), width).into_iter().map(Line::from));
    }
    lines.push(Line::from(""));
}

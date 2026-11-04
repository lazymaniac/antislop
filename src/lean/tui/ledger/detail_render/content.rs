use crate::lean::tui::{Theme, sanitize, view::wrap};
use ratatui::text::{Line, Span};
use serde_json::Value;

pub(super) fn overview_lines(
    entry: &crate::lean::ledger::LedgerEntry,
    theme: &Theme,
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines = vec![badge_line(entry, theme, width), Line::from("")];
    push_section(
        &mut lines,
        "Intent",
        &entry.spec.interpretation,
        theme,
        width,
    );
    if !entry.spec.qualifiers.is_empty() {
        let qualifiers = qualifiers_text(&entry.spec.qualifiers);
        push_section(
            &mut lines,
            "Constraints & qualifiers",
            &qualifiers,
            theme,
            width,
        );
    }
    append_dependencies(&mut lines, entry, theme, width);
    lines
}

/// `R1 · required · active · identified`: what the record is, on one line.
fn badge_line(
    entry: &crate::lean::ledger::LedgerEntry,
    theme: &Theme,
    width: usize,
) -> Line<'static> {
    let kind = if entry.spec.mandatory {
        "required"
    } else {
        "optional"
    };
    let disposition = match entry.spec.disposition {
        crate::lean::ledger::Disposition::Active => "active",
        crate::lean::ledger::Disposition::SupersededByUser => "superseded by user",
    };
    let parent = entry
        .spec
        .parent
        .as_deref()
        .map(|parent| format!(" · under {parent}"))
        .unwrap_or_default();
    let text = theme.fit(
        &sanitize(&format!(
            "{} · {kind} · {disposition}{parent} · ",
            entry.spec.id
        )),
        width,
    );
    Line::from(vec![
        Span::styled(text, theme.dim()),
        Span::styled(
            format!("{} {}", theme.status_glyph(&entry.status), entry.status),
            theme.status(&entry.status),
        ),
    ])
}

/// Criteria with the evidence that settles each one directly beneath it.
pub(super) fn append_criteria_with_evidence(
    lines: &mut Vec<Line<'static>>,
    entry: &crate::lean::ledger::LedgerEntry,
    evidence: Option<&[Value]>,
    theme: &Theme,
    width: usize,
) {
    let criteria = &entry.spec.criteria;
    let reviewed = evidence.map(|items| {
        criteria
            .iter()
            .filter(|criterion| {
                items
                    .iter()
                    .any(|item| evidence_for(item, &criterion.id) && approved(item))
            })
            .count()
    });
    let heading = match reviewed {
        Some(reviewed) if !criteria.is_empty() => {
            format!(
                "Acceptance criteria · {reviewed}/{} reviewed",
                criteria.len()
            )
        }
        _ => "Acceptance criteria".into(),
    };
    lines.push(Line::from(Span::styled(heading, theme.title())));
    if criteria.is_empty() {
        lines.extend(
            wrap("No criteria have been recorded yet.", width)
                .into_iter()
                .map(|line| Line::styled(line, theme.dim())),
        );
    }
    for criterion in criteria {
        append_criterion(lines, criterion, evidence, theme, width);
    }
    lines.push(Line::from(""));
}

fn append_criterion(
    lines: &mut Vec<Line<'static>>,
    criterion: &crate::lean::ledger::Criterion,
    evidence: Option<&[Value]>,
    theme: &Theme,
    width: usize,
) {
    let matching: Vec<&Value> = evidence
        .unwrap_or_default()
        .iter()
        .filter(|item| evidence_for(item, &criterion.id))
        .collect();
    let mark = if matching.iter().any(|item| approved(item)) {
        theme.status_glyph("approved")
    } else if matching.is_empty() {
        theme.status_glyph("queued")
    } else {
        theme.status_glyph("reviewing")
    };
    let required = if criterion.mandatory {
        "required"
    } else {
        "optional"
    };
    let integration = if criterion.integration {
        " · integration"
    } else {
        ""
    };
    lines.push(Line::from(vec![
        Span::styled(format!("{mark} "), theme.accent()),
        Span::styled(
            sanitize(&format!("{} · {required}{integration}", criterion.id)),
            theme.accent(),
        ),
    ]));
    push_indented(lines, &criterion.text, theme.text(), width);
    for item in matching {
        push_indented(lines, &evidence_summary(item), theme.dim(), width);
    }
}

fn evidence_for(item: &Value, criterion: &str) -> bool {
    item["proposal"]["criterion_id"].as_str() == Some(criterion)
}

fn approved(item: &Value) -> bool {
    item["approved"].as_bool() == Some(true)
}

fn evidence_summary(item: &Value) -> String {
    let state = if approved(item) {
        "Evidence (reviewed)"
    } else {
        "Evidence (needs review)"
    };
    let parts = [
        item["proposal"]["result"].as_str(),
        item["proposal"]["artifact"].as_str(),
        item["assessment"].as_str(),
    ]
    .into_iter()
    .flatten()
    .map(str::trim)
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>();
    if parts.is_empty() {
        state.into()
    } else {
        format!("{state}: {}", parts.join(" · "))
    }
}

fn push_indented(
    lines: &mut Vec<Line<'static>>,
    text: &str,
    style: ratatui::style::Style,
    width: usize,
) {
    for line in text.lines() {
        lines.extend(
            wrap(&sanitize(line), width.saturating_sub(4).max(1))
                .into_iter()
                .map(|line| Line::styled(format!("    {line}"), style)),
        );
    }
}

/// Linked tasks, plus evidence that names no criterion of this record.
pub(super) fn append_coverage(
    lines: &mut Vec<Line<'static>>,
    entry: &crate::lean::ledger::LedgerEntry,
    tasks: Option<&[Value]>,
    evidence: Option<&[Value]>,
    theme: &Theme,
    width: usize,
) {
    lines.push(Line::from(Span::styled("Linked tasks", theme.title())));
    match tasks {
        None => lines.push(Line::styled("Loading coverage…", theme.dim())),
        Some([]) => lines.extend(
            wrap(
                "None yet. Tasks are linked when a reviewed plan is prepared.",
                width,
            )
            .into_iter()
            .map(|line| Line::styled(line, theme.dim())),
        ),
        Some(tasks) => {
            for task in tasks {
                append_task(lines, task, theme, width);
            }
        }
    }
    lines.push(Line::from(""));
    let other: Vec<&Value> = evidence
        .unwrap_or_default()
        .iter()
        .filter(|item| {
            !entry
                .spec
                .criteria
                .iter()
                .any(|criterion| evidence_for(item, &criterion.id))
        })
        .collect();
    if !other.is_empty() {
        lines.push(Line::from(Span::styled("Other evidence", theme.title())));
        for item in other {
            push_indented(lines, &evidence_summary(item), theme.dim(), width);
        }
        lines.push(Line::from(""));
    }
}

fn append_task(lines: &mut Vec<Line<'static>>, task: &Value, theme: &Theme, width: usize) {
    let id = task["task"]
        .as_str()
        .or_else(|| task["id"].as_str())
        .unwrap_or("");
    let title = task["title"].as_str().unwrap_or("Task");
    let status = task["status"].as_str().unwrap_or("pending");
    let heading = if id.is_empty() {
        title.to_owned()
    } else {
        format!("{id} · {title}")
    };
    let text = theme.fit(
        &sanitize(&format!("{heading} ")),
        width.saturating_sub(status.len() + 2),
    );
    lines.push(Line::from(vec![
        Span::styled(
            format!("{} ", theme.status_glyph(status)),
            theme.status(status),
        ),
        Span::styled(text, theme.text()),
        Span::styled(status.to_owned(), theme.status(status)),
    ]));
}

pub(super) fn history_entries(
    history: &[Value],
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
    version: &Value,
    theme: &Theme,
    width: usize,
) {
    let spec = &version["spec"];
    let kind = if spec["mandatory"].as_bool() == Some(false) {
        "optional"
    } else {
        "required"
    };
    let disposition = spec["disposition"].as_str().unwrap_or("active");
    let title = format!(
        "Version {} · revision {} · {kind} · {disposition}",
        version["version"], version["spec_revision"]
    );
    let body = history_entry_body(spec);
    push_section(lines, &title, &body, theme, width);
}

fn history_entry_body(spec: &Value) -> String {
    let mut parts = vec![spec["interpretation"].as_str().unwrap_or("").to_owned()];
    if let Some(qualifiers) = spec["qualifiers"].as_array().filter(|q| !q.is_empty()) {
        for q in qualifiers.iter().filter_map(|q| q.as_str()) {
            parts.push(format!("- {q}"));
        }
    }
    if let Some(criteria) = spec["criteria"].as_array().filter(|c| !c.is_empty()) {
        for c in criteria {
            let id = c["id"].as_str().unwrap_or("");
            let text = c["text"].as_str().unwrap_or("");
            parts.push(format!("{id}: {text}"));
        }
    }
    parts.join("\n")
}

fn qualifiers_text(qualifiers: &[String]) -> String {
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

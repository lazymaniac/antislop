use super::super::super::view::Comment;
use super::*;

pub(super) fn checks(details: &mut Details, view: &View, records: &mut Vec<Rc<DetailRecord>>) {
    let task = details
        .task_id()
        .and_then(|id| view.tasks.iter().find(|task| task.id == id));
    let task_id = details.task_id().unwrap_or_default().to_owned();
    match (&details.task, task) {
        (_, None) => records.push(record(vec![(dim(), "Tasks have checks: the goal, its acceptance criteria and its kind's risks, accounted for by the verifier. [no check data in history]".into())], " ")),
        (None, Some(_)) => records.push(record(vec![(dim(), "Loading…".into())], " ")),
        (Some(stored), Some(_)) => append_checks(&mut details.check, view, &task_id, stored, records),
    }
}

fn append_checks(
    cursor: &mut usize,
    view: &View,
    task_id: &str,
    stored: &Value,
    records: &mut Vec<Rc<DetailRecord>>,
) {
    let rows = check_rows(stored);
    *cursor = (*cursor).min(rows.len().saturating_sub(1));
    records.push(summary_record(stored, &rows));
    for (index, row) in rows.iter().enumerate() {
        records.push(check_record(view, task_id, row, index == *cursor));
    }
}

fn summary_record(stored: &Value, rows: &[CheckRow]) -> Rc<DetailRecord> {
    let settled = rows.iter().filter(|row| row.state == "settled").count();
    let issues = rows
        .iter()
        .filter(|row| matches!(row.status.as_deref(), Some("issue" | "blocked")))
        .count();
    let mut lines = vec![label(&format!(
        "{settled}/{} checks settled{}",
        rows.len(),
        issue_suffix(issues)
    ))];
    let now: Vec<&str> = rows
        .iter()
        .filter(|row| row.state == "checking")
        .map(|row| row.id.as_str())
        .collect();
    if !now.is_empty() {
        lines.push((
            Style::default().fg(crate::lean::tui::theme::ayu::BLUE),
            format!("Checking now: {}", now.join(", ")),
        ));
    }
    append_not_started(&mut lines, stored);
    lines.push((dim(), "↑/↓ select a check · c comment on it".into()));
    lines.push((Style::default(), String::new()));
    record(lines, " ")
}

fn append_not_started(lines: &mut Vec<(Style, String)>, stored: &Value) {
    if stored["verdict"].is_null() && stored["checks"].as_array().is_none_or(Vec::is_empty) {
        lines.push((dim(), "Verifier has not started yet; checks appear here as it settles them [no historical data]".into()));
    }
}

fn issue_suffix(issues: usize) -> String {
    if issues == 0 {
        String::new()
    } else {
        format!(" · {issues} with issues")
    }
}

fn check_record(view: &View, task_id: &str, row: &CheckRow, selected: bool) -> Rc<DetailRecord> {
    let style = if selected {
        row.style.add_modifier(Modifier::REVERSED)
    } else {
        row.style
    };
    let comments = view.check_comments(task_id, &row.id);
    let count = match comments.len() {
        0 => String::new(),
        1 => " [1 comment]".into(),
        count => format!(" [{count} comments]"),
    };
    let cursor = if selected { "▸" } else { " " };
    let mut lines = vec![(
        style,
        format!(
            "{cursor}{} {} {}: {}{count}",
            row.mark, row.id, row.kind, row.text
        ),
    )];
    lines.extend(evidence_lines(row));
    lines.extend(
        comments
            .into_iter()
            .map(|comment| comment_line(view, comment)),
    );
    record(lines, " ")
}

fn comment_line(view: &View, comment: &Comment) -> (Style, String) {
    (
        Style::default().fg(crate::lean::tui::theme::ayu::PURPLE),
        format!(
            "    💬 #{} {}: {}",
            comment.id,
            view.comment_state(comment.id),
            comment.text
        ),
    )
}

fn evidence_lines(row: &CheckRow) -> Vec<(Style, String)> {
    let line = match (&row.note, row.state) {
        (Some(note), _) => (Style::default(), format!("    evidence: {note}")),
        (None, "checking") => (dim(), "    checking… no evidence yet".into()),
        (None, "settled") => (
            dim(),
            "    settled without evidence [none given in history]".into(),
        ),
        (None, "unaccounted") => (dim(), "    unaccounted for [no evidence in history]".into()),
        _ => (dim(), "    not checked yet".into()),
    };
    vec![line]
}

fn label(text: &str) -> (Style, String) {
    (
        Style::default().add_modifier(Modifier::BOLD),
        text.to_owned(),
    )
}

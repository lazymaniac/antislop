use super::comparison::{list_diff_lines, readable, text_diff_lines};
use super::*;
use crate::lean::tui::{Theme, sanitize, view::wrap};
use ratatui::text::{Line, Span};
use serde_json::Value;

impl LedgerView {
    pub(super) fn preview_lines(&self, theme: &Theme, width: usize) -> Vec<Line<'static>> {
        let mut lines = preview_heading(theme);
        let Some(change) = &self.proposed_change else {
            return lines;
        };
        append_reason(&mut lines, &change.reason, width);
        let before = self
            .preview
            .as_ref()
            .and_then(|proposal| proposal["diff"]["before"].as_array());
        for entry in &change.entries {
            append_entry(&mut lines, entry, before, theme, width);
        }
        append_affected_tasks(&mut lines, self.preview.as_ref(), theme, width);
        lines
    }
}

fn preview_heading(theme: &Theme) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled("Review requirements changes", theme.title())),
        Line::from(""),
    ]
}

fn append_reason(lines: &mut Vec<Line<'static>>, reason: &str, width: usize) {
    lines.extend(
        wrap(&format!("Reason: {}", sanitize(reason)), width)
            .into_iter()
            .map(Line::from),
    );
    lines.push(Line::from(""));
}

fn append_entry(
    lines: &mut Vec<Line<'static>>,
    entry: &crate::lean::ledger::EntrySpec,
    before: Option<&Vec<Value>>,
    theme: &Theme,
    width: usize,
) {
    lines.push(Line::from(Span::styled(sanitize(&entry.id), theme.title())));
    let old = before
        .and_then(|entries| {
            entries
                .iter()
                .find(|value| value["spec"]["id"].as_str() == Some(&entry.id))
        })
        .map(|value| &value["spec"]);
    let value = serde_json::to_value(entry).unwrap_or_default();
    if let Some(fields) = value.as_object() {
        for (field, current) in fields {
            append_field(lines, field, current, old, theme, width);
        }
    }
}

fn append_field(
    lines: &mut Vec<Line<'static>>,
    field: &str,
    current: &Value,
    old: Option<&Value>,
    theme: &Theme,
    width: usize,
) {
    if field == "id" || old.and_then(|value| value.get(field)) == Some(current) {
        return;
    }
    lines.push(Line::from(Span::styled(
        field.replace('_', " "),
        theme.accent(),
    )));
    let before = old.and_then(|value| value.get(field));
    let prior_text = before.map(readable).unwrap_or_default();
    if current.is_array() {
        lines.extend(list_diff_lines(before, current, theme, width));
    } else {
        lines.extend(text_diff_lines(
            &prior_text,
            &readable(current),
            theme,
            width,
        ));
    }
    lines.push(Line::from(""));
}

fn append_affected_tasks(
    lines: &mut Vec<Line<'static>>,
    preview: Option<&Value>,
    theme: &Theme,
    width: usize,
) {
    let tasks = preview
        .map(|value| readable(&value["diff"]["affected_tasks"]))
        .unwrap_or_default();
    lines.push(Line::from(Span::styled("Affected tasks", theme.title())));
    lines.extend(wrap(&tasks, width).into_iter().map(Line::from));
}

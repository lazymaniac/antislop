use super::*;

/// One check of a verifier as the Checks tab shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckRow {
    pub id: String,
    pub kind: &'static str,
    pub text: String,
    pub mark: &'static str,
    pub style: Style,
    /// `pending`, `checking`, `settled` or `unaccounted`.
    pub state: &'static str,
    pub status: Option<String>,
    pub note: Option<String>,
}

pub fn check_rows(stored: &Value) -> Vec<CheckRow> {
    let Ok(spec) = serde_json::from_value::<TaskContract>(stored["spec"].clone()) else {
        return Vec::new();
    };
    let live = stored["checks"].as_array().filter(|rows| !rows.is_empty());
    let verdict = !stored["verdict"].is_null();
    obligations(&spec)
        .into_iter()
        .map(|(id, text)| make_check_row(id, text, stored, live, verdict))
        .collect()
}

fn make_check_row(
    id: String,
    text: String,
    stored: &Value,
    live: Option<&Vec<Value>>,
    verdict: bool,
) -> CheckRow {
    let (state, status, note) = check_state(&id, stored, live, verdict);
    let (mark, style) = check_mark(state, status.as_deref());
    let kind = match id.chars().next() {
        Some('R') => "risk",
        Some('V') => "common",
        _ => "check",
    };
    CheckRow {
        id,
        kind,
        text,
        mark,
        style,
        state,
        status,
        note,
    }
}

fn check_state(
    id: &str,
    stored: &Value,
    live: Option<&Vec<Value>>,
    verdict: bool,
) -> (&'static str, Option<String>, Option<String>) {
    if let Some(rows) = live {
        return rows
            .iter()
            .find(|row| row["id"] == id)
            .map_or(("pending", None, None), live_check_state);
    }
    let entry = &stored["verdict"]["coverage"][id];
    match entry["status"].as_str() {
        Some(status) => (
            "settled",
            Some(status.to_ascii_lowercase()),
            entry["note"].as_str().map(str::to_owned),
        ),
        None if verdict => ("unaccounted", None, None),
        None => ("pending", None, None),
    }
}

fn live_check_state(row: &Value) -> (&'static str, Option<String>, Option<String>) {
    let note = row["note"]
        .as_str()
        .filter(|note| !note.is_empty())
        .map(str::to_owned);
    if row["state"] == "settled" {
        (
            "settled",
            row["status"].as_str().map(str::to_ascii_lowercase),
            note,
        )
    } else {
        ("checking", None, note)
    }
}

fn check_mark(state: &str, status: Option<&str>) -> (&'static str, Style) {
    match (state, status) {
        ("checking", _) => ("◐", Style::default().fg(super::super::theme::ayu::BLUE)),
        (_, Some("ok")) => ("✓", Style::default().fg(super::super::theme::ayu::GREEN)),
        (_, Some("issue")) => ("✗", Style::default().fg(super::super::theme::ayu::RED)),
        (_, Some("blocked")) => ("!", Style::default().fg(super::super::theme::ayu::YELLOW)),
        (_, Some("n/a")) => ("–", dim()),
        ("unaccounted", _) => ("?", Style::default().fg(super::super::theme::ayu::YELLOW)),
        _ => ("○", dim()),
    }
}

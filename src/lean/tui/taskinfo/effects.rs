use super::super::super::tasks::{Outcome, UNKNOWN_ID_BYTES, UNKNOWN_ID_LIMIT, bound};
use super::super::view::TaskRow;
use serde_json::Value;

use super::super::super::tasks::status;
use super::one_line;

const NO_EFFECT_STATED: &str = "effect: not reported by worker";

/// The effect of a round as its two shared parts: the text collapsed to
/// one normalized line, and whether a bound cut left it a fragment.
/// `None` while the round has produced nothing to report at all. Every
/// effect shape a view shows composes itself from these parts, so what
/// counts as a fragment is decided exactly once.
///
/// Only explicit projection metadata marks a fragment. Text ending in an
/// ellipsis is ordinary user-visible text, never a control signal.
fn effect_pieces(row: &TaskRow) -> Option<(String, bool)> {
    let reported = row.effect.is_some()
        || row.obligations.is_some()
        || row.integration.is_some()
        || status::finished(&row.status);
    if !reported {
        return None;
    }
    let text = one_line(row.effect.as_deref().unwrap_or_default())
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let fragment = row.effect_cut == Some(true);
    Some((text, fragment))
}

/// From the shared parts to one "effect:" line. `front` says whether the
/// fragment marker leads the text — where a slot narrow enough to cut the
/// text again still shows it was broken off — or trails it whole.
fn effect_shape(row: &TaskRow, front: bool) -> Option<String> {
    let (text, fragment) = effect_pieces(row)?;
    if text.is_empty() {
        return Some(NO_EFFECT_STATED.to_owned());
    }
    Some(match (fragment, front) {
        (true, true) => format!("effect: [fragment] {text}"),
        (true, false) => format!("effect: {text} [fragment]"),
        _ => format!("effect: {text}"),
    })
}

/// The effect line a list or the details screen adds: `None` while the
/// round has produced nothing to report at all; the plain words the
/// contract asks for when a round ends without an effect, and the
/// fragment marker trailing the whole text.
pub fn effect_line(row: &TaskRow) -> Option<String> {
    effect_shape(row, false)
}

/// The same effect in the shape a card's fixed slot takes, with the
/// fragment marker in front of the text.
pub fn effect_front(row: &TaskRow) -> Option<String> {
    effect_shape(row, true)
}

/// Obligations the verdict settled over the task's real obligations.
/// `None` when no verdict has reported coverage: shown as `?`, never `0`.
pub fn settled_total(row: &TaskRow) -> Option<(u64, u64)> {
    settled_from(row.obligations.as_ref()?.as_object()?)
}

fn settled_from(coverage: &serde_json::Map<String, Value>) -> Option<(u64, u64)> {
    let total = coverage.get("total").and_then(Value::as_u64)?;
    let missing = coverage
        .get("missing")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    Some((total.saturating_sub(missing), total))
}

/// The one-glance coverage a card gets.
pub fn coverage_short(row: &TaskRow) -> String {
    match settled_total(row) {
        Some((settled, total)) => format!("{settled}/{total}"),
        None => "?".to_owned(),
    }
}

/// The whole coverage account a list or the details screen gets.
pub fn coverage_full(row: &TaskRow) -> String {
    let Some(coverage) = row.obligations.as_ref().and_then(Value::as_object) else {
        return "not reported".to_owned();
    };
    let counts =
        ["ok", "issue", "blocked", "na", "missing"].map(|key| coverage_count(coverage, key));
    let (settled, total) = settled_from(coverage).unwrap_or_default();
    let mut line = format!(
        "{settled} of {total} accounted · ok {} · issue {} · blocked {} · n/a {} · missing {}",
        counts[0], counts[1], counts[2], counts[3], counts[4]
    );
    append_unknown_ids(&mut line, coverage);
    line
}

fn coverage_count(coverage: &serde_json::Map<String, Value>, key: &str) -> u64 {
    coverage
        .get(key)
        .and_then(Value::as_u64)
        .unwrap_or_default()
}

fn append_unknown_ids(line: &mut String, coverage: &serde_json::Map<String, Value>) {
    let unknown = coverage.get("unknown").and_then(Value::as_array);
    let ids: Vec<&str> = unknown
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let more = coverage_count(coverage, "unknown_more");
    let shown: Vec<String> = ids
        .iter()
        .take(UNKNOWN_ID_LIMIT)
        .map(|id| bound(&one_line(id), UNKNOWN_ID_BYTES))
        .collect();
    let dropped = more + (ids.len() - shown.len()) as u64;
    append_unknown_summary(line, &shown, dropped);
}

fn append_unknown_summary(line: &mut String, shown: &[String], dropped: u64) {
    if !shown.is_empty() {
        let rest = if dropped > 0 {
            format!(" (+{dropped} more)")
        } else {
            String::new()
        };
        line.push_str(&format!(
            " · ids not in the plan: {}{rest}",
            shown.join(", ")
        ));
    } else if dropped > 0 {
        line.push_str(&format!(" · ids not in the plan: +{dropped} more"));
    }
}

/// The one label saying how the task's result landed, read through the
/// runtime's own outcome table. An outcome the daemon did not report (or
/// named wrongly) reads `not reported`, never a guessed landing.
pub fn integration_label(row: &TaskRow) -> String {
    row.integration
        .as_ref()
        .and_then(|integration| integration["outcome"].as_str())
        .and_then(Outcome::named)
        .map_or_else(
            || "not reported".to_owned(),
            |outcome| outcome.label().to_owned(),
        )
}

/// The whole landing line a card or the details screen shows: the one label
/// and the raw detail beside it.
pub fn integration_line(row: &TaskRow) -> String {
    format!("{} · {}", integration_label(row), integration_detail(row))
}

/// The raw landing detail, kept whole for lists and the details screen.
pub fn integration_detail(row: &TaskRow) -> String {
    row.integration
        .as_ref()
        .and_then(|integration| integration["detail"].as_str())
        .map(one_line)
        .filter(|detail| !detail.is_empty())
        .unwrap_or_else(|| "detail not reported".to_owned())
}

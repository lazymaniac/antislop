//! One interpretation of a task row, shared by the graph cards, the task
//! carousel and the details screen. Round, activity, cause, next step,
//! effect, obligation coverage and integration are read here and only here,
//! so the three views can never tell different stories about the same task.

use super::super::tasks::status;
use super::view::{TaskRow, View, clean, status_label};
use serde_json::Value;
use unicode_width::UnicodeWidthStr;

/// The round a card may show. `None` before the first round has started:
/// a task that never ran never claims a quiet `r1`.
pub fn round_shown(round: u64) -> Option<String> {
    (round >= 1).then(|| format!("r{round}"))
}

fn one_line(text: &str) -> String {
    clean(text)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Tasks this one waits on, by id: the named dependencies that are not
/// approved yet. An empty list on a queued task stays unexplained, because
/// the daemon reports a real reason when it has one and the frontend never
/// guesses a parallel limit or a write conflict.
pub fn waiting_on<'a>(view: &'a View, row: &TaskRow) -> Vec<&'a str> {
    view.tasks
        .iter()
        .filter(|parent| row.depends_on.iter().any(|dep| dep == &parent.id))
        .filter(|parent| parent.status != status::APPROVED)
        .map(|parent| parent.id.as_str())
        .collect()
}

/// Why the task stopped short of approval; `None` while it still runs.
/// A stop the daemon did not explain says so instead of inventing a cause.
pub fn cause(row: &TaskRow) -> Option<String> {
    if !matches!(
        row.status.as_str(),
        status::FAILED | status::NEEDS_USER | status::WAITING | status::SKIPPED
    ) {
        return None;
    }
    let reason = row.reason.as_deref().map(one_line).unwrap_or_default();
    Some(if reason.is_empty() {
        "cause not reported".to_owned()
    } else {
        reason
    })
}

/// The worker's state line: its round right after the state word once one
/// has started, the concrete dependencies it waits on, and the raw cause
/// when it stopped.
pub fn worker_state(view: &View, row: &TaskRow) -> String {
    let round = round_shown(row.round).map_or_default(|round| format!(" {round}"));
    match row.status.as_str() {
        status::QUEUED => {
            let waiting = waiting_on(view, row);
            if waiting.is_empty() {
                format!("queued{round}")
            } else {
                format!("queued{round} · waits for {}", waiting.join(", "))
            }
        }
        status::WORKING => format!("working{round}"),
        status::VERIFYING => format!("awaiting verdict{round}"),
        status::FAILED | status::NEEDS_USER | status::WAITING | status::SKIPPED => match cause(row)
        {
            Some(why) => format!("{}{round} · {why}", status_label(&row.status)),
            None => format!("{}{round}", status_label(&row.status)),
        },
        other => format!("{}{round}", status_label(other)),
    }
}

/// The verifier's state line, with the same round the worker shows.
pub fn verifier_state(row: &TaskRow) -> String {
    let round = round_shown(row.round).map_or_default(|round| format!(" {round}"));
    match row.status.as_str() {
        status::VERIFYING => format!("checking{round}"),
        status::FAILED | status::NEEDS_USER => match cause(row) {
            Some(why) => format!("rejected{round} · {why}"),
            None => format!("rejected{round}"),
        },
        status::PROPOSED | status::QUEUED | status::WORKING => {
            format!("waits for worker{round}")
        }
        other => format!("{}{round}", status_label(other)),
    }
}

/// The effect line a worker card may add under its summary: `None` while
/// the round has produced nothing to report at all; the plain words the
/// contract asks for when a round ends without an effect, and a fragment
/// marker when the bounded text was cut short.
pub fn effect_line(row: &TaskRow) -> Option<String> {
    let reported = row.effect.is_some()
        || row.obligations.is_some()
        || row.integration.is_some()
        || status::finished(&row.status);
    if !reported {
        return None;
    }
    let text = row.effect.as_deref().map(one_line).unwrap_or_default();
    Some(if text.is_empty() {
        "effect: not reported by worker".to_owned()
    } else if text.ends_with('…') {
        format!("effect: {text} [fragment]")
    } else {
        format!("effect: {text}")
    })
}

/// Obligations the verdict settled over the task's real obligations.
/// `None` when no verdict has reported coverage: shown as `?`, never `0`.
pub fn settled_total(row: &TaskRow) -> Option<(u64, u64)> {
    let coverage = row.obligations.as_ref()?;
    let total = coverage["total"].as_u64()?;
    let missing = coverage["missing"].as_u64().unwrap_or_default();
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
    match row.obligations.as_ref().and_then(Value::as_object) {
        None => "not reported".to_owned(),
        Some(coverage) => {
            let count = |key: &str| {
                coverage
                    .get(key)
                    .and_then(Value::as_u64)
                    .unwrap_or_default()
            };
            let settled = count("total").saturating_sub(count("missing"));
            let mut line = format!(
                "{settled} of {} accounted · ok {} · issue {} · blocked {} · n/a {} · missing {}",
                count("total"),
                count("ok"),
                count("issue"),
                count("blocked"),
                count("na"),
                count("missing"),
            );
            let unknown: Vec<&str> = coverage
                .get("unknown")
                .and_then(Value::as_array)
                .map(|ids| ids.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            if !unknown.is_empty() {
                line.push_str(&format!(" · ids not in the plan: {}", unknown.join(", ")));
            }
            line
        }
    }
}

/// The one label saying how the task's result landed. An outcome the daemon
/// did not report reads `not reported`, never a guessed landing.
pub fn integration_label(row: &TaskRow) -> String {
    let outcome = row
        .integration
        .as_ref()
        .and_then(|integration| integration["outcome"].as_str());
    let label = match outcome {
        Some("committed") => "committed",
        Some("applied") => "applied uncommitted",
        Some("in_place_committed") => "committed in place",
        Some("in_place_applied") => "applied in place",
        Some("no_changes") => "no changes",
        Some("conflict") => "conflict",
        Some("user_edits") => "user edits",
        Some("left_on_branch") => "on branch",
        Some("not_applicable") => "read-only none",
        Some("force_approved") => "force approved",
        Some("cancelled") => "cancelled",
        _ => return "not reported".to_owned(),
    };
    label.to_owned()
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

/// The bottom line of a task card: the verifier shows the coverage it
/// accounts for, both roles show how the result landed, concrete
/// dependencies and the count of messages follow. Parts that do not fit
/// the card drop from the right, so the line never overlaps a card edge.
pub fn card_meta(role: &str, view: &View, row: &TaskRow, width: usize) -> String {
    let mut parts: Vec<String> = Vec::new();
    if role == "verifier" {
        parts.push(coverage_short(row));
    }
    parts.push(integration_label(row));
    let waiting = waiting_on(view, row);
    if !waiting.is_empty() {
        parts.push(format!("D:{}", waiting.join("+")));
    }
    let mail: u32 = view
        .links
        .iter()
        .filter(|((from, to), _)| {
            from.ends_with(&format!(":{}", row.id)) || to.ends_with(&format!(":{}", row.id))
        })
        .map(|(_, link)| link.count)
        .sum();
    if mail > 0 {
        parts.push(format!("M{mail}"));
    }
    while parts.len() > 1 && UnicodeWidthStr::width(parts.join(" · ").as_str()) > width {
        parts.pop();
    }
    parts.join(" · ")
}

/// What the viewer can do next, naming only keys the frontend really
/// handles: Ctrl+A accepts a plan, Ctrl+R resumes a settled one, `u`
/// opens unblock, `c` comments, Enter opens details.
pub fn next_step(view: &View, row: &TaskRow) -> String {
    match row.status.as_str() {
        status::PROPOSED => "next: Ctrl+A accepts the plan".to_owned(),
        status::QUEUED => {
            let waiting = waiting_on(view, row);
            if waiting.is_empty() {
                "next: starts when its place in the queue comes; no reason reported".to_owned()
            } else {
                format!("next: waits for {}", waiting.join(", "))
            }
        }
        status::WORKING => "next: the worker is running; Enter opens its details".to_owned(),
        status::VERIFYING => "next: the verifier is checking; Enter opens its details".to_owned(),
        status::WAITING => {
            "next: finish the login or wait out the limit, then Ctrl+R resumes".to_owned()
        }
        status::FAILED | status::NEEDS_USER => {
            "next: u unblocks, c comments; Ctrl+R resumes once the plan settles".to_owned()
        }
        status::APPROVED => format!("done: {}", integration_label(row)),
        _ => "next: Ctrl+R resumes".to_owned(),
    }
}

/// Sums the plan's task states so a list can show every state's count;
/// states the plan does not know land in `other` and are never lost.
pub fn state_counts(view: &View) -> Vec<(&'static str, usize)> {
    let named = [
        status::PROPOSED,
        status::QUEUED,
        status::WORKING,
        status::VERIFYING,
        status::APPROVED,
        status::WAITING,
        status::NEEDS_USER,
        status::FAILED,
        status::SKIPPED,
        status::CANCELLED,
        status::REVISED,
    ];
    let mut counts: Vec<(&'static str, usize)> = named
        .iter()
        .map(|state| {
            (
                *state,
                view.tasks
                    .iter()
                    .filter(|task| task.status == *state)
                    .count(),
            )
        })
        .collect();
    let other = view
        .tasks
        .iter()
        .filter(|task| !named.contains(&task.status.as_str()))
        .count();
    if other > 0 {
        counts.push(("other", other));
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lean::tui::view::TaskRow;
    use serde_json::json;

    fn row(status: &str, round: u64) -> TaskRow {
        TaskRow {
            id: "t1".into(),
            status: status.into(),
            round,
            ..TaskRow::default()
        }
    }

    #[test]
    fn rounds_appear_only_once_work_reached_them() {
        assert_eq!(round_shown(0), None);
        assert_eq!(round_shown(1).as_deref(), Some("r1"));
        let worker = worker_state(&View::default(), &row("queued", 0));
        assert_eq!(worker, "queued");
        assert!(!worker.contains('r'));
        assert_eq!(
            worker_state(&View::default(), &row("working", 1)),
            "working r1"
        );
        assert_eq!(verifier_state(&row("working", 1)), "waits for worker r1");
        assert_eq!(verifier_state(&row("proposed", 0)), "waits for worker");
    }

    #[test]
    fn stops_name_the_cause_and_never_invent_one() {
        let mut failed = row("failed", 2);
        failed.reason = Some("conflicts with the current branch in src/a.rs".into());
        assert_eq!(
            worker_state(&View::default(), &failed),
            "failed r2 · conflicts with the current branch in src/a.rs"
        );
        let quiet = row("failed", 1);
        assert_eq!(
            worker_state(&View::default(), &quiet),
            "failed r1 · cause not reported"
        );
        // A queued task with no reported reason gets no invented excuse.
        let queued = worker_state(&View::default(), &row("queued", 0));
        assert!(!queued.contains("limit") && !queued.contains("conflict"));
    }

    #[test]
    fn coverage_is_counts_or_a_question_never_a_quiet_zero() {
        let bare = row("verifying", 1);
        assert_eq!(coverage_short(&bare), "?");
        assert_eq!(coverage_full(&bare), "not reported");
        let mut reported = row("verifying", 1);
        reported.obligations = Some(json!({
            "total": 8, "ok": 5, "issue": 1, "blocked": 0, "na": 0,
            "missing": 2, "unknown": ["X9"]
        }));
        assert_eq!(coverage_short(&reported), "6/8");
        let full = coverage_full(&reported);
        assert!(full.contains("6 of 8 accounted"), "{full}");
        assert!(full.contains("ids not in the plan: X9"), "{full}");
    }

    #[test]
    fn effects_mark_fragments_and_say_when_none_came() {
        assert_eq!(effect_line(&row("working", 1)), None);
        let mut running = row("working", 1);
        running.integration = Some(json!({"outcome": "applied"}));
        assert_eq!(
            effect_line(&running).as_deref(),
            Some("effect: not reported by worker")
        );
        let mut cut = row("approved", 1);
        cut.effect = Some("3 files changed — src/a.rs, src/b…".into());
        assert!(effect_line(&cut).unwrap().ends_with("[fragment]"));
    }

    #[test]
    fn integration_labels_are_the_reported_set() {
        for (outcome, label) in [
            ("committed", "committed"),
            ("applied", "applied uncommitted"),
            ("in_place_committed", "committed in place"),
            ("in_place_applied", "applied in place"),
            ("no_changes", "no changes"),
            ("conflict", "conflict"),
            ("user_edits", "user edits"),
            ("left_on_branch", "on branch"),
            ("not_applicable", "read-only none"),
        ] {
            let mut r = row("approved", 1);
            r.integration = Some(json!({"outcome": outcome}));
            assert_eq!(integration_label(&r), label);
        }
        assert_eq!(integration_label(&row("approved", 1)), "not reported");
        let mut odd = row("approved", 1);
        odd.integration = Some(json!({"outcome": "quantum-merge"}));
        assert_eq!(integration_label(&odd), "not reported");
    }

    #[test]
    fn next_steps_name_keys_the_frontend_handles() {
        assert!(next_step(&View::default(), &row("proposed", 0)).contains("Ctrl+A"));
        assert!(next_step(&View::default(), &row("waiting", 1)).contains("Ctrl+R"));
        assert!(next_step(&View::default(), &row("failed", 1)).contains("u unblocks"));
        let mut waiting = row("queued", 0);
        waiting.depends_on = vec!["t0".into()];
        let mut view = View::default();
        view.tasks = vec![
            TaskRow {
                id: "t0".into(),
                status: "working".into(),
                ..TaskRow::default()
            },
            waiting.clone(),
        ];
        assert!(
            next_step(&view, &waiting).contains("waits for t0"),
            "dependencies are named, not hinted at"
        );
    }

    #[test]
    fn card_meta_drops_the_least_important_part_first() {
        let mut view = View::default();
        let mut t1 = row("working", 2);
        t1.depends_on = vec!["t0".into()];
        view.tasks = vec![
            TaskRow {
                id: "t0".into(),
                status: "working".into(),
                ..TaskRow::default()
            },
            t1.clone(),
        ];
        let wide = card_meta("worker", &view, &t1, 40);
        assert!(
            wide.contains("D:t0") && wide.starts_with("not reported"),
            "{wide}"
        );
        let narrow = card_meta("worker", &view, &t1, 12);
        assert_eq!(narrow, "not reported", "deps give way, the label stays");
    }

    #[test]
    fn state_counts_cover_every_state() {
        let mut view = View::default();
        view.tasks = vec![
            row("approved", 1),
            row("working", 1),
            row("needs_user", 2),
            TaskRow {
                id: "t1".into(),
                status: "some-new-state".into(),
                ..TaskRow::default()
            },
        ];
        let counts = state_counts(&view);
        let total: usize = counts.iter().map(|(_, count)| count).sum();
        assert_eq!(total, view.tasks.len());
        assert!(
            counts
                .iter()
                .any(|(state, c)| *state == "needs_user" && *c == 1)
        );
        assert!(counts.iter().any(|(state, c)| *state == "other" && *c == 1));
    }
}

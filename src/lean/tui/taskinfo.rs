//! One interpretation of a task row, shared by the graph cards and the
//! details screen. Round, activity, cause, next step,
//! effect, obligation coverage and integration are read here and only here,
//! so both views can never tell different stories about the same task.

use super::super::tasks::{Outcome, UNKNOWN_ID_BYTES, UNKNOWN_ID_LIMIT, bound, status};
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

/// The worker's state word with its round, alone: what a card's state line
/// holds. The waiting reason and the stop cause get their own line
/// ([`worker_activity`]), so a long wait or a provider never crowds the
/// state out, and the state never has to repeat them.
pub fn worker_status(row: &TaskRow) -> String {
    let round = round_shown(row.round).map_or_default(|round| format!(" {round}"));
    match row.status.as_str() {
        status::VERIFYING => format!("awaiting verdict{round}"),
        other => format!("{}{round}", status_label(other)),
    }
}

/// The worker's state line: its round right after the state word once one
/// has started, the concrete dependencies it waits on, and the raw cause
/// when it stopped. It is the same reading the card splits over two lines,
/// word for word.
pub fn worker_state(view: &View, row: &TaskRow) -> String {
    let status = worker_status(row);
    let reason = match row.status.as_str() {
        status::QUEUED => {
            let waiting = waiting_on(view, row);
            (!waiting.is_empty()).then(|| format!("waits for {}", waiting.join(", ")))
        }
        status::FAILED | status::NEEDS_USER | status::WAITING | status::SKIPPED => cause(row),
        _ => None,
    };
    match reason {
        Some(reason) => format!("{status} · {reason}"),
        None => status,
    }
}

/// The verifier's state word with its round, alone, for a card's state line.
pub fn verifier_status(row: &TaskRow) -> String {
    let round = round_shown(row.round).map_or_default(|round| format!(" {round}"));
    match row.status.as_str() {
        status::VERIFYING => format!("checking{round}"),
        status::FAILED | status::NEEDS_USER => format!("rejected{round}"),
        status::PROPOSED | status::QUEUED | status::WORKING => format!("waits for worker{round}"),
        other => format!("{}{round}", status_label(other)),
    }
}

/// The verifier's state line, with the same round the worker shows.
pub fn verifier_state(row: &TaskRow) -> String {
    let status = verifier_status(row);
    match row.status.as_str() {
        status::FAILED | status::NEEDS_USER => match cause(row) {
            Some(why) => format!("{status} · {why}"),
            None => status,
        },
        _ => status,
    }
}

/// The provider beside a state, in the one shape every view shows it:
/// cards and details read the same state and add the same
/// provider, so no two views can drift apart.
pub fn with_provider(state: &str, provider: Option<&str>) -> String {
    match provider
        .map(str::trim)
        .filter(|provider| !provider.is_empty())
    {
        Some(provider) => format!("{state} · {provider}"),
        None => state.to_owned(),
    }
}

/// What one agent is really doing right now, cleaned to a single line:
/// only while its turn runs, and never the bare "starting turn" placeholder.
/// Every view that shows live activity asks here, so no view can show the
/// placeholder as work or carry activity its turn has already stopped.
pub fn doing(view: &View, key: &str) -> Option<String> {
    let agent = view.agents.get(key)?;
    if !agent.running {
        return None;
    }
    let text = clean(&agent.activity)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (!text.is_empty() && text != "starting turn").then_some(text)
}

/// Live activity for the task card follows the currently executing phase.
pub fn task_activity(view: &View, row: &TaskRow) -> Option<String> {
    if row.status == status::VERIFYING {
        verifier_activity(view, row)
    } else {
        worker_activity(view, row)
    }
}

/// The worker's own line under the state: what it is doing while it runs,
/// the concrete dependencies it waits for while queued, or the cause it
/// stopped with. Independent of the state word and the provider, and
/// `None` when there is nothing of the sort to say.
pub fn worker_activity(view: &View, row: &TaskRow) -> Option<String> {
    if row.status == status::WORKING
        && let Some(doing) = doing(view, &format!("worker:{}", row.id))
    {
        return Some(format!("doing: {doing}"));
    }
    match row.status.as_str() {
        status::QUEUED => {
            let waiting = waiting_on(view, row);
            (!waiting.is_empty()).then(|| format!("waits for {}", waiting.join(", ")))
        }
        status::FAILED | status::NEEDS_USER | status::WAITING | status::SKIPPED => cause(row),
        _ => None,
    }
}

/// The verifier's own line under its state, read the same way: what its
/// check is doing while it runs, or the cause a rejection stopped with.
pub fn verifier_activity(view: &View, row: &TaskRow) -> Option<String> {
    if row.status == status::VERIFYING
        && let Some(doing) = doing(view, &format!("verifier:{}", row.id))
    {
        return Some(format!("doing: {doing}"));
    }
    match row.status.as_str() {
        status::FAILED | status::NEEDS_USER => cause(row),
        _ => None,
    }
}

/// The one wording for a finished round that stated no effect.
const NO_EFFECT_STATED: &str = "effect: not reported by worker";

/// The effect of a round as its two shared parts: the text collapsed to
/// one normalized line, and whether a bound cut left it a fragment.
/// `None` while the round has produced nothing to report at all. Every
/// effect shape a view shows composes itself from these parts, so what
/// counts as a fragment is decided exactly once.
///
/// The fragment verdict reads the daemon's explicit `effect_cut` stamp —
/// recorded at the moment of the cut, and the only honest answer: honest
/// text may end in `…` and a kept whole oversized cluster carries no
/// mark. Only feedback that predates the stamp (`effect_cut: None`) falls
/// back to the trailing mark, the one signal such historical rows still
/// carry. Sanitizing the text always happens before that mark is read, so
/// a smuggled escape can neither pose as nor hide the cut.
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
    let fragment = match row.effect_cut {
        Some(cut) => cut,
        None => text.ends_with('…'),
    };
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
    match row.obligations.as_ref().and_then(Value::as_object) {
        None => "not reported".to_owned(),
        Some(coverage) => {
            let count = |key: &str| {
                coverage
                    .get(key)
                    .and_then(Value::as_u64)
                    .unwrap_or_default()
            };
            let (settled, total) = settled_from(coverage).unwrap_or_default();
            let mut line = format!(
                "{settled} of {total} accounted · ok {} · issue {} · blocked {} · n/a {} · missing {}",
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
            let more = coverage
                .get("unknown_more")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            // The preview lists at most [`UNKNOWN_ID_LIMIT`] ids, each a
            // sanitized line within [`UNKNOWN_ID_BYTES`] — the preview's
            // own bound. A feedback written before ids were bounded can
            // carry a longer or fatter list; the renderer trims it to the
            // same shape, and every id left out, by the runtime or here,
            // is counted, never silently dropped. The full ids stay in
            // the verdict, outside this compact account.
            let shown: Vec<String> = unknown
                .iter()
                .take(UNKNOWN_ID_LIMIT)
                .map(|id| bound(&one_line(id), UNKNOWN_ID_BYTES))
                .collect();
            let dropped = more + (unknown.len() - shown.len()) as u64;
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
            line
        }
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
    fn the_activity_line_is_its_own_and_the_state_line_its_own() {
        // A queued task waiting on a dependency: the state stays short and
        // the concrete wait gets its own line, provider-independent.
        let mut view = View::default();
        let mut parent = TaskRow {
            id: "t0".into(),
            status: "working".into(),
            ..TaskRow::default()
        };
        parent.summary = Some("Parent work.".into());
        let mut waiting = row("queued", 2);
        waiting.depends_on = vec!["t0".into()];
        view.tasks = vec![parent, waiting.clone()];
        assert_eq!(worker_status(&waiting), "queued r2");
        assert_eq!(
            worker_activity(&view, &waiting).as_deref(),
            Some("waits for t0")
        );
        assert_eq!(worker_state(&view, &waiting), "queued r2 · waits for t0");
        assert_eq!(
            with_provider(&worker_status(&waiting), Some("codex")),
            "queued r2 · codex"
        );
        assert_eq!(with_provider("queued r2", Some("")), "queued r2");
        // A running worker shows its activity as its own line; the verifier
        // never borrows it, and the state line never carries it.
        view.agents.insert(
            "worker:t1".into(),
            crate::lean::tui::view::Agent {
                running: true,
                provider: "codex".into(),
                activity: "reading  src/a.rs".into(),
                ..Default::default()
            },
        );
        let working = row("working", 1);
        assert_eq!(worker_status(&working), "working r1");
        assert_eq!(
            worker_activity(&view, &working).as_deref(),
            Some("doing: reading src/a.rs")
        );
        assert!(!worker_status(&working).contains("reading"));
        // Placeholder activity is no activity.
        view.agents.insert(
            "worker:t1".into(),
            crate::lean::tui::view::Agent {
                running: true,
                activity: "starting turn".into(),
                ..Default::default()
            },
        );
        assert_eq!(worker_activity(&view, &working), None);
        // The verifier's own line belongs to its own status and agent.
        let checking = row("verifying", 1);
        assert_eq!(verifier_status(&checking), "checking r1");
        assert_eq!(verifier_activity(&view, &checking), None);
        view.agents.insert(
            "verifier:t1".into(),
            crate::lean::tui::view::Agent {
                running: true,
                activity: "checking  the diff".into(),
                ..Default::default()
            },
        );
        assert_eq!(
            verifier_activity(&view, &checking).as_deref(),
            Some("doing: checking the diff")
        );
        // A stop puts the cause on the activity line, and the combined
        // reading stays word-for-word what the split says.
        let mut failed = row("failed", 2);
        failed.reason = Some("  it conflicted\n".into());
        assert_eq!(worker_status(&failed), "failed r2");
        assert_eq!(
            worker_activity(&view, &failed).as_deref(),
            Some("it conflicted")
        );
        assert_eq!(worker_state(&view, &failed), "failed r2 · it conflicted");
        assert_eq!(
            verifier_activity(&view, &failed).as_deref(),
            Some("it conflicted")
        );
        let quiet = row("needs_user", 1);
        assert_eq!(worker_status(&quiet), "needs decision r1");
        assert_eq!(
            worker_activity(&view, &quiet).as_deref(),
            Some("cause not reported")
        );
        // An idle approved task has nothing to put on the line, and says so
        // by staying empty rather than inventing filler.
        let done = row("approved", 1);
        assert_eq!(worker_activity(&view, &done), None);
        assert_eq!(verifier_activity(&view, &done), None);
        assert_eq!(worker_state(&view, &done), "approved r1");
    }

    #[test]
    fn live_activity_never_shows_the_placeholder_or_a_stopped_turn() {
        let mut view = View::default();
        view.agents.insert(
            "plan verifier".into(),
            crate::lean::tui::view::Agent {
                running: true,
                activity: "exec  two  words".into(),
                ..Default::default()
            },
        );
        assert_eq!(
            doing(&view, "plan verifier").as_deref(),
            Some("exec two words")
        );
        view.agents.insert(
            "auditor".into(),
            crate::lean::tui::view::Agent {
                running: true,
                activity: "starting turn".into(),
                ..Default::default()
            },
        );
        assert_eq!(
            doing(&view, "auditor"),
            None,
            "a placeholder is not activity"
        );
        view.agents.insert(
            "worker:t9".into(),
            crate::lean::tui::view::Agent {
                running: false,
                activity: "✗ failed: boom".into(),
                ..Default::default()
            },
        );
        assert_eq!(
            doing(&view, "worker:t9"),
            None,
            "a stopped turn is not live activity"
        );
        assert_eq!(doing(&view, "nobody"), None);
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
        assert_eq!(effect_front(&row("working", 1)), None);
        let mut running = row("working", 1);
        running.integration = Some(json!({"outcome": "applied"}));
        assert_eq!(
            effect_line(&running).as_deref(),
            Some("effect: not reported by worker")
        );
        assert_eq!(
            effect_front(&running).as_deref(),
            Some("effect: not reported by worker")
        );
        let mut cut = row("approved", 1);
        cut.effect = Some("3 files changed — src/a.rs, src/b…".into());
        assert!(effect_line(&cut).unwrap().ends_with("[fragment]"));
        // Both shapes come from the one reading: the same text and the one
        // fragment verdict, only the marker's seat differs.
        let trailing = effect_line(&cut).unwrap();
        let leading = effect_front(&cut).unwrap();
        let body = "3 files changed — src/a.rs, src/b…";
        assert_eq!(trailing, format!("effect: {body} [fragment]"));
        assert_eq!(leading, format!("effect: [fragment] {body}"));
        // The text is one normalized line for every view alike.
        let mut spaced = row("approved", 1);
        spaced.effect = Some("two  spaces\n\tand a\ttab".into());
        assert_eq!(
            effect_line(&spaced).as_deref(),
            Some("effect: two spaces and a tab")
        );
        assert_eq!(
            effect_front(&spaced).as_deref(),
            Some("effect: two spaces and a tab")
        );
    }

    #[test]
    fn the_fragment_verdict_reads_the_stamp_the_mark_is_only_the_fallback() {
        // An honest sentence ending in an ellipsis is no fragment once the
        // daemon's stamp says the stored text was never cut.
        let mut honest = row("approved", 1);
        honest.effect = Some("fixed the ellipsis bug in src/a.rs…".into());
        honest.effect_cut = Some(false);
        assert_eq!(
            effect_line(&honest).as_deref(),
            Some("effect: fixed the ellipsis bug in src/a.rs…"),
            "a natural ellipsis never marks a fragment on stamped feedback"
        );
        // A cut the stamp reports shows even when the kept whole carries
        // no trailing mark (an oversized first cluster kept whole).
        let mut stamped = row("approved", 1);
        stamped.effect = Some("👩‍👩‍👧‍👦".into());
        stamped.effect_cut = Some(true);
        assert_eq!(
            effect_front(&stamped).as_deref(),
            Some("effect: [fragment] 👩‍👩‍👧‍👦"),
            "the stamp decides, and the card puts it in front of the text"
        );
        // Historical feedback without the stamp falls back to the mark,
        // stated as the best the row allows.
        let mut legacy = row("approved", 1);
        legacy.effect = Some("3 files changed — src/a…".into());
        assert!(effect_line(&legacy).unwrap().ends_with("[fragment]"));
        let mut legacy_plain = row("approved", 1);
        legacy_plain.effect = Some("3 files changed — src/a.rs".into());
        assert!(!effect_line(&legacy_plain).unwrap().contains("[fragment]"));
    }

    #[test]
    fn coverage_render_caps_fat_ids_and_counts_every_one_left_out() {
        // The runtime's preview shape: few short ids plus an honest count
        // of the rest.
        let mut preview = row("verifying", 1);
        preview.obligations = Some(json!({
            "total": 3, "ok": 1, "missing": 2,
            "unknown": ["XX1", "XX2"], "unknown_more": 6,
        }));
        let full = coverage_full(&preview);
        assert!(
            full.contains("ids not in the plan: XX1, XX2 (+6 more)"),
            "{full}"
        );
        // An old replay from before ids were bounded: an oversized list of
        // oversized ids. The renderer lists at most the preview's share,
        // bounds each id, and counts the rest — the true total stands.
        let fat: Vec<String> = (0..12)
            .map(|i| format!("x{i}{}", "y".repeat(200)))
            .collect();
        let mut replay = row("verifying", 1);
        replay.obligations = Some(json!({
            "total": 2, "ok": 0, "missing": 2, "unknown": fat,
        }));
        let full = coverage_full(&replay);
        let listed = full
            .split("ids not in the plan: ")
            .nth(1)
            .expect("ids named")
            .split(" (")
            .next()
            .expect("listing");
        let ids: Vec<&str> = listed.split(", ").collect();
        assert_eq!(ids.len(), UNKNOWN_ID_LIMIT, "{full}");
        assert!(
            ids.iter().all(|id| id.len() <= UNKNOWN_ID_BYTES),
            "every listed id is within the preview bound: {ids:?}"
        );
        assert!(
            full.contains(&format!("(+{} more)", 12 - UNKNOWN_ID_LIMIT)),
            "the ids left out are counted, not lost: {full}"
        );
        assert!(full.starts_with("0 of 2 accounted"), "{full}");
    }

    #[test]
    fn integration_labels_are_the_reported_set() {
        for (outcome, label) in [
            ("committed", "committed"),
            ("applied", "applied uncommitted"),
            ("in_place_committed", "committed in place"),
            ("in_place_applied", "applied in place"),
            ("in_place_user_edits", "held for your edits"),
            ("no_changes", "no changes"),
            ("conflict", "conflict"),
            ("user_edits", "user edits"),
            ("left_on_branch", "on branch"),
            ("not_applicable", "read-only none"),
            ("force_approved", "force approved"),
            ("cancelled", "cancelled"),
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
    fn the_landing_line_pairs_label_with_the_whole_detail() {
        let mut r = row("approved", 2);
        r.integration = Some(json!({
            "outcome": "in_place_committed",
            "detail": "committed in place as deadbee",
        }));
        assert_eq!(
            integration_line(&r),
            "committed in place · committed in place as deadbee"
        );
        assert_eq!(
            integration_line(&row("working", 1)),
            "not reported · detail not reported"
        );
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
}

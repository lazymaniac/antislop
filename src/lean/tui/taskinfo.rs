//! One interpretation of a task row, shared by the graph cards and the
//! details screen. Round, activity, cause, next step,
//! effect, obligation coverage and integration are read here and only here,
//! so both views can never tell different stories about the same task.

use super::super::tasks::status;
use super::view::{TaskRow, View, clean, status_label};
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
mod effects;
pub use effects::{
    coverage_full, coverage_short, effect_front, effect_line, integration_detail,
    integration_label, integration_line, settled_total,
};

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
#[cfg(test)]
#[path = "taskinfo/tests.rs"]
mod tests;

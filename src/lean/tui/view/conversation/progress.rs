use super::super::*;

pub(super) fn is_running(status: &str) -> bool {
    matches!(status, "queued" | "working" | "verifying")
}

pub(super) fn progress_line(
    view: &View,
    id: &str,
    title: &str,
    was: (&str, u64),
    now: (&str, u64),
    reason: Option<String>,
) -> Option<(String, crate::lean::tui::theme::Tone)> {
    if was == now {
        return None;
    }
    let name = if title.is_empty() {
        id.to_owned()
    } else {
        format!("{id} {title}")
    };
    let why = reason
        .map(|text| format!(" — {}", clean(&text).replace('\n', " ")))
        .unwrap_or_default();
    let done = view
        .tasks
        .iter()
        .filter(|task| task.status == "approved")
        .count();
    progress_status(&name, why, now, was.1, done, view.tasks.len())
}

fn progress_status(
    name: &str,
    why: String,
    now: (&str, u64),
    previous_round: u64,
    done: usize,
    total: usize,
) -> Option<(String, crate::lean::tui::theme::Tone)> {
    use crate::lean::tui::theme::Tone;
    Some(match now.0 {
        "approved" => (format!("✓ {name} approved ({done}/{total})"), Tone::Done),
        "failed" => (format!("✗ {name} failed{why}"), Tone::Failed),
        "coordinating" => (format!("! {name} awaits orchestration{why}"), Tone::Waiting),
        "needs_user" => (format!("! {name} needs your decision{why}"), Tone::Waiting),
        "waiting" => (format!("‖ {name} waits{why}"), Tone::Waiting),
        "skipped" => (format!("– {name} skipped{why}"), Tone::Queued),
        "working" if now.1 > previous_round && previous_round > 0 => (
            format!(
                "↺ {name}: round {} after the verifier asked for changes",
                now.1
            ),
            Tone::Active,
        ),
        _ => return None,
    })
}

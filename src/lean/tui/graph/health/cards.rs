use super::*;

/// The card's activity line: what this role is doing, the dependencies it
/// waits for, or the cause it stopped with — one line of its own, shared
/// with details through taskinfo, and independent of the
/// state word and the provider.
pub(in crate::lean::tui::graph) fn activity_line(view: &View, key: &str) -> Option<String> {
    let (_, id) = key.split_once(':')?;
    let task = view.tasks.iter().find(|task| task.id == id)?;
    taskinfo::task_activity(view, task)
}

/// The same bounded, grapheme-safe text drives both layout and rendering.
pub(in crate::lean::tui::graph) fn card_lines(
    view: &View,
    key: &str,
    width: usize,
    max: usize,
) -> Vec<String> {
    let Some(text) = card_text(view, key) else {
        let name = clean(&node_name(key))
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        return fit_lines(&name, width, 1);
    };
    fit_lines(&text, width, max)
}

fn card_text(view: &View, key: &str) -> Option<String> {
    match key {
        ORCHESTRATOR => Some("Agrees requirements with you and steers the team.".to_owned()),
        crate::lean::PLANNER => Some("Turns agreed requirements into a task plan.".to_owned()),
        PLAN_VERIFIER => Some("Independently checks the plan before you see it.".to_owned()),
        AUDITOR => Some("Checks the finished work delivers your request.".to_owned()),
        crate::lean::COMMENT_ORCHESTRATOR => {
            Some("Answers your comments and acts on them.".to_owned())
        }
        _ if requirements_round(key).is_some() => {
            Some("Extracts the requirements and proposes options.".to_owned())
        }
        _ => task_card_text(view, key),
    }
}

fn task_card_text(view: &View, key: &str) -> Option<String> {
    let normalize = |text: &str| clean(text).split_whitespace().collect::<Vec<_>>().join(" ");
    let (role, task) = key.split_once(':').and_then(|(role, id)| {
        view.tasks
            .iter()
            .find(|task| task.id == id)
            .map(|task| (role, task))
    })?;
    let summary = if role == "worker" {
        &task.summary
    } else {
        &task.verifier_summary
    };
    Some(
        summary
            .as_deref()
            .map(normalize)
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| task_fallback(role, task, &normalize)),
    )
}

fn task_fallback(role: &str, task: &TaskRow, normalize: &impl Fn(&str) -> String) -> String {
    let label = [&task.title, &task.kind, &task.id]
        .into_iter()
        .map(|text| normalize(text))
        .find(|text| !text.is_empty())
        .unwrap_or_else(|| "task".to_owned());
    if role == "worker" {
        label
    } else {
        format!("Verify {label}.")
    }
}

/// The same bounded, grapheme-safe wrapping the summary uses: at most
/// `max` lines of `width` columns, the cut line forced to end in `…`.
pub(in crate::lean::tui::graph) fn fit_lines(text: &str, width: usize, max: usize) -> Vec<String> {
    if width == 0 || max == 0 {
        return Vec::new();
    }
    let mut lines = wrap(text, width);
    if lines.len() > max {
        lines.truncate(max);
        let last = lines
            .last_mut()
            .expect("a wrapped text has at least one line");
        // Force an ellipsis even when the final visible line has spare columns.
        let mut used = 0;
        let prefix: String = last
            .trim_end()
            .graphemes(true)
            .take_while(|g| {
                used += g.width();
                used < width
            })
            .collect();
        *last = format!("{prefix}…");
    }
    lines
}

/// The worker's effect in its own fixed slot under the summary: never a
/// substitute for a summary line, never a growth of the card. The card's
/// shape of the shared reading (taskinfo) — text and fragment marker in
/// front — wrapped to the slot; `None` keeps the slot empty until the
/// round has something to report at all.
pub(in crate::lean::tui::graph) fn effect_lines(
    view: &View,
    key: &str,
    width: usize,
    max: usize,
) -> Vec<String> {
    let Some(row) = key.split_once(':').and_then(|(role, id)| {
        (role == "worker")
            .then(|| view.tasks.iter().find(|task| task.id == id))
            .flatten()
    }) else {
        return Vec::new();
    };
    let Some(effect) = taskinfo::effect_front(row) else {
        return Vec::new();
    };
    fit_lines(&effect, width, max)
}

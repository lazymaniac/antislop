//! One content measurement shared by graph placement and card painting.
use super::health::{activity_line, card_lines, effect_lines};
use super::*;

#[derive(Clone, Copy)]
pub(super) enum Tone {
    Text,
    Dim,
    Status,
    Activity,
}

pub(super) struct Content {
    pub(super) rows: Vec<(Tone, String)>,
}

impl Content {
    pub(super) fn new(view: &View, key: &str, width: u16, limit: u16, folded: bool) -> Self {
        let inner = usize::from(width.saturating_sub(4));
        let mut rows = Vec::new();
        if let Some(text) = verifier_text(view, key) {
            rows.push((Tone::Status, text));
        }
        if folded {
            return Self { rows };
        }
        let branch = branch(view, key);
        if task_card(view, key).is_some() {
            if let Some(text) = activity_line(view, key) {
                rows.push((Tone::Activity, text));
            }
            if let Some(branch) = &branch {
                rows.push((Tone::Dim, format!("branch: {branch}")));
            }
        }
        let metadata = task_card(view, key)
            .map(|(task, _)| {
                let role = key.split_once(':').map_or("", |(role, _)| role);
                taskinfo::card_meta(role, view, task, inner)
            })
            .filter(|text| !text.is_empty());
        let room =
            usize::from(limit).saturating_sub(2 + rows.len() + usize::from(metadata.is_some()));
        let effects = effect_lines(
            view,
            key,
            inner,
            MAX_EFFECT_LINES.min(room.saturating_sub(1)),
        );
        let summary_max = if is_admin(key) {
            ADMIN_SUMMARY_LINES
        } else {
            MAX_SUMMARY_LINES
        };
        let summary_room =
            room.saturating_sub(effects.len() + usize::from(is_admin(key) && branch.is_some()));
        rows.extend(
            card_lines(view, key, inner, summary_max.min(summary_room))
                .into_iter()
                .map(|text| (Tone::Text, text)),
        );
        rows.extend(effects.into_iter().map(|text| (Tone::Text, text)));
        if is_admin(key)
            && let Some(branch) = branch
        {
            rows.push((Tone::Dim, format!("branch: {branch}")));
        }
        if let Some(text) = metadata {
            rows.push((Tone::Text, text));
        }
        Self { rows }
    }

    pub(super) fn height(&self) -> u16 {
        2 + self.rows.len() as u16
    }
}

pub(super) fn height(view: &View, key: &str, width: u16, limit: u16, folded: bool) -> u16 {
    Content::new(view, key, width, limit, folded).height()
}

pub(super) fn pair_finished(task: &TaskRow, role: Role) -> bool {
    role == Role::Pair
        && matches!(
            task.status.as_str(),
            "approved" | "failed" | "skipped" | "cancelled" | "revised"
        )
}

pub(super) fn verifier_text(view: &View, key: &str) -> Option<String> {
    let (task, role) = task_card(view, key)?;
    if pair_finished(task, role) {
        return None;
    }
    let coverage = task
        .obligations
        .as_ref()
        .map(|_| taskinfo::coverage_short(task));
    match (role, coverage) {
        (Role::Pair, coverage) => Some(format!(
            "V: {}{}",
            taskinfo::verifier_status(task),
            coverage.map_or_default(|coverage| format!(" · {coverage}"))
        )),
        (Role::Verifier, Some(coverage)) => Some(format!("coverage {coverage}")),
        _ => None,
    }
}

fn branch(view: &View, key: &str) -> Option<String> {
    let agent = match task_card(view, key) {
        Some((task, Role::Pair)) => format!(
            "{}:{}",
            if task.status == "verifying" {
                "verifier"
            } else {
                "worker"
            },
            task.id
        ),
        Some(_) => key.to_owned(),
        None => current_round(view, &card_members(view, key))
            .unwrap_or(key)
            .to_owned(),
    };
    view.agents
        .get(&agent)
        .and_then(|agent| agent.branch.clone())
}

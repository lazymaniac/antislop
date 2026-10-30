use super::*;
use crate::lean::tui::view::TaskRow;

/// How an agent is doing, which decides its icon and color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Idle,
    Working,
    Waiting,
    Problem,
    Done,
}

pub fn health(view: &View, key: &str) -> Health {
    key.split_once(':')
        .map_or_else(|| admin_health(view, key), |(_, id)| task_health(view, id))
}

fn task_health(view: &View, id: &str) -> Health {
    let Some(task) = view.tasks.iter().find(|task| task.id == id) else {
        return Health::Idle;
    };
    if task_problem(view, task, id) {
        return Health::Problem;
    }
    if task_running(view, id) {
        return Health::Working;
    }
    match task.status.as_str() {
        "waiting" => Health::Waiting,
        "approved" => Health::Done,
        "working" | "verifying" => Health::Working,
        _ => Health::Idle,
    }
}

fn task_problem(view: &View, task: &TaskRow, id: &str) -> bool {
    matches!(
        task.status.as_str(),
        "failed" | "coordinating" | "needs_user"
    ) || ["worker", "verifier"].iter().any(|role| {
        view.agents
            .get(&format!("{role}:{id}"))
            .is_some_and(|agent| agent.activity.starts_with('✗') || agent.activity.starts_with('⚠'))
    })
}

fn task_running(view: &View, id: &str) -> bool {
    ["worker", "verifier"].iter().any(|role| {
        view.agents
            .get(&format!("{role}:{id}"))
            .is_some_and(|agent| agent.running)
    })
}

fn admin_health(view: &View, key: &str) -> Health {
    let members = card_members(view, key);
    let agents: Vec<_> = members
        .iter()
        .filter_map(|member| view.agents.get(*member))
        .collect();
    if agents.iter().any(|agent| agent.running) {
        Health::Working
    } else if current_round(view, &members)
        .and_then(|round| view.agents.get(round))
        .is_some_and(|agent| agent.activity.starts_with('✗') || agent.activity.starts_with('⚠'))
    {
        Health::Problem
    } else {
        Health::Idle
    }
}

/// The node's status text (without an icon), including its provider. For
/// task cards this is the state word, its round and the provider, read
/// through the shared interpretation (taskinfo); what the agent is doing
/// and why it waits have their own line and never crowd the state out.
/// The CLI that runs this card's agent: the verifier's while a task is
/// verified, else the worker's, else whichever ran before.
pub(super) fn provider_of<'a>(view: &'a View, key: &str) -> Option<&'a str> {
    let provider = |key: &str| {
        view.agents
            .get(key)
            .map(|agent| agent.provider.trim())
            .filter(|provider| !provider.is_empty())
    };
    if let Some((_, id)) = key.split_once(':') {
        let task = view.tasks.iter().find(|task| task.id == id)?;
        let (first, second) = if task.status == "verifying" {
            ("verifier", "worker")
        } else {
            ("worker", "verifier")
        };
        return provider(&format!("{first}:{id}")).or_else(|| provider(&format!("{second}:{id}")));
    }
    let members = card_members(view, key);
    provider(current_round(view, &members).unwrap_or(key))
}

pub(super) fn describe(view: &View, key: &str) -> String {
    key.split_once(':').map_or_else(
        || describe_agent(view, key),
        |(_, id)| describe_task(view, id),
    )
}

fn describe_task(view: &View, id: &str) -> String {
    let Some(task) = view.tasks.iter().find(|task| task.id == id) else {
        return "?".into();
    };
    let status = if task.status == "verifying" {
        format!("verifier r{}", task.round)
    } else {
        format!("{} · worker", taskinfo::worker_status(task))
    };
    let role = if task.status == "verifying" {
        "verifier"
    } else {
        "worker"
    };
    let provider = view
        .agents
        .get(&format!("{role}:{id}"))
        .map(|agent| agent.provider.as_str());
    taskinfo::with_provider(&status, provider)
}

fn describe_agent(view: &View, key: &str) -> String {
    let members = card_members(view, key);
    let round = current_round(view, &members);
    let agent = round.or(Some(key)).and_then(|round| view.agents.get(round));
    let provider = agent.map(|agent| agent.provider.as_str());
    let working = health(view, key) == Health::Working;
    let doing = taskinfo::doing(view, round.unwrap_or(key));
    let text = requirements_label(round, &activity_label(agent, working));
    let text = taskinfo::with_provider(&text, provider);
    match doing {
        Some(doing) => format!("{text} · {doing}"),
        None => text,
    }
}

fn activity_label(agent: Option<&super::super::view::Agent>, working: bool) -> String {
    if working {
        return "working".to_owned();
    }
    let activity = agent
        .map_or("", |agent| agent.activity.as_str())
        .trim_start_matches(['✗', '⚠', '✓', ' ']);
    if activity.is_empty() {
        "idle".to_owned()
    } else {
        activity.to_owned()
    }
}

fn requirements_label(round: Option<&str>, activity: &str) -> String {
    let Some(index) = round.and_then(requirements_round) else {
        return activity.to_owned();
    };
    format!(
        "r{}/{} {} · {activity}",
        index + 1,
        REQUIREMENTS_ROUNDS.len(),
        REQUIREMENTS_ROUNDS[index].trim_start_matches("requirements-")
    )
}

/// The card's activity line: what this role is doing, the dependencies it
/// waits for, or the cause it stopped with — one line of its own, shared
/// with details through taskinfo, and independent of the
/// state word and the provider.
pub(super) fn activity_line(view: &View, key: &str) -> Option<String> {
    let (_, id) = key.split_once(':')?;
    let task = view.tasks.iter().find(|task| task.id == id)?;
    taskinfo::task_activity(view, task)
}

/// The same bounded, grapheme-safe text drives both layout and rendering.
pub(super) fn card_lines(view: &View, key: &str, width: usize, max: usize) -> Vec<String> {
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
        super::super::super::PLANNER => {
            Some("Turns agreed requirements into a task plan.".to_owned())
        }
        PLAN_VERIFIER => Some("Independently checks the plan before you see it.".to_owned()),
        AUDITOR => Some("Checks the finished work delivers your request.".to_owned()),
        super::super::super::COMMENT_ORCHESTRATOR => {
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
pub(super) fn fit_lines(text: &str, width: usize, max: usize) -> Vec<String> {
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
pub(super) fn effect_lines(view: &View, key: &str, width: usize, max: usize) -> Vec<String> {
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

pub fn mark(status: &str) -> (&'static str, Color) {
    match status {
        "approved" => ("✓", super::super::theme::ayu::GREEN),
        "coordinating" | "needs_user" | "waiting" => ("!", super::super::theme::ayu::YELLOW),
        "failed" => ("✗", super::super::theme::ayu::RED),
        "skipped" | "cancelled" | "revised" => ("–", super::super::theme::ayu::DIM),
        "working" | "verifying" => ("●", super::super::theme::ayu::BLUE),
        _ => ("○", super::super::theme::ayu::DIM),
    }
}

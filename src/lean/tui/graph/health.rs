use super::*;
use crate::lean::tui::view::TaskRow;

mod cards;
pub(super) use cards::{activity_line, card_lines, effect_lines, fit_lines};

/// How an agent is doing, which decides its icon and color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Health {
    Idle,
    Working,
    Waiting,
    Problem,
    Done,
}

impl Health {
    /// Every state, in the order the graph filter lists them.
    pub const ALL: [Self; 5] = [
        Self::Working,
        Self::Waiting,
        Self::Problem,
        Self::Idle,
        Self::Done,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Idle => "queued",
            Self::Working => "working",
            Self::Waiting => "waiting",
            Self::Problem => "failed",
            Self::Done => "done",
        }
    }
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

/// Every provider a card's agents run on: a task's worker and verifier.
pub(super) fn providers_of<'a>(view: &'a View, key: &str) -> Vec<&'a str> {
    let Some((_, id)) = key.split_once(':') else {
        return provider_of(view, key).into_iter().collect();
    };
    ["worker", "verifier"]
        .iter()
        .filter_map(|role| view.agents.get(&format!("{role}:{id}")))
        .map(|agent| agent.provider.trim())
        .filter(|provider| !provider.is_empty())
        .collect()
}

/// The providers the team's agents run on, sorted, for the graph filter.
pub fn providers(view: &View) -> Vec<String> {
    let mut found: Vec<String> = view
        .agents
        .values()
        .map(|agent| agent.provider.trim().to_owned())
        .filter(|provider| !provider.is_empty())
        .collect();
    found.sort_unstable();
    found.dedup();
    found
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

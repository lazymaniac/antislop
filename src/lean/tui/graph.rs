//! The team as a two-dimensional graph: one card per agent, a task's agents
//! framed together, orthogonal edges for what relates them. The layout
//! follows the visible content of each card and reroutes links as it changes.

use super::taskinfo;
use super::theme::Theme;
use super::view::{
    AUDITOR, ORCHESTRATOR, PLAN_REVIEWER, TaskRow, View, agent_name, clean, truncate, wrap,
};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use std::collections::HashMap;
use std::time::Duration;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Content rectangle inside an expanded card, clipped to the visible graph.
/// Keep this geometry shared by panel rendering and its layout measurements.
pub(super) fn expanded_card_content_area(card: Rect, viewport: Rect) -> Rect {
    Rect::new(
        card.x.saturating_add(2),
        card.y.saturating_add(4),
        card.width.saturating_sub(4),
        card.height.saturating_sub(5),
    )
    .intersection(viewport)
}

/// Current extraction and historical requirements roles share one card.
/// These are role addresses, not a fixed number of rounds to execute.
const REQUIREMENTS_ROUNDS: [&str; 4] = [
    "requirements-extraction-analyst",
    "requirements-extraction-reviewer",
    "requirements-options-analyst",
    "requirements-options-reviewer",
];
const REQUIREMENTS_NAME: &str = "requirements";

fn requirements_round(key: &str) -> Option<usize> {
    REQUIREMENTS_ROUNDS.iter().position(|role| *role == key)
}

/// The members of the agent a card stands for: the card's own key for most
/// agents, every round present for the requirements agent.
fn card_members<'a>(view: &'a View, key: &'a str) -> Vec<&'a str> {
    if requirements_round(key).is_none() {
        return vec![key];
    }
    REQUIREMENTS_ROUNDS
        .into_iter()
        .filter(|role| view.agents.contains_key(*role))
        .collect()
}

/// The round the requirements agent is in: the last one running, else the
/// last one that reported anything.
fn current_round<'a>(view: &View, members: &[&'a str]) -> Option<&'a str> {
    members
        .iter()
        .rev()
        .find(|key| view.agents.get(**key).is_some_and(|agent| agent.running))
        .or_else(|| {
            members.iter().rev().find(|key| {
                view.agents
                    .get(**key)
                    .is_some_and(|agent| !agent.activity.is_empty())
            })
        })
        .copied()
}

fn node_name(key: &str) -> String {
    if requirements_round(key).is_some() {
        REQUIREMENTS_NAME.into()
    } else {
        match key {
            "requirements-audit-analyst" => "requirements audit".into(),
            "requirements-audit-reviewer" => "audit reviewer".into(),
            _ => agent_name(key),
        }
    }
}

/// Workflow order, independent of agent creation time or alphabetical order.
fn workflow_stage(key: &str) -> u8 {
    match key {
        ORCHESTRATOR => 0,
        _ if requirements_round(key).is_some() => 1,
        crate::lean::PLANNER => 2,
        PLAN_REVIEWER => 3,
        "requirements-audit-analyst" => 5,
        "requirements-audit-reviewer" => 6,
        AUDITOR => 7,
        _ => 4,
    }
}

fn is_final_review(key: &str) -> bool {
    workflow_stage(key) >= 5
}

pub const WIDTH: u16 = 32;
const MAX_SUMMARY_LINES: usize = 5;
const MAX_EFFECT_LINES: usize = 2;
/// Smallest card a small screen shrinks to: the header slots, one summary
/// line and the meta line; narrower than this the text has no room left.
const MIN_WIDTH: u16 = 16;
/// Upper bounds for compact cards; actual height follows visible content.
pub const HEIGHT: u16 = 14;
const MIN_HEIGHT: u16 = 8;
const ADMIN_SUMMARY_LINES: usize = 2;
const GAP_X: u16 = 6;
const GAP_Y: u16 = 3;
/// Room left of the first column for edges entering it.
const MARGIN: u16 = 3;
/// Columns between a task's frame and its cards.
const FRAME_X: u16 = 2;
/// How long a message keeps its edge highlighted.
const RECENT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Depends,
    Reviews,
    Mail { recent: bool },
    Supervises,
}

impl Kind {
    fn style(self) -> Style {
        match self {
            Self::Depends => Style::default().fg(super::theme::ayu::BLUE),
            Self::Reviews => Style::default().fg(super::theme::ayu::YELLOW),
            Self::Mail { recent: true } => Style::default()
                .fg(super::theme::ayu::PURPLE)
                .add_modifier(Modifier::BOLD),
            Self::Mail { recent: false } => Style::default().fg(super::theme::ayu::PURPLE),
            Self::Supervises => Style::default().fg(super::theme::ayu::DIM),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GroupingMode {
    #[default]
    Dependency,
    Status,
}

impl GroupingMode {
    pub fn next(self) -> Self {
        match self {
            Self::Dependency => Self::Status,
            Self::Status => Self::Dependency,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Dependency => "dependencies",
            Self::Status => "state",
        }
    }
}

mod model;
pub(in crate::lean::tui::graph) use model::{Node, Size};

#[derive(Debug, Clone, PartialEq)]
struct Edge {
    from: String,
    to: String,
    kind: Kind,
    label: Option<String>,
    /// The path laid out with the graph, for edges between and inside
    /// frames; other edges are routed between their two cards.
    path: Option<Vec<(u16, u16)>>,
}

/// The border around one task's cards: its options card, worker and
/// reviewer, titled with the task.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    /// `task:{id}`, the `group` of every card inside.
    pub key: String,
    pub task: String,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub visible: bool,
}

#[derive(Debug, Default)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub frames: Vec<Frame>,
    edges: Vec<Edge>,
    pub width: u16,
    pub height: u16,
    pub scope: Scope,
    pub collapsed: std::collections::HashSet<String>,
    pub grouping: GroupingMode,
    pub expanded: Option<String>,
    pub expanded_tab: usize,
}

mod cache;
mod card_content;
mod filters;
pub use cache::{LayoutCache, LayoutIdentity};
pub use filters::{RELATIONS, Relations, Scope};

/// Which of a task's agents a card stands for. A task that only proposes
/// options is one short exchange, so its worker and reviewer share a card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Worker,
    Reviewer,
    Pair,
}

fn is_pair(task: &TaskRow) -> bool {
    task.kind == "options"
}

/// The task a card belongs to and the role it shows.
fn task_card<'a>(view: &'a View, key: &str) -> Option<(&'a TaskRow, Role)> {
    let (id, reviewer) = crate::lean::task_role(key)?;
    let task = view.tasks.iter().find(|task| task.id == id)?;
    let role = if is_pair(task) {
        Role::Pair
    } else if reviewer {
        Role::Reviewer
    } else {
        Role::Worker
    };
    Some((task, role))
}

/// The card an agent is shown on: its own, except that the requirements
/// rounds share one card and so do the two agents of an options task.
/// Role addresses in events and storage stay intact.
pub fn card_key(view: &View, key: &str) -> String {
    if requirements_round(key).is_some() {
        return REQUIREMENTS_ROUNDS
            .into_iter()
            .find(|role| view.agents.contains_key(*role))
            .unwrap_or(key)
            .to_owned();
    }
    match task_card(view, key) {
        Some((task, Role::Pair)) => format!("worker:{}", task.id),
        _ => key.to_owned(),
    }
}

/// Agents of the session itself, not of a task; their cards are compact.
fn is_admin(key: &str) -> bool {
    !key.contains(':')
}

pub type EdgeRoute<'a> = (&'a str, &'a str, Vec<(u16, u16)>);

mod canvas;
mod plan;
pub(super) use plan::PlanGraph;
mod health;
mod layout;
mod nodes;
mod routing;

pub use canvas::Canvas;
pub use health::{Health, health, mark, providers};
#[cfg(test)]
#[path = "graph/tests.rs"]
mod tests;

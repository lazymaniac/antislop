//! The team as a two-dimensional graph: one card per task, orthogonal edges for
//! what relates them. The layout depends only on the plan's shape, so nodes
//! stay put while events arrive.

use super::taskinfo;
use super::theme::Theme;
use super::view::{AUDITOR, ORCHESTRATOR, PLAN_VERIFIER, View, agent_name, clean, truncate, wrap};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use std::collections::HashMap;
use std::time::Duration;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// The requirements phase runs these roles as rounds of one agent; the graph
/// shows them as a single card, represented by the first one present.
const REQUIREMENTS_ROUNDS: [&str; 4] = [
    "requirements-extraction-analyst",
    "requirements-extraction-verifier",
    "requirements-options-analyst",
    "requirements-options-verifier",
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
        agent_name(key)
    }
}

pub const WIDTH: u16 = 32;
const MAX_SUMMARY_LINES: usize = 5;
const MAX_EFFECT_LINES: usize = 2;
/// Smallest card a small screen shrinks to: the header slots, one summary
/// line and the meta line; narrower than this the text has no room left.
const MIN_WIDTH: u16 = 16;
/// Fixed slots inside a card, counted from its top, each starting right
/// where the previous block can end: title, agent and work kind, phase, verifier state, one activity
/// line, the working branch, up to `MAX_SUMMARY_LINES` summary lines, up to `MAX_EFFECT_LINES`
/// effect lines and the meta line at the bottom. The height is that same
/// arithmetic's end, so frame and slots are one rule that cannot drift
/// apart: heights and positions depend only on the plan's shape, filters
/// and grouping — never on activity, an effect, a verdict or a retry.
const ACTIVITY_ROW: u16 = 4;
const BRANCH_ROW: u16 = ACTIVITY_ROW + 1;
const BODY_ROW: u16 = BRANCH_ROW + 1;
const EFFECT_ROW: u16 = BODY_ROW + MAX_SUMMARY_LINES as u16;
pub const HEIGHT: u16 = EFFECT_ROW + MAX_EFFECT_LINES as u16 + 1;
const MIN_HEIGHT: u16 = BODY_ROW + 2;
/// Cards of the session's own agents (orchestrator, planner, verifiers of
/// the plan, auditor, requirements) carry no task: their fixed one-sentence
/// description follows the title, provider and state rows directly (a
/// recorded branch takes its last line), and the card is only as tall as that.
const ADMIN_BODY_ROW: u16 = 3;
const ADMIN_SUMMARY_LINES: usize = 2;
const ADMIN_HEIGHT: u16 = ADMIN_BODY_ROW + ADMIN_SUMMARY_LINES as u16;
const GAP_X: u16 = 6;
const GAP_Y: u16 = 3;
/// Room left of the first column for edges entering it.
const MARGIN: u16 = 3;
/// How long a message keeps its edge highlighted.
const RECENT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Filter {
    #[default]
    All,
    Depends,
    Verifies,
    Mail,
    Supervises,
}

impl Filter {
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::Depends,
            Self::Depends => Self::Verifies,
            Self::Verifies => Self::Mail,
            Self::Mail => Self::Supervises,
            Self::Supervises => Self::All,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Depends => "dependencies",
            Self::Verifies => "verification",
            Self::Mail => "messages",
            Self::Supervises => "supervision",
        }
    }

    fn shows(self, kind: Kind) -> bool {
        match self {
            Self::All => true,
            Self::Depends => kind == Kind::Depends,
            Self::Verifies => kind == Kind::Verifies,
            Self::Mail => matches!(kind, Kind::Mail { .. }),
            Self::Supervises => kind == Kind::Supervises,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Depends,
    Verifies,
    Mail { recent: bool },
    Supervises,
}

impl Kind {
    fn style(self) -> Style {
        match self {
            Self::Depends => Style::default().fg(super::theme::ayu::BLUE),
            Self::Verifies => Style::default().fg(super::theme::ayu::YELLOW),
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
}

#[derive(Debug, Default)]
pub struct Graph {
    pub nodes: Vec<Node>,
    edges: Vec<Edge>,
    pub width: u16,
    pub height: u16,
    pub filter_query: Option<String>,
    pub collapsed: std::collections::HashSet<String>,
    pub grouping: GroupingMode,
}

mod cache;
pub use cache::LayoutCache;

/// Canonical UI identity; role addresses in events and storage stay intact.
pub fn task_node_key(key: &str) -> String {
    match crate::lean::task_role(key) {
        Some((id, true)) => format!("worker:{id}"),
        _ => key.to_owned(),
    }
}

/// `task_node_key`, with the requirements rounds folded into their one card.
fn card_key(view: &View, key: &str) -> String {
    if requirements_round(key).is_some() {
        return REQUIREMENTS_ROUNDS
            .into_iter()
            .find(|role| view.agents.contains_key(*role))
            .unwrap_or(key)
            .to_owned();
    }
    task_node_key(key)
}

fn column_x(size: Size, column: u16) -> u16 {
    MARGIN + column * (size.w + GAP_X)
}

fn row_y(size: Size, row: u16) -> u16 {
    row * (size.h + GAP_Y)
}

/// Agents of the session itself, not of a task; their cards are compact.
fn is_admin(key: &str) -> bool {
    !key.contains(':')
}

pub type EdgeRoute<'a> = (&'a str, &'a str, Vec<(u16, u16)>);

mod canvas;
mod health;
mod layout;
mod nodes;
mod routing;

pub use canvas::Canvas;
pub use health::{Health, health, mark};
#[cfg(test)]
#[path = "graph/tests.rs"]
mod tests;

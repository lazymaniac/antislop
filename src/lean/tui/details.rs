//! Full-screen details of one agent or conversation: what it was asked,
//! its session history, messages, deliverables, obligations, and comments.

use super::super::tasks::{TaskContract, obligations, status};
use super::taskinfo;
use super::theme::Theme;
use super::view::{View, agent_name, dim, duration, status_label, wrap};
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
};
use serde_json::Value;
use std::rc::Rc;

mod checks;
mod model;
mod navigation;
mod presentation;
mod records;
pub use model::DetailRecord;
pub use model::{Composer, Details, Drafts};
pub use presentation::{
    CheckRow, check_rows, details_footer_hint, details_footer_hint_for, format_tabs,
};

pub const TABS: [&str; 9] = [
    "Overview",
    "Messages",
    "Result",
    "Checks",
    "Comments",
    "Review",
    "Requirements",
    "Verifier",
    "Plan",
];
pub const RESULT: usize = 2;
pub const CHECKS: usize = 3;
pub const COMMENTS: usize = 4;
pub const REVIEW: usize = 5;
pub const REQUIREMENTS: usize = 6;
/// The verifier's own Overview: what it checks, and its terminal.
pub const VERIFIER: usize = 7;
pub const PLAN: usize = 8;

/// The label of a tab for this address: the first tab of a task's worker
/// section is the worker's Overview, the first of the verifier's is its own.
pub fn tab_name(key: &str, tab: usize) -> &'static str {
    match (tab, key.split_once(':').map(|(role, _)| role)) {
        (0, Some("worker" | "verifier")) => "Worker",
        _ => TABS[tab],
    }
}

/// Whether a tab is an agent's Overview (its info beside its terminal).
pub fn is_overview(tab: usize) -> bool {
    tab == 0 || tab == VERIFIER
}

/// Where the verifier's section starts in a task's tab list.
pub fn section_starts(key: &str, tab: usize) -> bool {
    tab == VERIFIER && tabs_for(key).contains(&VERIFIER)
}

/// Task addresses share one set of tabs, for both worker and verifier addresses.
pub fn tabs_for(key: &str) -> Vec<usize> {
    match key.split_once(':').map_or(key, |(role, _)| role) {
        "worker" | "verifier" => vec![0, 1, RESULT, VERIFIER, CHECKS, COMMENTS, REQUIREMENTS],
        "planner" => vec![0, PLAN, 1, COMMENTS],
        "plan-verifier" | "auditor" => vec![0, REVIEW, COMMENTS],
        _ => vec![0, 1, COMMENTS],
    }
}

impl Details {
    pub(super) fn fetch_failed(&mut self, error: &str) -> bool {
        self.fetch_in_flight = false;
        self.fetched_round = None;
        self.fetch_failures = self.fetch_failures.saturating_add(1);
        // One immediate recovery attempt, then exponential backoff capped at 30s.
        let seconds = if self.fetch_failures == 1 {
            0
        } else {
            (1u64 << self.fetch_failures.saturating_sub(2).min(5)).min(30)
        };
        self.fetch_retry_after =
            Some(std::time::Instant::now() + std::time::Duration::from_secs(seconds));
        let changed = self.fetch_error.as_deref() != Some(error);
        self.fetch_error = Some(error.into());
        changed
    }

    pub(super) fn load_task(&mut self, task: Value) {
        self.fetch_in_flight = false;
        self.fetch_failures = 0;
        self.fetch_retry_after = None;
        self.fetch_error = None;
        self.fetched_round = task["round"].as_u64().or(self.fetched_round);
        self.history_unavailable = task["history_unavailable"] == true
            && task["round"].as_u64().is_some_and(|round| round > 0);
        if let Some(id) = task["id"].as_str() {
            self.task_uuid = Some(id.into());
        }
        self.available_rounds = task["_rounds"]
            .as_array()
            .map(|rounds| {
                rounds
                    .iter()
                    .filter_map(|round| round["round"].as_u64())
                    .collect()
            })
            .unwrap_or_default();
        self.documents.clear();
        self.task = Some(task);
    }

    pub fn new(key: String) -> Self {
        Details {
            key,
            input_width: [1; 2],
            follow: true,
            ..Details::default()
        }
    }

    pub(super) fn comments<'a>(&self, view: &'a View) -> Vec<&'a super::view::Comment> {
        let round = self
            .selected_round
            .or_else(|| self.task.as_ref().and_then(|task| task["round"].as_u64()));
        let run = self.task.as_ref().and_then(|task| task["_run"].as_str());
        view.comments_on(&self.key)
            .into_iter()
            .filter(|comment| {
                view.comment_scopes
                    .get(&comment.id)
                    .is_none_or(|(scope_round, scope_run)| {
                        scope_round.is_none_or(|value| Some(value) == round)
                            && scope_run.as_deref().is_none_or(|value| Some(value) == run)
                    })
            })
            .collect()
    }

    /// The id of the selected comment, on the Comments tab.
    pub fn selected_comment(&self, view: &View) -> Option<u64> {
        (self.tab == COMMENTS)
            .then(|| self.comments(view).get(self.comment).map(|c| c.id))
            .flatten()
    }

    /// The id of the selected check, on the Checks tab.
    pub fn selected_check(&self) -> Option<String> {
        if self.tab != CHECKS {
            return None;
        }
        let rows = check_rows(self.task.as_ref()?);
        rows.get(self.check.min(rows.len().saturating_sub(1)))
            .map(|row| row.id.clone())
    }

    /// How many checks the open task has, for moving the selection.
    pub fn check_count(&self) -> usize {
        self.task.as_ref().map_or(0, |task| check_rows(task).len())
    }

    /// The task id of a worker or verifier.
    pub fn task_id(&self) -> Option<&str> {
        self.key.split_once(':').map(|(_, id)| id)
    }

    pub fn lines(&mut self, view: &View) -> Vec<(Style, String)> {
        let records = self.records(view);
        Self::render_records(&records, &self.query)
    }

    /// Compact header; source prompts remain in the session document.
    pub(super) fn summary(&self, view: &View, theme: &Theme) -> Vec<Line<'static>> {
        self.hero(view, theme)
    }

    /// Both agents of this task, in the order they work: worker, verifier.
    pub(super) fn role_keys(&self) -> Vec<String> {
        match self.key.split_once(':') {
            Some(("worker" | "verifier", id)) => {
                vec![format!("worker:{id}"), format!("verifier:{id}")]
            }
            _ => vec![self.key.clone()],
        }
    }

    pub fn overview(&self) -> bool {
        is_overview(self.tab)
    }

    /// The terminal shows unless the info page took its place.
    pub fn terminal_visible(&self) -> bool {
        self.overview() && (!self.show_info || self.split)
    }

    /// The one agent the Overview in front shows: the worker's tab holds the
    /// worker, the verifier's tab the verifier.
    pub(super) fn session_keys(&self) -> Vec<String> {
        match self.key.split_once(':') {
            Some(("worker" | "verifier", id)) => {
                vec![if self.tab == VERIFIER {
                    format!("verifier:{id}")
                } else {
                    format!("worker:{id}")
                }]
            }
            _ => vec![self.key.clone()],
        }
    }

    fn render_records(records: &[Rc<DetailRecord>], query: &str) -> Vec<(Style, String)> {
        let mut out = vec![];
        let query_lower = query.trim().to_lowercase();
        if !query.is_empty() {
            out.push((
                Style::default()
                    .fg(super::theme::ayu::BLUE)
                    .add_modifier(Modifier::BOLD),
                format!("🔍 Record filter: \"{}\"  (Esc: clear)", query),
            ));
            out.push((Style::default(), String::new()));
        }
        let mut matched_records = 0;
        for rec in records {
            let is_match =
                query_lower.is_empty() || rec.search_text.to_lowercase().contains(&query_lower);
            if is_match {
                matched_records += 1;
                out.extend(rec.lines.iter().cloned());
            }
        }
        if !query_lower.is_empty() && matched_records == 0 {
            out.push((dim(), format!("No records matching query \"{}\".", query)));
        }
        out
    }
}

// The verifier's obligations (the goal, the task's acceptance criteria, then
// its kind's risks) and how far each is: what the verifier reported while it
// works, else how the latest verdict accounted for it.

#[cfg(test)]
#[path = "details/tests.rs"]
mod tests;

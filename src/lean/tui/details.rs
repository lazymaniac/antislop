//! Full-screen details of one agent or conversation: what it was asked,
//! its session history, messages, deliverables, obligations, and comments.

use super::super::tasks::{TaskSpec, obligations, status};
use super::taskinfo;
use super::theme::Theme;
use super::view::{View, agent_name, dim, duration, status_label, wrap};
use crate::lean::store::SessionInfo;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Padding, Paragraph, Scrollbar, ScrollbarOrientation,
        ScrollbarState,
    },
};
use serde_json::Value;
use std::collections::HashMap;
use std::rc::Rc;

pub const TABS: [&str; 7] = [
    "Overview",
    "Messages",
    "Result",
    "Checks",
    "Comments",
    "Review",
    "Requirements",
];
pub const RESULT: usize = 2;
pub const CHECKS: usize = 3;
pub const COMMENTS: usize = 4;
pub const REVIEW: usize = 5;
pub const REQUIREMENTS: usize = 6;

/// Task addresses share one set of tabs, including legacy verifier keys.
pub fn tabs_for(key: &str) -> Vec<usize> {
    match key.split_once(':').map_or(key, |(role, _)| role) {
        "worker" | "verifier" => vec![0, 1, RESULT, CHECKS, COMMENTS, REQUIREMENTS],
        "plan-verifier" | "auditor" => vec![0, REVIEW, COMMENTS],
        _ => vec![0, 1, COMMENTS],
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DetailRecord {
    pub lines: Vec<(Style, String)>,
    pub search_text: String,
    pub raw_value: Option<Value>,
}

/// Only the currently inspected agents are retained; unchanged blocks reuse
/// their sanitized, formatted and searchable records between redraws.
#[derive(Debug, Clone, PartialEq, Default)]
struct SessionCache {
    provider: String,
    expanded: bool,
    blocks: Vec<super::session::Block>,
    records: Vec<Rc<DetailRecord>>,
    revision: Option<u64>,
}

impl SessionCache {
    fn records(
        &mut self,
        provider: &str,
        blocks: &[super::session::Block],
        expanded: bool,
        revision: Option<u64>,
    ) -> &[Rc<DetailRecord>] {
        if revision.is_some()
            && self.revision == revision
            && self.provider == provider
            && self.expanded == expanded
        {
            return &self.records;
        }
        self.revision = revision;
        let shared = if self.provider == provider && self.expanded == expanded {
            self.blocks
                .iter()
                .zip(blocks)
                .take_while(|(a, b)| a == b)
                .count()
        } else {
            0
        };
        self.provider = provider.to_owned();
        self.expanded = expanded;
        self.blocks.truncate(shared);
        self.records.truncate(shared);
        for block in &blocks[shared..] {
            let lines = super::session::render(provider, std::slice::from_ref(block), true);
            let search_text = lines
                .iter()
                .map(|(_, line)| line.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            self.records.push(Rc::new(DetailRecord {
                raw_value: Some(serde_json::json!({ "session": search_text })),
                lines,
                search_text,
            }));
            self.blocks.push(block.clone());
        }
        &self.records
    }
}

/// Wrapped output is rebuilt only when records or display options change.
/// Idle redraws inspect record identities and copy only the visible lines.
#[derive(Debug, Clone, PartialEq, Default)]
struct LayoutCache {
    records: Vec<Rc<DetailRecord>>,
    query: String,
    raw: bool,
    width: usize,
    lines: Vec<Line<'static>>,
}

impl LayoutCache {
    fn update(
        &mut self,
        records: Vec<Rc<DetailRecord>>,
        query: &str,
        raw: bool,
        width: usize,
    ) -> &[Line<'static>] {
        let unchanged = self.width == width
            && self.query == query
            && self.raw == raw
            && self.records.len() == records.len()
            && self
                .records
                .iter()
                .zip(&records)
                .all(|(a, b)| Rc::ptr_eq(a, b) || a == b);
        if !unchanged {
            self.lines = Details::render_records(&records, query, raw)
                .into_iter()
                .flat_map(|(style, text)| {
                    wrap(&text, width)
                        .into_iter()
                        .map(move |line| Line::styled(line, style))
                })
                .collect();
            self.records = records;
            self.query = query.to_owned();
            self.raw = raw;
            self.width = width;
        }
        &self.lines
    }
}

#[derive(Debug, Clone, PartialEq)]
struct OverviewCache {
    key: String,
    revision: Option<u64>,
    expanded: bool,
    record: Rc<DetailRecord>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Composer {
    pub input: super::InputBuffer,
    pub sending: bool,
    pub operation: Option<String>,
    pub error: Option<String>,
    pub sent_text: Option<String>,
    pub mail: Option<u64>,
    pub delivery: Option<String>,
    pub address: Option<String>,
}

pub type Drafts = std::rc::Rc<std::cell::RefCell<HashMap<(String, String), Composer>>>;

#[derive(Debug, Clone, PartialEq)]
pub struct Details {
    /// Last rendered document viewport, excluding the scrollbar.
    pub text_area: Rect,
    /// None follows the current round; an explicit older round freezes selection.
    pub selected_round: Option<u64>,
    pub available_rounds: Vec<u64>,
    pub latest_observed: u64,
    pub active_panel: usize,
    pub panel_scroll: [usize; 2],
    pub panel_expanded: [bool; 2],
    pub documents: HashMap<String, super::document::Document>,
    pub round_agents: HashMap<String, super::view::Agent>,
    panel_layout: [LayoutCache; 2],
    overview_cache: [Option<OverviewCache>; 2],
    pub fetched_events: u64,
    pub task_uuid: Option<String>,
    pub conversation: String,
    pub drafts: Drafts,
    pub editing: bool,
    pub input_width: [usize; 2],
    pub key: String,
    pub tab: usize,
    /// Lines scrolled down from the top.
    pub scroll: usize,
    /// The task as the daemon stores it, once fetched.
    pub task: Option<Value>,
    /// The task round the last task fetch was issued for, so a reply for a
    /// round that already moved on can be dropped and refetched, not shown.
    pub fetched_round: Option<u64>,
    /// Request identity belongs to the saved details, including across A→B→A.
    pub fetched_generation: Option<u64>,
    pub fetched_plan: Option<String>,
    /// The selected comment on the Comments tab.
    pub comment: usize,
    /// The selected check on the Checks tab.
    pub check: usize,
    /// The selection moved: the next draw scrolls it into view.
    pub reveal_check: bool,
    /// How many check events the last task fetch had seen.
    pub fetched_checks: u64,
    pub fetch_failures: u32,
    pub fetch_retry_after: Option<std::time::Instant>,
    pub fetch_error: Option<String>,
    /// Scroll memory per (agent_key, tab_index)
    pub scroll_memory: HashMap<(String, usize), usize>,
    /// Query memory per (agent_key, tab_index)
    pub query_memory: HashMap<(String, usize), String>,
    /// Active search/filter query on current tab
    pub query: String,
    /// Whether user is typing a query in search mode
    pub searching: bool,
    /// Readable view (false) vs raw JSON (true)
    pub raw: bool,
    /// Retained turn metadata returned by the daemon
    pub sessions: Vec<SessionInfo>,
    /// Explicit flag indicating missing/pruned historical data
    pub history_unavailable: bool,
    /// Prompts of the Overview tab are shown whole instead of folded.
    pub expand_prompts: bool,
    /// Follow new entries by default; scrolling up pauses, G/End resumes.
    pub follow: bool,
    /// Full task metadata, explicitly opened instead of the live transcript.
    pub show_info: bool,
    pub(super) pending_g: bool,
    session_cache: HashMap<String, SessionCache>,
    layout_cache: LayoutCache,
}

impl Details {
    pub(super) fn fetch_failed(&mut self, error: &str) -> bool {
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
        self.fetch_failures = 0;
        self.fetch_retry_after = None;
        self.fetch_error = None;
        self.fetched_round = task["round"].as_u64().or(self.fetched_round);
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
        self.round_agents.clear();
        if let Some(transcripts) = task["_transcripts"].as_object() {
            for (key, events) in transcripts {
                let mut projection = View::session_history();
                if let Some(events) = events.as_array() {
                    for event in events {
                        projection.apply(event);
                    }
                }
                let mut agent = projection.agents.into_values().next().unwrap_or_default();
                // The durable projection is already scoped to one round.
                agent.feed_dropped = 0;
                self.round_agents.insert(key.clone(), agent);
            }
        }
        if self.task.as_ref().and_then(|old| old["round"].as_u64()) != task["round"].as_u64() {
            self.overview_cache = Default::default();
        }
        self.task = Some(task);
    }

    pub fn new(key: String) -> Self {
        Details {
            text_area: Default::default(),
            selected_round: None,
            available_rounds: Vec::new(),
            latest_observed: 0,
            active_panel: usize::from(key.starts_with("verifier:")),
            panel_scroll: [0; 2],
            panel_expanded: [false; 2],
            documents: HashMap::new(),
            round_agents: HashMap::new(),
            panel_layout: Default::default(),
            overview_cache: Default::default(),
            fetched_events: 0,
            task_uuid: None,
            conversation: String::new(),
            drafts: Default::default(),
            editing: false,
            input_width: [1; 2],
            key,
            tab: 0,
            scroll: 0,
            task: None,
            fetched_round: None,
            fetched_generation: None,
            fetched_plan: None,
            comment: 0,
            check: 0,
            reveal_check: false,
            fetched_checks: 0,
            fetch_failures: 0,
            fetch_retry_after: None,
            fetch_error: None,
            scroll_memory: HashMap::new(),
            query_memory: HashMap::new(),
            query: String::new(),
            searching: false,
            raw: false,
            sessions: Vec::new(),
            history_unavailable: false,
            expand_prompts: false,
            follow: true,
            show_info: false,
            pending_g: false,
            session_cache: HashMap::new(),
            layout_cache: LayoutCache::default(),
        }
    }

    /// The tab `steps` places after (or, negative, before) the current one
    /// among those this agent has.
    pub fn neighbour_tab(&self, steps: isize) -> usize {
        let tabs = tabs_for(&self.key);
        let at = tabs.iter().position(|tab| *tab == self.tab).unwrap_or(0) as isize;
        tabs[(at + steps).rem_euclid(tabs.len() as isize) as usize]
    }

    /// The tab a number key picks: the n-th (from 0) of this agent's tabs.
    pub fn nth_tab(&self, n: usize) -> Option<usize> {
        tabs_for(&self.key).get(n).copied()
    }

    pub fn switch_tab(&mut self, next: usize) {
        let next_tab = next % TABS.len();
        if next_tab == self.tab {
            return;
        }
        self.save_current_tab();
        self.tab = next_tab;
        self.restore_current_tab();
        self.searching = false;
    }

    pub fn switch_key(&mut self, new_key: String) {
        self.save_current_tab();
        self.key = new_key;
        self.show_info = false;
        self.selected_round = None;
        self.task_uuid = None;
        self.available_rounds.clear();
        self.documents.clear();
        self.round_agents.clear();
        self.overview_cache = Default::default();
        self.panel_scroll = [0; 2];
        self.panel_expanded = [false; 2];
        self.active_panel = usize::from(self.key.starts_with("verifier:"));
        self.session_cache.clear();
        // A different element is a different task: its old record is gone.
        self.task = None;
        self.fetched_round = None;
        self.fetched_generation = None;
        self.fetched_plan = None;
        self.fetch_failures = 0;
        self.fetch_retry_after = None;
        self.fetch_error = None;
        self.restore_current_tab();
        self.searching = false;
    }

    fn composer_recipient(&self, key: &str) -> String {
        match (key.split_once(':'), &self.task_uuid) {
            (Some((role, _)), Some(uuid)) => format!("{role}:{uuid}"),
            _ => key.to_owned(),
        }
    }

    pub fn composer_key(&self) -> (String, String) {
        let keys = self.session_keys();
        let key = keys.get(self.active_panel).unwrap_or(&self.key);
        (self.conversation.clone(), self.composer_recipient(key))
    }

    pub fn change_round(&mut self, view: &View, direction: isize) {
        let latest = self
            .task_id()
            .and_then(|id| view.tasks.iter().find(|task| task.id == id))
            .map_or_else(
                || self.available_rounds.last().copied().unwrap_or(0),
                |task| task.round,
            );
        let mut rounds = self.available_rounds.clone();
        if !rounds.contains(&latest) {
            rounds.push(latest);
            rounds.sort_unstable();
        }
        let current = self.selected_round.unwrap_or(latest);
        if let Some(index) = rounds.iter().position(|round| *round == current) {
            let next = (index as isize + direction)
                .clamp(0, rounds.len().saturating_sub(1) as isize) as usize;
            let round = rounds[next];
            self.selected_round = (round != latest).then_some(round);
            self.editing = false;
            self.task = None;
            self.documents.clear();
            self.round_agents.clear();
            self.overview_cache = Default::default();
            self.scroll = 0;
            self.panel_scroll = [0; 2];
            self.follow = false;
        }
    }

    pub fn save_current_tab(&mut self) {
        self.scroll_memory
            .insert((self.key.clone(), self.tab), self.scroll);
        self.query_memory
            .insert((self.key.clone(), self.tab), self.query.clone());
    }

    pub fn restore_current_tab(&mut self) {
        self.scroll = *self
            .scroll_memory
            .get(&(self.key.clone(), self.tab))
            .unwrap_or(&0);
        self.follow = self.tab == 0 && !self.show_info;
        self.pending_g = false;
        self.query = self
            .query_memory
            .get(&(self.key.clone(), self.tab))
            .cloned()
            .unwrap_or_default();
    }

    fn comments<'a>(&self, view: &'a View) -> Vec<&'a super::view::Comment> {
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
        Self::render_records(&records, &self.query, self.raw)
    }

    /// Compact header; source prompts remain in the session document.
    fn summary(&self, view: &View, theme: &Theme) -> Vec<Line<'static>> {
        let key = self
            .session_keys()
            .get(self.active_panel)
            .cloned()
            .unwrap_or_else(|| self.key.clone());
        let agent = self
            .round_agents
            .get(&key)
            .or_else(|| view.agents.get(&key));
        let mut summary = agent.map_or_else(String::new, |agent| {
            format!(
                "{} · {} · {}",
                agent.provider,
                agent.model.as_deref().unwrap_or("default model"),
                agent.effort.as_deref().unwrap_or("default effort")
            )
        });
        if let Some(task) = self
            .task_id()
            .and_then(|id| view.tasks.iter().find(|task| task.id == id))
        {
            summary.push_str(&format!(
                " · {}",
                task.summary.as_deref().unwrap_or(&task.title)
            ));
        }
        vec![Line::styled(
            theme.clean(&summary).replace(['\n', '\r', '\t'], " "),
            theme.dim(),
        )]
    }

    fn session_keys(&self) -> Vec<String> {
        match self.key.split_once(':') {
            Some(("worker" | "verifier", id)) => {
                vec![format!("worker:{id}"), format!("verifier:{id}")]
            }
            _ => vec![self.key.clone()],
        }
    }

    fn render_records(
        records: &[Rc<DetailRecord>],
        query: &str,
        raw: bool,
    ) -> Vec<(Style, String)> {
        let mut out = vec![];
        let query_lower = query.trim().to_lowercase();
        if !query.is_empty() {
            out.push((
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
                format!(
                    "🔍 Record filter: \"{}\"  (v: toggle raw/readable view · Esc: clear)",
                    query
                ),
            ));
            out.push((Style::default(), String::new()));
        }
        let mut matched_records = 0;
        for rec in records {
            let is_match =
                query_lower.is_empty() || rec.search_text.to_lowercase().contains(&query_lower);
            if is_match {
                matched_records += 1;
                if raw {
                    if let Some(val) = &rec.raw_value {
                        let pretty = serde_json::to_string_pretty(val).unwrap_or_default();
                        for line in pretty.lines() {
                            out.push((Style::default().fg(Color::Green), line.to_owned()));
                        }
                    } else {
                        for (style, line) in &rec.lines {
                            out.push((*style, format!("[RAW] {line}")));
                        }
                    }
                    out.push((Style::default(), String::new()));
                } else {
                    out.extend(rec.lines.iter().cloned());
                }
            }
        }
        if !query_lower.is_empty() && matched_records == 0 {
            out.push((dim(), format!("No records matching query \"{}\".", query)));
        }
        out
    }

    fn records(&mut self, view: &View) -> Vec<Rc<DetailRecord>> {
        let plain = |text: String| (Style::default(), text);
        let label = |text: &str| {
            (
                Style::default().add_modifier(Modifier::BOLD),
                text.to_owned(),
            )
        };
        let is_conv = self.key == "conversation";
        let agent = self
            .round_agents
            .get(&self.key)
            .or_else(|| view.agents.get(&self.key));
        let task = self
            .task_id()
            .and_then(|id| view.tasks.iter().find(|task| task.id == id));
        let verifier = self.key.starts_with("verifier:");
        let mut records = vec![];

        match self.tab {
            REQUIREMENTS => {
                let mut lines = vec![label("Requirement ledger · shared across all rounds")];
                if let Some(stored) = &self.task {
                    for (field,name) in [("requirement_ids","Requirements"),("criterion_ids","Criteria"),("plan_item_ids","Plan items")] {
                        let ids = stored["spec"][field].as_array().map(|ids|ids.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).unwrap_or_default();
                        lines.push(plain(format!("{name}: {ids}")));
                    }
                    lines.push(plain(format!("Input generation: {}",stored["input_generation"])));
                }
                lines.push(plain("Open /requirements or /plan-items for sources, evidence and version history. Task approval alone does not verify requirements.".into()));
                let text = lines.iter().map(|(_,text)|text.as_str()).collect::<Vec<_>>().join(" ");
                records.push(Rc::new(DetailRecord {lines,search_text:text,raw_value:None}));
            }
            0 => {
                if is_conv {
                    let mut conv_lines = vec![];
                    conv_lines.push(label("Conversation details"));
                    conv_lines.push(plain(format!(
                        "Project: {}",
                        view.plan.as_deref().unwrap_or("current")
                    )));
                    conv_lines.push(plain(format!(
                        "Mode: {}",
                        match view.autonomous {
                            Some(true) => "autonomous",
                            Some(false) => "supervised",
                            None => "unspecified",
                        }
                    )));
                    conv_lines.push(plain(format!(
                        "Status: {}",
                        if view.paused {
                            "paused"
                        } else if view.idle {
                            "idle"
                        } else {
                            "active"
                        }
                    )));
                    if let Some(p) = &view.plan {
                        conv_lines.push(plain(format!("Active plan: {p}")));
                    }
                    let total_tasks = view.tasks.len();
                    let done_tasks = view
                        .tasks
                        .iter()
                        .filter(|t| status::finished(&t.status))
                        .count();
                    conv_lines.push(plain(format!(
                        "Tasks: total {total_tasks}, completed {done_tasks}"
                    )));
                    let _missing_reports = view
                        .tasks
                        .iter()
                        .filter(|t| status::finished(&t.status) && t.reason.is_none())
                        .count();
                    conv_lines.push(plain(format!(
                        "Task reports: {done_tasks}/{total_tasks} ready (explicitly missing: {} in progress)",
                        total_tasks.saturating_sub(done_tasks)
                    )));
                    let text = conv_lines
                        .iter()
                        .map(|(_, t)| t.as_str())
                        .collect::<Vec<_>>()
                        .join(" ");
                    records.push(Rc::new(DetailRecord {
                        lines: conv_lines,
                        search_text: text,
                        raw_value: Some(serde_json::json!({
                            "plan": view.plan,
                            "tasks_total": total_tasks,
                            "tasks_done": done_tasks,
                            "autonomous": view.autonomous,
                            "paused": view.paused,
                            "idle": view.idle,
                        })),
                    }));
                } else if self.show_info {
                    if let Some(agent) = agent.filter(|agent| !agent.provider.is_empty()) {
                        let mut cli = format!("CLI: {}", agent.provider);
                        if let Some(model) = &agent.model {
                            cli.push_str(&format!(" · model {model}"));
                        }
                        if let Some(effort) = &agent.effort {
                            cli.push_str(&format!(" · effort {effort}"));
                        }
                        let mut cli_lines = vec![plain(cli)];
                        if agent.turns > 0 {
                            cli_lines.push(plain(format!("Tokens: {}", agent.tokens())));
                        }
                        if let Some(last) = agent.last_ms {
                            cli_lines.push(plain(format!(
                                "Time in turns: {} (last {})",
                                duration(agent.busy_ms),
                                duration(last)
                            )));
                        }
                        let text = cli_lines
                            .iter()
                            .map(|(_, t)| t.as_str())
                            .collect::<Vec<_>>()
                            .join(" ");
                        records.push(Rc::new(DetailRecord {
                            lines: cli_lines,
                            search_text: text,
                            raw_value: Some(serde_json::json!({
                                "provider": agent.provider,
                                "model": agent.model,
                                "effort": agent.effort,
                                "turns": agent.turns,
                                "tokens_in": agent.tokens_in,
                                "tokens_out": agent.tokens_out,
                                "busy_ms": agent.busy_ms,
                            })),
                        }));
                    }
                    if let Some(task) = task {
                        let mut task_lines = vec![];
                        task_lines.push(plain(format!(
                            "Task status: {}{}",
                            status_label(&task.status),
                            match taskinfo::round_shown(task.round) {
                                Some(round) => format!(" (round {round})"),
                                None => " (not started yet)".to_owned(),
                            }
                        )));
                        if let Some(reason) = &task.reason {
                            // The raw reason stays whole here, never cut.
                            task_lines.push(plain(format!("Reason: {reason}")));
                        } else if let Some(why) = taskinfo::cause(task) {
                            task_lines.push(plain(format!("Reason: {why}")));
                        }
                        if let Some(effect) = taskinfo::effect_line(task) {
                            task_lines.push(plain(effect));
                        }
                        task_lines.push(plain(format!(
                            "Obligations: {}",
                            taskinfo::coverage_full(task)
                        )));
                        task_lines.push(plain(format!(
                            "Integration: {}",
                            taskinfo::integration_line(task)
                        )));
                        task_lines.push(plain(taskinfo::next_step(view, task)));
                        task_lines.push(plain(format!("Task {}: {}", task.id, task.title)));
                        task_lines.push(plain(format!(
                            "Kind: {}{}",
                            task.kind,
                            if task.writes {
                                " · modifies files"
                            } else {
                                " · read-only"
                            }
                        )));
                        if !task.depends_on.is_empty() {
                            task_lines.push(plain(format!(
                                "Depends on: {}",
                                task.depends_on.join(", ")
                            )));
                        }
                        if !task.scope.is_empty() {
                            task_lines.push(plain(format!("Scope: {}", task.scope.join(", "))));
                        }
                        task_lines.push(plain(String::new()));
                        task_lines.push(label("Goal"));
                        task_lines.push(plain(task.goal.clone()));
                        if verifier || !task.focus.is_empty() {
                            task_lines.push(plain(String::new()));
                            task_lines.push(label("Verification"));
                            task_lines.push(plain(task.focus.clone()));
                        }
                        if !task.risks.is_empty() {
                            task_lines.push(plain(String::new()));
                            task_lines.push(label("Risks"));
                            for risk in &task.risks {
                                task_lines.push(plain(format!("- {risk}")));
                            }
                        }
                        let text = task_lines
                            .iter()
                            .map(|(_, t)| t.as_str())
                            .collect::<Vec<_>>()
                            .join(" ");
                        records.push(Rc::new(DetailRecord {
                            lines: task_lines,
                            search_text: text,
                            raw_value: self.task.clone(),
                        }));
                    }
                    if let Some(agent) = agent {
                        let mut act_lines = vec![plain(String::new()), label("Now")];
                        act_lines.push(plain(if agent.running {
                            format!("working: {}", agent.activity)
                        } else {
                            format!("idle · {}", agent.activity)
                        }));
                        let text = act_lines
                            .iter()
                            .map(|(_, t)| t.as_str())
                            .collect::<Vec<_>>()
                            .join(" ");
                        records.push(Rc::new(DetailRecord {
                            lines: act_lines,
                            search_text: text,
                            raw_value: Some(serde_json::json!({
                                "running": agent.running,
                                "activity": agent.activity,
                            })),
                        }));
                    } else if task.is_none() {
                        records.push(Rc::new(DetailRecord {
                            lines: vec![plain("This agent has not worked yet.".into())],
                            search_text: "This agent has not worked yet.".into(),
                            raw_value: None,
                        }));
                    }
                }
                if !self.show_info && !is_conv {
                    let mut keys = self.session_keys();
                    let has_events = |key: &String| view.agents.get(key).is_some_and(|agent|
                        !agent.feed.is_empty() || !agent.session.is_empty() || agent.pid.is_some());
                    if keys.iter().any(has_events) {
                        keys.retain(has_events);
                    }
                    // Keep the currently running role at the tail of the shared
                    // transcript, including a worker repairing a verifier round.
                    keys.sort_by_key(|key| view.agents.get(key).map(|a| (a.running, a.feed_revision)));
                    for key in keys {
                        let role = key.split_once(':').map_or(key.as_str(), |(role, _)| role);
                        let heading = format!("Session · {role}");
                        records.push(Rc::new(DetailRecord {
                            lines: vec![plain(String::new()), label(&heading)],
                            search_text: heading,
                            raw_value: Some(serde_json::json!({ "agent": key })),
                        }));
                        let agent = self.round_agents.get(&key).or_else(|| self.selected_round.is_none().then(||view.agents.get(&key)).flatten());
                        if let Some(agent) = agent
                            && (!agent.feed.is_empty() || !agent.session.is_empty() || agent.pid.is_some())
                        {
                            let pid_label = agent
                                .pid
                                .map(|p| format!("PID {p}"))
                                .unwrap_or_else(|| "no active PID".into());
                            let state = if agent.running { "running" } else { "idle" };
                            let provider = if agent.provider.is_empty() {
                                "agent"
                            } else {
                                agent.provider.as_str()
                            };
                            let header = format!("{provider} · {state} · {pid_label}");
                            records.push(Rc::new(DetailRecord {
                                lines: vec![(
                                    Style::default()
                                        .fg(Color::Cyan)
                                        .add_modifier(Modifier::BOLD),
                                    header.clone(),
                                )],
                                search_text: format!("session {header}"),
                                raw_value: Some(serde_json::json!({
                                    "agent": key,
                                    "status": state,
                                    "pid": agent.pid,
                                })),
                            }));
                            if agent.feed_dropped > 0 {
                                records.push(Rc::new(DetailRecord {
                                    lines: vec![(
                                        dim(),
                                        format!("… {} earlier entries are not shown", agent.feed_dropped),
                                    )],
                                    search_text: "earlier entries".into(),
                                    raw_value: None,
                                }));
                            }
                            if agent.feed.is_empty() {
                                // Older event logs only kept the short summary lines.
                                for line in &agent.session {
                                    records.push(Rc::new(DetailRecord {
                                        lines: vec![(Style::default(), line.clone())],
                                        search_text: line.clone(),
                                        raw_value: Some(serde_json::json!({ "session": line })),
                                    }));
                                }
                            }
                            records.extend(self.session_cache.entry(key.clone()).or_default()
                                .records(provider, &agent.feed, self.expand_prompts, agent.feed_revision).iter().cloned());
                        } else {
                            records.push(Rc::new(DetailRecord {
                                lines: vec![(
                                    dim(),
                                    "No session events for this agent. [historical data unavailable or agent has not worked yet]. Tool events will appear during work.".into(),
                                )],
                                search_text: "no session events historical data unavailable".into(),
                                raw_value: None,
                            }));
                        }
                    }
                }
            }
            1 if self.task.as_ref().is_some_and(|task| task["messages"].is_array()) => {
                for message in self.task.as_ref().unwrap()["messages"].as_array().unwrap() {
                    let text = format!("{} -> {} · {}\n{}",message["sender"].as_str().unwrap_or_default(),message["recipient"].as_str().unwrap_or_default(),
                        if message["delivered_turn"].is_string() {"delivered"} else {"queued"},message["text"].as_str().unwrap_or_default());
                    records.push(Rc::new(DetailRecord {lines:vec![plain(text.clone())],search_text:text,raw_value:Some(message.clone())}));
                }
                if records.is_empty() { records.push(Rc::new(DetailRecord{lines:vec![(dim(),"No messages in this round.".into())],search_text:String::new(),raw_value:None})); }
            }
            1 => {
                let mailbox_items = view.mailbox.get(&self.key);
                match mailbox_items {
                    Some(mail) if !mail.is_empty() => {
                        for item in mail {
                            records.push(Rc::new(DetailRecord {
                                lines: vec![plain(item.clone())],
                                search_text: item.clone(),
                                raw_value: Some(serde_json::json!({ "mail": item })),
                            }));
                        }
                    }
                    _ => {
                        let id = self.task_id().unwrap_or(&self.key);
                        records.push(Rc::new(DetailRecord {
                            lines: vec![(
                                dim(),
                                format!("No messages. [no recorded messages in history for this agent]. Use '@{id} message' in chat to write to this agent."),
                            )],
                            search_text: "no messages".into(),
                            raw_value: None,
                        }));
                    }
                }
            }
            REVIEW if self.round_agents.contains_key(&self.key) => {
                let lines = self.round_agents[&self.key].feed.iter().filter_map(|block| match block {
                    super::session::Block::Final(text) => Some(plain(text.clone())), _ => None,
                }).collect::<Vec<_>>();
                records.push(Rc::new(DetailRecord {search_text:lines.iter().map(|(_,text)| text.as_str()).collect::<Vec<_>>().join(" "),lines,raw_value:None}));
            }
            REVIEW => match view.reviews.get(&self.key) {
                Some(review) if !review.is_empty() => {
                    records.push(Rc::new(DetailRecord {
                        lines: review.iter().cloned().map(plain).collect(),
                        search_text: review.join(" "),
                        raw_value: Some(serde_json::json!({ "review": review })),
                    }));
                }
                _ => records.push(Rc::new(DetailRecord {
                    lines: vec![(
                        dim(),
                        "No review yet. The latest verdict appears here once this agent has reviewed something."
                            .into(),
                    )],
                    search_text: "no review".into(),
                    raw_value: None,
                })),
            },
            COMMENTS => {
                let comments = self.comments(view);
                if comments.is_empty() {
                    records.push(Rc::new(DetailRecord {
                        lines: vec![(
                            dim(),
                            "No comments. [no comments in history for this element]. Press 'c' to add a comment to this element.".into(),
                        )],
                        search_text: "no comments".into(),
                        raw_value: None,
                    }));
                } else {
                    self.comment = self.comment.min(comments.len().saturating_sub(1));
                    for (index, comment) in comments.iter().enumerate() {
                        let mut style = if comment.status == "open" {
                            Style::default()
                        } else {
                            dim()
                        };
                        if index == self.comment {
                            style = style.add_modifier(Modifier::REVERSED);
                        }
                        let state = view.comment_state(comment.id);
                        let stale = if view.stale(comment) {
                            " · from previous plan"
                        } else {
                            ""
                        };
                        let scope = view.comment_scopes.get(&comment.id).map_or(" · shared", |(round,run)| if round.is_none() && run.is_none() {" · shared"} else {""});
                        let header_str = format!(
                            "#{} {} · {state}{stale}{scope}",
                            comment.id,
                            super::view::target_name(&comment.target)
                        );
                        let text_str = format!("  {}", comment.text);
                        let search_text = format!("{header_str} {text_str}");
                        records.push(Rc::new(DetailRecord {
                            lines: vec![(style, header_str), (Style::default(), text_str)],
                            search_text,
                            raw_value: Some(serde_json::to_value(comment).unwrap_or_default()),
                        }));
                    }
                }
            }
            CHECKS => match (&self.task, task) {
                (_, None) => records.push(Rc::new(DetailRecord {
                    lines: vec![(
                        dim(),
                        "Tasks have checks: task risks and its kind's checklist, accounted for by the verifier. [no check data in history]".into(),
                    )],
                    search_text: "tasks have checks".into(),
                    raw_value: None,
                })),
                (None, Some(_)) => records.push(Rc::new(DetailRecord {
                    lines: vec![(dim(), "Loading…".into())],
                    search_text: "loading".into(),
                raw_value: None,
                })),
                (Some(stored), Some(_)) => {
                    let rows = check_rows(stored);
                    self.check = self.check.min(rows.len().saturating_sub(1));
                    let task_id = self.task_id().unwrap_or_default().to_owned();
                    let now: Vec<&str> = rows
                        .iter()
                        .filter(|row| row.state == "checking")
                        .map(|row| row.id.as_str())
                        .collect();
                    let settled = rows.iter().filter(|row| row.state == "settled").count();
                    let issues = rows
                        .iter()
                        .filter(|row| matches!(row.status.as_deref(), Some("issue" | "blocked")))
                        .count();
                    let mut summary = format!("{settled}/{} checks settled", rows.len());
                    if issues > 0 {
                        summary.push_str(&format!(" · {issues} with issues"));
                    }
                    let working = if now.is_empty() {
                        String::new()
                    } else {
                        format!("Checking now: {}", now.join(", "))
                    };
                    let mut head = vec![label(&summary)];
                    if !working.is_empty() {
                        head.push((Style::default().fg(Color::Cyan), working));
                    }
                    if stored["verdict"].is_null() && stored["checks"].as_array().is_none_or(Vec::is_empty) {
                        head.push((
                            dim(),
                            "Verifier has not started yet; checks appear here as it settles them [no historical data]".into(),
                        ));
                    }
                    head.push((dim(), "↑/↓ select a check · c comment on it".into()));
                    head.push(plain(String::new()));
                    records.push(Rc::new(DetailRecord {
                        search_text: head
                            .iter()
                            .map(|(_, text)| text.as_str())
                            .collect::<Vec<_>>()
                            .join(" "),
                        lines: head,
                        raw_value: None,
                    }));
                    for (index, row) in rows.iter().enumerate() {
                        let selected = index == self.check;
                        let mut header_style = row.style;
                        if selected {
                            header_style = header_style.add_modifier(Modifier::REVERSED);
                        }
                        let comments = view.check_comments(&task_id, &row.id);
                        let count = if comments.is_empty() {
                            String::new()
                        } else {
                            format!(" [{} comment{}]", comments.len(), if comments.len() == 1 { "" } else { "s" })
                        };
                        let cursor = if selected { "▸" } else { " " };
                        let mut lines = vec![(
                            header_style,
                            format!("{cursor}{} {} {}: {}{count}", row.mark, row.id, row.kind, row.text),
                        )];
                        match (&row.note, row.state) {
                            (Some(note), _) => {
                                lines.push((Style::default(), format!("    evidence: {note}")))
                            }
                            (None, "checking") => {
                                lines.push((dim(), "    checking… no evidence yet".into()))
                            }
                            (None, "settled") => lines.push((
                                dim(),
                                "    settled without evidence [none given in history]".into(),
                            )),
                            (None, "unaccounted") => lines.push((
                                dim(),
                                "    unaccounted for [no evidence in history]".into(),
                            )),
                            _ => lines.push((dim(), "    not checked yet".into())),
                        }
                        for comment in comments {
                            let state = view.comment_state(comment.id);
                            lines.push((
                                Style::default().fg(Color::Magenta),
                                format!("    💬 #{} {state}: {}", comment.id, comment.text),
                            ));
                        }
                        records.push(Rc::new(DetailRecord {
                            search_text: lines
                                .iter()
                                .map(|(_, text)| text.as_str())
                                .collect::<Vec<_>>()
                                .join(" "),
                            lines,
                            raw_value: Some(serde_json::json!({
                                "id": row.id, "state": row.state,
                                "status": row.status, "evidence": row.note,
                            })),
                        }));
                    }
                }
            },
            _ => match (&self.task, task) {
                (_, None) => records.push(Rc::new(DetailRecord {
                    lines: vec![(
                        dim(),
                        "Task workers and verifiers have results; plan verifier and auditor results are in Overview. [no historical result data]".into(),
                    )],
                    search_text: "workers and verifiers have results".into(),
                    raw_value: None,
                })),
                (None, Some(_)) => records.push(Rc::new(DetailRecord {
                    lines: vec![(dim(), "Loading…".into())],
                    search_text: "loading".into(),
                    raw_value: None,
                })),
                (Some(stored), Some(_)) if verifier => match &stored["verdict"] {
                    Value::Null => records.push(Rc::new(DetailRecord {
                        lines: vec![(
                            dim(),
                            "Verifier has not evaluated the task yet (verdict will appear after verification). [no historical verdict data]".into(),
                        )],
                        search_text: "verifier has not evaluated".into(),
                        raw_value: None,
                    })),
                    verdict => {
                        let mut lines = vec![];
                        lines.extend(
                            super::super::cli::review_lines("verdict", verdict)
                                .into_iter()
                                .map(plain),
                        );
                        if let Some(findings) = verdict["findings"].as_array() {
                            if findings.is_empty() {
                                lines.push(plain("Findings: no issues (verifier approved the task without notes).".into()));
                            } else {
                                lines.push(label("Verifier findings"));
                                for f in findings {
                                    let sev = f["severity"].as_str().unwrap_or("note");
                                    let issue = f["issue"].as_str().unwrap_or_default();
                                    let where_str = f["where"]
                                        .as_str()
                                        .map(|w| format!(" ({w})"))
                                        .unwrap_or_default();
                                    lines.push(plain(format!("• [{sev}] {issue}{where_str}")));
                                }
                            }
                        } else {
                            lines.push((
                                dim(),
                                "Findings: [no historical findings data]".into(),
                            ));
                        }
                        let text = lines
                            .iter()
                            .map(|(_, t)| t.as_str())
                            .collect::<Vec<_>>()
                            .join(" ");
                        records.push(Rc::new(DetailRecord {
                            lines,
                            search_text: text,
                            raw_value: Some(verdict.clone()),
                        }));
                    }
                },
                (Some(stored), Some(_)) => {
                    let mut art_lines = vec![];
                    art_lines.push(label("Artifacts and changes"));
                    if let (Some(base), Some(result)) =
                        (stored["base"].as_str(), stored["result"].as_str())
                    {
                        art_lines.push(plain(format!(
                            "Git changes: {}..{}",
                            &base[..base.len().min(10)],
                            &result[..result.len().min(10)]
                        )));
                    } else {
                        art_lines.push((
                            dim(),
                            "Artifacts: no recorded git changes [historical data unavailable]".into(),
                        ));
                    }
                    let art_text = art_lines
                        .iter()
                        .map(|(_, t)| t.as_str())
                        .collect::<Vec<_>>()
                        .join(" ");
                    records.push(Rc::new(DetailRecord {
                        lines: art_lines,
                        search_text: art_text,
                        raw_value: Some(serde_json::json!({
                            "base": stored["base"],
                            "result": stored["result"],
                        })),
                    }));

                    let mut rep_lines = vec![];
                    rep_lines.push(label("Report"));
                    match stored["report"].as_str() {
                        Some(report) => {
                            rep_lines.push(plain(report.to_owned()));
                        }
                        None => {
                            rep_lines.push((
                                dim(),
                                "Worker has not submitted a report yet (work in progress or waiting to start) [no historical report data].".into(),
                            ));
                        }
                    }
                    let rep_text = rep_lines
                        .iter()
                        .map(|(_, t)| t.as_str())
                        .collect::<Vec<_>>()
                        .join(" ");
                    records.push(Rc::new(DetailRecord {
                        lines: rep_lines,
                        search_text: rep_text,
                        raw_value: Some(serde_json::json!({
                            "report": stored["report"],
                        })),
                    }));

                    let mut ev_lines = vec![label("Evidence and findings")];
                    if let Some(verdict) = stored.get("verdict").filter(|v| !v.is_null()) {
                        if let Some(findings) = verdict["findings"].as_array() {
                            if findings.is_empty() {
                                ev_lines.push(plain("Findings: no issues.".into()));
                            } else {
                                for f in findings {
                                    let sev = f["severity"].as_str().unwrap_or("note");
                                    let issue = f["issue"].as_str().unwrap_or_default();
                                    ev_lines.push(plain(format!("• [{sev}] {issue}")));
                                }
                            }
                        }
                    } else {
                        ev_lines.push((dim(), "Evidence: [no attached evidence in history / awaiting verification]".into()));
                    }
                    let ev_text = ev_lines
                        .iter()
                        .map(|(_, t)| t.as_str())
                        .collect::<Vec<_>>()
                        .join(" ");
                    records.push(Rc::new(DetailRecord {
                        lines: ev_lines,
                        search_text: ev_text,
                        raw_value: Some(stored["verdict"].clone()),
                    }));
                }
            },
        }
        records
    }

    fn draw_overview(&mut self, frame: &mut Frame, view: &View, body: Rect, theme: &Theme) {
        let keys = self.session_keys();
        self.active_panel = self.active_panel.min(keys.len().saturating_sub(1));
        let areas = if keys.len() == 2 && body.width >= 80 {
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(body)
                .to_vec()
        } else {
            vec![body]
        };
        for (visible, area) in areas.iter().enumerate() {
            let panel = if areas.len() == 1 {
                self.active_panel
            } else {
                visible
            };
            let key = &keys[panel];
            let active = panel == self.active_panel;
            let running = self
                .round_agents
                .get(key)
                .or_else(|| view.agents.get(key))
                .is_some_and(|agent| agent.running);
            let mut block = Block::default()
                .borders(Borders::ALL)
                .border_style(if active { theme.dim() } else { theme.faint() })
                .title(theme.clean(&format!(
                    " {}Session · {} · {} ",
                    if active { "> " } else { "" },
                    key.split(':').next().unwrap_or(key),
                    if running && self.selected_round.is_none() {
                        "LIVE"
                    } else {
                        "history"
                    }
                )));
            if theme.ascii {
                block = block.border_set(ratatui::symbols::border::Set {
                    top_left: "+",
                    top_right: "+",
                    bottom_left: "+",
                    bottom_right: "+",
                    vertical_left: "|",
                    vertical_right: "|",
                    horizontal_top: "-",
                    horizontal_bottom: "-",
                });
            }
            let inner = block.inner(*area);
            frame.render_widget(block, *area);
            let [inner, composer_area] =
                Layout::vertical([Constraint::Min(1), Constraint::Length(4.min(inner.height))])
                    .areas(inner);
            let draft_key = (self.conversation.clone(), self.composer_recipient(key));
            let drafts = self.drafts.borrow();
            let composer = drafts.get(&draft_key);
            let input = composer.map(|composer| &composer.input);
            let status = if self.selected_round.is_some() {
                "Return to current round to send"
            } else if composer.is_some_and(|composer| composer.sending) {
                "Sending..."
            } else if let Some(error) = composer.and_then(|composer| composer.error.as_deref()) {
                error
            } else if active && self.editing {
                "Enter send · Alt+Enter newline · Esc navigation"
            } else {
                composer
                    .and_then(|composer| composer.delivery.as_deref())
                    .unwrap_or("i message")
            };
            let status_area = Rect::new(
                composer_area.x,
                composer_area.y,
                composer_area.width,
                1.min(composer_area.height),
            );
            frame.render_widget(
                Paragraph::new(theme.clean(status)).style(theme.dim()),
                status_area,
            );
            let input_area = Rect::new(
                composer_area.x,
                composer_area.y.saturating_add(1),
                composer_area.width,
                composer_area.height.saturating_sub(1),
            );
            self.input_width[panel] = usize::from(input_area.width).max(1);
            if let Some(input) = input {
                let text = theme.clean(&input.text);
                let lines = wrap(&text, usize::from(input_area.width));
                let (column, row) = input.visual_cursor(usize::from(input_area.width));
                let start = row.saturating_sub(usize::from(input_area.height.saturating_sub(1)));
                frame.render_widget(
                    Paragraph::new(
                        lines
                            .into_iter()
                            .skip(start)
                            .take(usize::from(input_area.height))
                            .map(Line::raw)
                            .collect::<Vec<_>>(),
                    ),
                    input_area,
                );
                if active && self.editing && input_area.width > 0 && input_area.height > 0 {
                    frame.set_cursor_position((
                        input_area
                            .x
                            .saturating_add(column.min(input_area.width.saturating_sub(1))),
                        input_area
                            .y
                            .saturating_add(row.saturating_sub(start) as u16)
                            .min(input_area.bottom().saturating_sub(1)),
                    ));
                }
            }
            drop(drafts);
            let document_area = Rect::new(
                inner.x,
                inner.y,
                inner.width.saturating_sub(1),
                inner.height,
            );
            if let Some(document) = self.documents.get(key) {
                let requested = if active {
                    self.scroll
                } else {
                    self.panel_scroll[panel]
                };
                match document.viewport(
                    usize::from(document_area.width),
                    usize::from(document_area.height),
                    requested,
                    active && self.follow && self.panel_expanded[panel],
                    self.panel_expanded[panel],
                ) {
                    Ok(Some(page)) => {
                        self.panel_scroll[panel] = page.scroll;
                        if active {
                            self.scroll = page.scroll;
                            self.text_area = document_area;
                        }
                        frame.render_widget(
                            Paragraph::new(
                                page.lines
                                    .into_iter()
                                    .map(|(dim, text)| {
                                        Line::styled(
                                            theme.clean(&text),
                                            if dim { theme.dim() } else { Style::default() },
                                        )
                                    })
                                    .collect::<Vec<_>>(),
                            ),
                            document_area,
                        );
                        if page.total > usize::from(document_area.height) && inner.width > 0 {
                            let bottom =
                                page.total.saturating_sub(usize::from(document_area.height));
                            let mut state = ScrollbarState::new(bottom.saturating_add(1))
                                .position(page.scroll)
                                .viewport_content_length(usize::from(document_area.height));
                            frame.render_stateful_widget(
                                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                                    .begin_symbol(None)
                                    .end_symbol(None),
                                inner,
                                &mut state,
                            );
                        }
                    }
                    Ok(None) => {
                        frame.render_widget(
                            Paragraph::new("Preparing session view...").style(theme.dim()),
                            document_area,
                        );
                    }
                    Err(error) => {
                        frame.render_widget(
                            Paragraph::new(theme.clean(&error.to_string())).style(theme.dim()),
                            document_area,
                        );
                    }
                }
                continue;
            }
            let agent = self.round_agents.get(key).or_else(|| {
                (self.selected_round.is_none())
                    .then(|| view.agents.get(key))
                    .flatten()
            });
            let revision = agent.and_then(|agent| agent.feed_revision);
            if revision.is_none()
                || self.overview_cache[panel].as_ref().is_none_or(|cache| {
                    cache.key != *key
                        || cache.revision != revision
                        || cache.expanded != self.panel_expanded[panel]
                })
            {
                let lines = agent.map_or_else(
                    || vec![(theme.dim(), "No session events for this agent.".into())],
                    |agent| {
                        super::session::overview(
                            &agent.provider,
                            &agent.feed,
                            self.panel_expanded[panel],
                        )
                    },
                );
                self.overview_cache[panel] = Some(OverviewCache {
                    key: key.clone(),
                    revision,
                    expanded: self.panel_expanded[panel],
                    record: Rc::new(DetailRecord {
                        search_text: String::new(),
                        raw_value: None,
                        lines,
                    }),
                });
            }
            let record = Rc::clone(
                &self.overview_cache[panel]
                    .as_ref()
                    .expect("cached overview")
                    .record,
            );
            let lines = self.panel_layout[panel].update(
                vec![record],
                "",
                false,
                usize::from(document_area.width),
            );
            let bottom = lines.len().saturating_sub(usize::from(inner.height));
            let scroll = if active && self.follow && self.panel_expanded[panel] {
                bottom
            } else if active {
                self.scroll.min(bottom)
            } else {
                self.panel_scroll[panel].min(bottom)
            };
            self.panel_scroll[panel] = scroll;
            if active {
                self.scroll = scroll;
                self.text_area = document_area;
            }
            let end = scroll
                .saturating_add(usize::from(inner.height))
                .min(lines.len());
            let visible = lines[scroll..end]
                .iter()
                .cloned()
                .map(|line| {
                    Line::from(
                        line.spans
                            .into_iter()
                            .map(|span| {
                                Span::styled(
                                    theme.clean(&span.content),
                                    if theme.color {
                                        span.style
                                    } else {
                                        Style {
                                            fg: None,
                                            bg: None,
                                            ..span.style
                                        }
                                    },
                                )
                            })
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>();
            frame.render_widget(Paragraph::new(visible), document_area);
            if lines.len() > usize::from(inner.height) && inner.width > 0 {
                let mut state = ScrollbarState::new(bottom.saturating_add(1))
                    .position(scroll)
                    .viewport_content_length(usize::from(inner.height));
                let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .begin_symbol(None)
                    .end_symbol(None)
                    .track_symbol(Some(theme.glyphs().track))
                    .thumb_symbol(theme.glyphs().thumb)
                    .track_style(theme.faint())
                    .thumb_style(theme.dim());
                frame.render_stateful_widget(scrollbar, inner, &mut state);
            }
        }
    }

    pub fn draw(&mut self, frame: &mut Frame, view: &View) {
        if !tabs_for(&self.key).contains(&self.tab) {
            self.tab = 0;
        }
        let area = frame.area();
        if area.width == 0 || area.height == 0 {
            return;
        }
        let theme = Theme::from_env();
        let summary = self.summary(view, &theme);
        // Even a five-row terminal keeps room for session content.
        let summary_height = if self.tab == 0 {
            (summary.len() as u16).min(area.height.saturating_sub(6))
        } else {
            0
        };
        let [header, summary_area, mut body, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(summary_height),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .areas(area);

        let mut tabs_line = format_tabs(&self.key, self.tab, usize::from(header.width));
        if self.raw {
            tabs_line.spans.push(Span::styled(
                " [RAW] ",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        if !self.query.is_empty() {
            tabs_line.spans.push(Span::styled(
                format!(" [🔍 \"{}\"]", self.query),
                Style::default().fg(Color::Cyan),
            ));
        }
        for span in &mut tabs_line.spans {
            span.content = theme.clean(&span.content).into();
            if !theme.color {
                span.style = Style {
                    fg: None,
                    bg: None,
                    ..span.style
                };
            }
        }
        frame.render_widget(Paragraph::new(tabs_line), header);
        frame.render_widget(Paragraph::new(summary).style(theme.bar()), summary_area);

        if body.height > 1 && self.key != "conversation" {
            let latest = self
                .task_id()
                .and_then(|id| view.tasks.iter().find(|task| task.id == id))
                .map_or_else(
                    || {
                        self.task
                            .as_ref()
                            .and_then(|task| task["round"].as_u64())
                            .unwrap_or(0)
                    },
                    |task| task.round,
                );
            let selected = self.selected_round.unwrap_or(latest);
            let label = format!(
                "Round {} · {} · Left/Right choose · Tab tabs",
                selected,
                if self.selected_round.is_none() {
                    "current"
                } else {
                    "history · return to current to send"
                }
            );
            frame.render_widget(
                Paragraph::new(theme.clean(&label)).style(theme.dim()),
                Rect::new(body.x, body.y, body.width, 1),
            );
            body.y = body.y.saturating_add(1);
            body.height = body.height.saturating_sub(1);
        }
        if self.tab == 0
            && !self.show_info
            && self.key != "conversation"
            && self.query.is_empty()
            && !self.raw
        {
            self.draw_overview(frame, view, body, &theme);
            let hint = "i message · Ctrl+W panel · Enter trace · m metadata · Left/Right rounds · Esc back";
            frame.render_widget(Paragraph::new(theme.clean(hint)).style(theme.dim()), footer);
            return;
        }

        let block = if self.tab == 0 {
            let title = if self.show_info {
                " Task info · m session "
            } else if self.follow {
                " Session · LIVE "
            } else {
                " Session · history · G live "
            };
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(theme.faint())
                .style(Style::default())
                .title(title)
                .padding(Padding::horizontal(u16::from(body.width >= 20)))
        } else {
            Block::default().borders(Borders::TOP)
        };
        let inner = block.inner(body);
        frame.render_widget(block, body);
        self.text_area = Rect::new(
            inner.x,
            inner.y,
            inner.width.saturating_sub(1),
            inner.height,
        );
        let width = usize::from(inner.width.saturating_sub(1));
        let document_key = match self.tab {
            1 => Some("messages"),
            RESULT => Some("result"),
            REVIEW => Some("review"),
            _ => None,
        };
        if let Some(document) = document_key.and_then(|key| self.documents.get(key)) {
            match document.viewport(width, usize::from(inner.height), self.scroll, false, false) {
                Ok(Some(page)) => {
                    self.scroll = page.scroll;
                    frame.render_widget(
                        Paragraph::new(
                            page.lines
                                .into_iter()
                                .map(|(_, text)| Line::raw(theme.clean(&text)))
                                .collect::<Vec<_>>(),
                        ),
                        self.text_area,
                    );
                    if page.total > usize::from(inner.height) && inner.width > 0 {
                        let mut state = ScrollbarState::new(
                            page.total
                                .saturating_sub(usize::from(inner.height))
                                .saturating_add(1),
                        )
                        .position(page.scroll)
                        .viewport_content_length(usize::from(inner.height));
                        frame.render_stateful_widget(
                            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                                .begin_symbol(None)
                                .end_symbol(None),
                            inner,
                            &mut state,
                        );
                    }
                }
                Ok(None) => {
                    frame.render_widget(
                        Paragraph::new("Preparing history view...").style(theme.dim()),
                        self.text_area,
                    );
                }
                Err(error) => {
                    frame.render_widget(
                        Paragraph::new(theme.clean(&error.to_string())).style(theme.dim()),
                        self.text_area,
                    );
                }
            }
            let hint = details_footer_hint_for(&self.key, self.tab, usize::from(footer.width));
            frame.render_widget(
                Paragraph::new(theme.clean(&hint)).style(theme.dim()),
                footer,
            );
            return;
        }

        let records = self.records(view);
        let lines = self
            .layout_cache
            .update(records, &self.query, self.raw, width);
        let height = usize::from(inner.height);
        let bottom = lines.len().saturating_sub(height);
        if self.tab == CHECKS
            && std::mem::take(&mut self.reveal_check)
            && let Some(at) = lines
                .iter()
                .position(|line| line.style.add_modifier.contains(Modifier::REVERSED))
        {
            if at < self.scroll {
                self.scroll = at;
            } else if at >= self.scroll.saturating_add(height) {
                self.scroll = at + 1 - height;
            }
        }
        if self.tab == 0 && !self.show_info && self.follow {
            self.scroll = bottom;
        } else {
            self.scroll = self.scroll.min(bottom);
            // Scrolling back down to the end picks the tail up again.
            self.follow = self.tab == 0 && !self.show_info && self.scroll >= bottom;
        }
        let end = (self.scroll + height).min(lines.len());
        frame.render_widget(
            Paragraph::new(lines[self.scroll..end].to_vec()),
            self.text_area,
        );
        if lines.len() > height && inner.width > 0 {
            let mut state = ScrollbarState::new(bottom.saturating_add(1))
                .position(self.scroll)
                .viewport_content_length(height);
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(Some(theme.glyphs().track))
                .thumb_symbol(theme.glyphs().thumb)
                .track_style(theme.faint())
                .thumb_style(theme.dim());
            frame.render_stateful_widget(scrollbar, inner, &mut state);
        }

        let hint = if self.searching {
            format!(
                "Search records: {}_ (Enter: confirm · Esc: cancel)",
                self.query
            )
        } else {
            details_footer_hint_for(&self.key, self.tab, usize::from(footer.width))
        };
        frame.render_widget(Paragraph::new(Span::styled(hint, dim())), footer);
    }
}

pub fn details_footer_hint_for(key: &str, tab: usize, width: usize) -> String {
    let controls = details_footer_hint(tab, width);
    if width >= 45 {
        format!("1–{} / {controls}", tabs_for(key).len())
    } else {
        controls.to_owned()
    }
}

pub fn details_footer_hint(tab: usize, width: usize) -> &'static str {
    if tab == 0 {
        if width >= 90 {
            "Tab tabs · ←/→ rounds · ↑/↓ PgUp/PgDn · gg/G top/live · m info · / search · c comment · Esc back"
        } else if width >= 65 {
            "Tab tabs · ←/→ rounds · ↑/↓ scroll · gg/G top/live · m info · / search · Esc back"
        } else if width >= 45 {
            "Tab tabs · ←/→ rounds · gg/G top/live · m info · Esc"
        } else {
            "gg/G · m info · Esc back"
        }
    } else if tab == CHECKS {
        if width >= 80 {
            "←/→ tab · ↑/↓ select check · c comment on check · / search · v raw · d diff · ? help · Esc back"
        } else if width >= 45 {
            "←/→ tab · ↑/↓ select · c comment · Esc"
        } else {
            "c com · Esc back"
        }
    } else if tab == COMMENTS {
        if width >= 95 {
            "←/→ tab · ↑/↓ select · / search · v raw · c comment · r resolve · o reopen · x delete · d diff · ? help · Esc back"
        } else if width >= 65 {
            "Tab tabs · ←/→ rounds · ↑/↓ select · / search · v raw · c add · r/o/x status · d diff · Esc back"
        } else if width >= 45 {
            "←/→ tab · ↑/↓ select · c add · r/o/x · Esc"
        } else {
            "c com · Esc back"
        }
    } else {
        if width >= 90 {
            "Tab tabs · ←/→ rounds · ↑/↓ PgUp/PgDn · gg/G top/end · / search · c comment · d diff · Esc back"
        } else if width >= 65 {
            "Tab tabs · ←/→ rounds · ↑/↓ scroll · gg/G top/end · / search · Esc back"
        } else if width >= 45 {
            "Tab tabs · ←/→ rounds · gg/G top/end · Esc"
        } else {
            "gg/G · ↑/↓ · Esc back"
        }
    }
}

pub fn format_tabs<'a>(agent_key: &str, active_tab: usize, max_width: usize) -> Line<'a> {
    let agent = format!(" {} ", agent_name(agent_key));
    let agent_w = agent.len();
    let visible = tabs_for(agent_key);
    let name = |slot: usize| TABS[visible[slot]];
    let active = visible
        .iter()
        .position(|tab| *tab == active_tab)
        .unwrap_or(0);
    let full_tabs_len: usize = agent_w
        + (0..visible.len())
            .map(|i| name(i).len() + if i == active { 4 } else { 3 })
            .sum::<usize>();

    if full_tabs_len <= max_width {
        let mut spans = vec![Span::styled(
            agent,
            Style::default().add_modifier(Modifier::BOLD),
        )];
        for index in 0..visible.len() {
            spans.push(Span::raw(" "));
            spans.push(if index == active {
                Span::styled(
                    format!("[{}]", name(index)),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled(format!(" {} ", name(index)), dim())
            });
        }
        return Line::from(spans);
    }

    // Narrow terminal: ensure active tab is visible with sliding window
    let mut spans = vec![Span::styled(
        agent,
        Style::default().add_modifier(Modifier::BOLD),
    )];

    let mut left = active;
    let mut right = active;
    let mut current_len = agent_w + name(active).len() + 4;
    while left > 0 || right + 1 < visible.len() {
        let mut expanded = false;
        if right + 1 < visible.len() {
            let next_len = name(right + 1).len() + 3;
            if current_len + next_len + 2 <= max_width {
                right += 1;
                current_len += next_len;
                expanded = true;
            }
        }
        if left > 0 {
            let prev_len = name(left - 1).len() + 3;
            if current_len + prev_len + 2 <= max_width {
                left -= 1;
                current_len += prev_len;
                expanded = true;
            }
        }
        if !expanded {
            break;
        }
    }

    if left > 0 {
        spans.push(Span::styled("◀ ", dim()));
    }
    for index in left..=right {
        spans.push(if index == active {
            Span::styled(
                format!("[{}] ", name(index)),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(format!("{} ", name(index)), dim())
        });
    }
    if right + 1 < visible.len() {
        spans.push(Span::styled("▶", dim()));
    }

    Line::from(spans)
}

/// One check of a verifier as the Checks tab shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckRow {
    pub id: String,
    pub kind: &'static str,
    pub text: String,
    pub mark: &'static str,
    pub style: Style,
    /// `pending`, `checking`, `settled` or `unaccounted`.
    pub state: &'static str,
    pub status: Option<String>,
    pub note: Option<String>,
}

/// The verifier's obligations (the task's risks, the common checks, then its
/// kind's checklist) and how far each is: what the verifier reported while it
/// works, else how the latest verdict accounted for it.
pub fn check_rows(stored: &Value) -> Vec<CheckRow> {
    let Ok(spec) = serde_json::from_value::<TaskSpec>(stored["spec"].clone()) else {
        return Vec::new();
    };
    let live = stored["checks"].as_array().filter(|rows| !rows.is_empty());
    let verdict = !stored["verdict"].is_null();
    obligations(&spec)
        .into_iter()
        .map(|(id, text)| {
            let (state, status, note) = match live {
                Some(rows) => match rows.iter().find(|row| row["id"] == id.as_str()) {
                    Some(row) if row["state"] == "settled" => (
                        "settled",
                        row["status"].as_str().map(str::to_ascii_lowercase),
                        row["note"].as_str().filter(|n| !n.is_empty()),
                    ),
                    Some(row) => (
                        "checking",
                        None,
                        row["note"].as_str().filter(|n| !n.is_empty()),
                    ),
                    None => ("pending", None, None),
                },
                None => {
                    let entry = &stored["verdict"]["coverage"][id.as_str()];
                    match entry["status"].as_str() {
                        Some(status) => (
                            "settled",
                            Some(status.to_ascii_lowercase()),
                            entry["note"].as_str(),
                        ),
                        None if verdict => ("unaccounted", None, None),
                        None => ("pending", None, None),
                    }
                }
            };
            let (mark, style) = match (state, status.as_deref()) {
                ("checking", _) => ("◐", Style::default().fg(Color::Cyan)),
                (_, Some("ok")) => ("✓", Style::default().fg(Color::Green)),
                (_, Some("issue")) => ("✗", Style::default().fg(Color::Red)),
                (_, Some("blocked")) => ("!", Style::default().fg(Color::Yellow)),
                (_, Some("n/a")) => ("–", dim()),
                ("unaccounted", _) => ("?", Style::default().fg(Color::Yellow)),
                _ => ("○", dim()),
            };
            let kind = match id.chars().next() {
                Some('R') => "risk",
                Some('V') => "common",
                _ => "check",
            };
            CheckRow {
                id,
                kind,
                text,
                mark,
                style,
                state,
                status,
                note: note.map(str::to_owned),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_cache_reuses_records_and_invalidates_changed_inputs() {
        use super::super::session::Block;
        let mut cache = SessionCache::default();
        let prompt = Block::Prompt((0..10).map(|n| format!("prompt {n}\n")).collect());
        let blocks = vec![prompt.clone(), Block::Text("old".into())];
        cache.records("codex", &blocks, false, None);
        let allocation = cache.records[0].search_text.as_ptr();
        cache.records("codex", &blocks, false, None);
        assert_eq!(cache.records[0].search_text.as_ptr(), allocation);
        let changed = vec![prompt, Block::Text("new".into())];
        cache.records("codex", &changed, false, None);
        assert_eq!(cache.records[0].search_text.as_ptr(), allocation);
        assert!(cache.records[1].search_text.contains("new"));
        cache.records("codex", &changed, true, None);
        assert!(cache.records[0].search_text.contains("prompt 9"));
        cache.records("claude", &changed, true, None);
        assert!(cache.records[0].search_text.contains("> prompt 0"));
        cache.records("claude", &changed[1..], true, None);
        assert_eq!(cache.records.len(), 1);
        assert!(cache.records[0].search_text.contains("new"));
    }

    #[test]
    fn runtime_revision_reuses_shared_records_and_updates_new_entries() {
        use super::super::session::Block;
        let mut cache = SessionCache::default();
        let mut blocks = vec![Block::Text("worker output".into())];
        let first = Rc::clone(&cache.records("codex", &blocks, false, Some(1))[0]);
        let unchanged = Rc::clone(&cache.records("codex", &blocks, false, Some(1))[0]);
        assert!(Rc::ptr_eq(&first, &unchanged));
        blocks.push(Block::Text("new output".into()));
        let updated = cache.records("codex", &blocks, false, Some(2));
        assert!(Rc::ptr_eq(&first, &updated[0]));
        assert!(updated[1].search_text.contains("new output"));
        blocks.remove(0);
        let trimmed = cache.records("codex", &blocks, false, Some(3));
        assert_eq!(trimmed.len(), 1);
        assert!(trimmed[0].search_text.contains("new output"));
    }

    #[test]
    fn redraw_and_scroll_reuse_layout_but_content_and_display_changes_refresh_it() {
        use super::super::session::Block;
        let mut view = View::default();
        view.agents.insert(
            "worker:t1".into(),
            super::super::view::Agent {
                provider: "codex".into(),
                feed: vec![Block::Text("unique transcript entry".into())],
                ..Default::default()
            },
        );
        let mut details = Details::new("worker:t1".into());
        details.panel_expanded[0] = true;
        let mut terminal = Terminal::new(TestBackend::new(120, 20)).unwrap();
        let screen = |terminal: &Terminal<TestBackend>| {
            terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
        };
        terminal.draw(|frame| details.draw(frame, &view)).unwrap();
        assert!(screen(&terminal).contains("unique transcript entry"));
        let original = details.layout_cache.lines.as_ptr();
        terminal.draw(|frame| details.draw(frame, &view)).unwrap();
        assert_eq!(original, details.layout_cache.lines.as_ptr());
        details.scroll = 1;
        terminal.draw(|frame| details.draw(frame, &view)).unwrap();
        assert_eq!(original, details.layout_cache.lines.as_ptr());
        details.scroll = 0;
        details.query = "absent query".into();
        terminal.draw(|frame| details.draw(frame, &view)).unwrap();
        assert!(screen(&terminal).contains("No records matching"));
        assert!(!screen(&terminal).contains("unique transcript entry"));
        details.query = "unique transcript".into();
        details.raw = true;
        terminal.draw(|frame| details.draw(frame, &view)).unwrap();
        assert!(screen(&terminal).contains("\"session\""));
        details.query.clear();
        details.raw = false;
        view.agents.get_mut("worker:t1").unwrap().feed =
            vec![Block::Text("replacement entry".into())];
        terminal.draw(|frame| details.draw(frame, &view)).unwrap();
        assert!(screen(&terminal).contains("replacement entry"));
        assert!(!screen(&terminal).contains("unique transcript entry"));
        let mut narrow = Terminal::new(TestBackend::new(16, 30)).unwrap();
        narrow.draw(|frame| details.draw(frame, &view)).unwrap();
        assert!(
            details.panel_layout[0]
                .lines
                .iter()
                .all(|line| line.width() <= 15)
        );
    }

    #[test]
    fn footer_numbers_match_each_keys_visible_tabs() {
        for key in [
            "worker:t1",
            "verifier:t1",
            "auditor",
            "plan-verifier",
            "orchestrator",
            "conversation",
        ] {
            for tab in tabs_for(key) {
                for width in [45, 65, 80, 120] {
                    let hint = details_footer_hint_for(key, tab, width);
                    assert!(
                        hint.starts_with(&format!("1–{} /", tabs_for(key).len())),
                        "{key}: {hint}"
                    );
                }
                assert!(!details_footer_hint_for(key, tab, 30).contains("1–"));
            }
        }
    }

    #[test]
    fn overview_opens_at_latest_session_for_all_agent_roles() {
        use super::super::session::Block;
        for key in ["worker:t1", "verifier:t1", "orchestrator", "auditor"] {
            let mut view = View::default();
            view.tasks.push(super::super::view::TaskRow {
                id: "t1".into(),
                goal: "Opening summary".into(),
                ..Default::default()
            });
            view.agents.insert(
                key.into(),
                super::super::view::Agent {
                    feed: (0..100)
                        .map(|n| Block::Text(format!("entry {n}")))
                        .collect(),
                    ..Default::default()
                },
            );
            let mut details = Details::new(key.into());
            details.panel_expanded = [true; 2];
            let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
            terminal.draw(|frame| details.draw(frame, &view)).unwrap();
            assert!(details.scroll > 0, "{key}");
            assert!(details.follow, "{key}");
            details.switch_tab(1);
            details.switch_tab(0);
            terminal.draw(|frame| details.draw(frame, &view)).unwrap();
            assert!(details.scroll > 0, "{key} on returning");
            assert!(details.follow, "{key} on returning");
        }
    }

    fn verifier_task(extra: Value) -> Value {
        let mut task = serde_json::json!({
            "spec": {"id": "t1", "title": "T", "goal": "g", "kind": "analysis",
                     "verifier": {"focus": "f", "risks": ["a risk"]}},
        });
        task.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        task
    }

    #[test]
    fn checks_show_live_progress_before_any_verdict() {
        let task = verifier_task(serde_json::json!({"checks": [
            {"id": "R1", "state": "settled", "status": "ok", "note": "a.rs:3"},
            {"id": "V1", "state": "checking", "status": null, "note": ""},
        ]}));
        let rows = check_rows(&task);
        assert_eq!(rows[0].state, "settled");
        assert_eq!(rows[0].note.as_deref(), Some("a.rs:3"));
        assert_eq!(rows[1].state, "checking");
        assert_eq!(rows[2].state, "pending");
    }

    #[test]
    fn checks_fall_back_to_the_verdict_and_flag_what_it_skipped() {
        let task = verifier_task(serde_json::json!({"verdict": {"coverage": {
            "R1": {"status": "issue", "note": "broken"}}}}));
        let rows = check_rows(&task);
        assert_eq!(rows[0].status.as_deref(), Some("issue"));
        assert_eq!(rows[0].note.as_deref(), Some("broken"));
        assert_eq!(rows[1].state, "unaccounted");
    }

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn ac_copy_01_tab_names_preserved_and_windowed_when_narrow() {
        // Wide: 100 columns -> all tabs rendered
        let line = format_tabs("orchestrator", 0, 100);
        let rendered: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        for tab in tabs_for("orchestrator") {
            let tab = TABS[tab];
            assert!(rendered.contains(tab), "missing tab {tab} in {rendered}");
        }
        assert!(!rendered.contains("Result") && !rendered.contains("Checks"));

        // Narrow: 50 columns -> active tab still visible
        let line = format_tabs("worker:t1", RESULT, 50);
        let rendered: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            rendered.contains("[Result]"),
            "active tab Result missing in {rendered}"
        );
    }

    #[test]
    fn details_footer_hints_prioritized_by_width() {
        // COMMENTS tab
        assert!(details_footer_hint(COMMENTS, 100).contains("d diff"));
        assert!(!details_footer_hint(COMMENTS, 30).contains("d diff"));
        assert!(details_footer_hint(COMMENTS, 30).contains("Esc"));

        // Other tabs
        assert!(details_footer_hint(0, 100).contains("PgUp/PgDn"));
        assert!(!details_footer_hint(0, 40).contains("PgUp/PgDn"));
        assert!(details_footer_hint(0, 40).contains("Esc"));
    }

    #[test]
    fn details_empty_states() {
        let view = View::default();
        let mut details = Details::new("worker:t1".into());

        // Overview includes session empty states
        details.tab = 0;
        let lines = details.lines(&view);
        assert!(
            lines
                .iter()
                .any(|(_, text)| text.contains("No session events"))
        );

        // Messages empty state
        details.tab = 1;
        let lines = details.lines(&view);
        assert!(lines.iter().any(|(_, text)| text.contains("No messages")));

        // Tab 5: Comments empty state
        details.tab = COMMENTS;
        let lines = details.lines(&view);
        assert!(lines.iter().any(|(_, text)| text.contains("No comments")));
    }

    #[test]
    fn details_draw_on_tiny_terminals() {
        let view = View::default();
        let mut details = Details::new("orchestrator".into());

        // 60x12 guard floor
        let backend = TestBackend::new(60, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                details.draw(frame, &view);
            })
            .unwrap();

        // 10x4 tiny terminal (must not panic)
        let backend = TestBackend::new(10, 4);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                details.draw(frame, &view);
            })
            .unwrap();

        // 2x2 extreme tiny terminal (must not panic)
        let backend = TestBackend::new(2, 2);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                details.draw(frame, &view);
            })
            .unwrap();
    }

    #[test]
    fn each_agent_has_only_the_tabs_that_mean_something_for_it() {
        let names =
            |key: &str| -> Vec<&str> { tabs_for(key).into_iter().map(|tab| TABS[tab]).collect() };
        assert_eq!(
            names("worker:t1"),
            [
                "Overview",
                "Messages",
                "Result",
                "Checks",
                "Comments",
                "Requirements"
            ]
        );
        assert_eq!(
            names("verifier:t1"),
            [
                "Overview",
                "Messages",
                "Result",
                "Checks",
                "Comments",
                "Requirements"
            ]
        );
        assert_eq!(names("auditor"), ["Overview", "Review", "Comments"]);
        assert_eq!(names("plan-verifier"), ["Overview", "Review", "Comments"]);
        assert_eq!(names("conversation"), ["Overview", "Messages", "Comments"]);

        // Navigation skips what is not there and wraps around.
        let mut details = Details::new("worker:t1".into());
        details.switch_tab(details.neighbour_tab(1));
        assert_eq!(details.tab, 1);
        details.switch_tab(RESULT);
        details.switch_tab(details.neighbour_tab(1));
        assert_eq!(details.tab, CHECKS);
        details.switch_tab(details.neighbour_tab(1));
        assert_eq!(details.tab, COMMENTS);
        details.switch_tab(details.neighbour_tab(1));
        assert_eq!(details.tab, REQUIREMENTS);
        details.switch_tab(details.neighbour_tab(1));
        assert_eq!(details.tab, 0);
        details.switch_tab(details.neighbour_tab(-1));
        assert_eq!(details.tab, REQUIREMENTS);
        // Number keys count the visible tabs equally for both task roles.
        assert_eq!(details.nth_tab(2), Some(RESULT));
        assert_eq!(details.nth_tab(3), Some(CHECKS));
        assert_eq!(details.nth_tab(4), Some(COMMENTS));
        assert_eq!(details.nth_tab(5), Some(REQUIREMENTS));
        assert_eq!(details.nth_tab(6), None);
        let verifier = Details::new("verifier:t1".into());
        assert_eq!(verifier.nth_tab(3), Some(CHECKS));
    }

    #[test]
    fn reviewers_show_their_latest_verdict_on_the_review_tab() {
        let mut view = View::default();
        view.reviews
            .insert("auditor".into(), vec!["✗ failed: missing tests".into()]);
        let mut details = Details::new("auditor".into());
        details.switch_tab(REVIEW);
        let shown: Vec<String> = details.lines(&view).into_iter().map(|(_, l)| l).collect();
        assert_eq!(shown, ["✗ failed: missing tests"]);
        details.switch_tab(0);
        assert!(
            details
                .lines(&view)
                .iter()
                .all(|(_, line)| !line.contains("missing tests"))
        );
        let mut empty = Details::new("plan-verifier".into());
        empty.switch_tab(REVIEW);
        assert!(empty.lines(&View::default())[0].1.contains("No review yet"));
    }
}

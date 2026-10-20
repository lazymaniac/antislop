//! Full-screen details of one agent or conversation: what it was asked,
//! its session history, messages, deliverables, obligations, and comments.

use super::super::tasks::{TaskSpec, obligations, status};
use super::taskinfo;
use super::view::{View, agent_name, dim, duration, status_label, wrap};
use crate::lean::store::SessionInfo;
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use serde_json::Value;
use std::collections::HashMap;

pub const TABS: [&str; 7] = [
    "Overview", "Session", "Messages", "Result", "Checks", "Comments", "Review",
];
pub const RESULT: usize = 3;
pub const CHECKS: usize = 4;
pub const COMMENTS: usize = 5;
pub const REVIEW: usize = 6;

/// The tabs that mean something for an agent: indexes into [`TABS`].
/// A worker has a result but no checks; its verifier has checks but no
/// result; the reviewers and the conversation have no task at all.
pub fn tabs_for(key: &str) -> Vec<usize> {
    match key.split_once(':').map_or(key, |(role, _)| role) {
        "worker" => vec![0, 1, 2, RESULT, COMMENTS],
        "verifier" => vec![0, 1, 2, CHECKS, COMMENTS],
        "plan-verifier" | "auditor" => vec![0, 1, REVIEW, COMMENTS],
        "orchestrator" | "comment-orchestrator" => vec![0, 1, 2, COMMENTS],
        _ => vec![0, 2, COMMENTS],
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DetailRecord {
    pub lines: Vec<(Style, String)>,
    pub search_text: String,
    pub raw_value: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Details {
    pub key: String,
    pub tab: usize,
    /// Lines scrolled down from the top.
    pub scroll: usize,
    /// The task as the daemon stores it, once fetched.
    pub task: Option<Value>,
    /// The task round the last task fetch was issued for, so a reply for a
    /// round that already moved on can be dropped and refetched, not shown.
    pub fetched_round: Option<u64>,
    /// The selected comment on the Comments tab.
    pub comment: usize,
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
    /// Prompts of the Session tab are shown whole instead of folded.
    pub expand_prompts: bool,
    /// The Session tab shows the newest entry and follows new ones until the
    /// user scrolls up.
    pub follow: bool,
}

impl Details {
    pub fn new(key: String) -> Self {
        Details {
            key,
            tab: 0,
            scroll: 0,
            task: None,
            fetched_round: None,
            comment: 0,
            scroll_memory: HashMap::new(),
            query_memory: HashMap::new(),
            query: String::new(),
            searching: false,
            raw: false,
            sessions: Vec::new(),
            history_unavailable: false,
            expand_prompts: false,
            follow: true,
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
        self.scroll_memory
            .insert((self.key.clone(), self.tab), self.scroll);
        self.query_memory
            .insert((self.key.clone(), self.tab), self.query.clone());
        self.tab = next_tab;
        self.follow = true;
        self.scroll = *self
            .scroll_memory
            .get(&(self.key.clone(), self.tab))
            .unwrap_or(&0);
        self.query = self
            .query_memory
            .get(&(self.key.clone(), self.tab))
            .cloned()
            .unwrap_or_default();
        self.searching = false;
    }

    pub fn switch_key(&mut self, new_key: String) {
        self.scroll_memory
            .insert((self.key.clone(), self.tab), self.scroll);
        self.query_memory
            .insert((self.key.clone(), self.tab), self.query.clone());
        self.key = new_key;
        // A different element is a different task: its old record is gone.
        self.task = None;
        self.fetched_round = None;
        self.scroll = *self
            .scroll_memory
            .get(&(self.key.clone(), self.tab))
            .unwrap_or(&0);
        self.query = self
            .query_memory
            .get(&(self.key.clone(), self.tab))
            .cloned()
            .unwrap_or_default();
        self.searching = false;
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
        self.query = self
            .query_memory
            .get(&(self.key.clone(), self.tab))
            .cloned()
            .unwrap_or_default();
    }

    /// The id of the selected comment, on the Comments tab.
    pub fn selected_comment(&self, view: &View) -> Option<u64> {
        (self.tab == COMMENTS)
            .then(|| view.comments_on(&self.key).get(self.comment).map(|c| c.id))
            .flatten()
    }

    /// The task id of a worker or verifier.
    pub fn task_id(&self) -> Option<&str> {
        self.key.split_once(':').map(|(_, id)| id)
    }

    pub fn lines(&mut self, view: &View) -> Vec<(Style, String)> {
        let records = self.records(view);
        let mut out = vec![];
        let query_lower = self.query.trim().to_lowercase();
        if !self.query.is_empty() {
            out.push((
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
                format!(
                    "🔍 Record filter: \"{}\"  (v: toggle raw/readable view · Esc: clear)",
                    self.query
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
                if self.raw {
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
                    out.extend(rec.lines);
                }
            }
        }
        if !query_lower.is_empty() && matched_records == 0 {
            out.push((
                dim(),
                format!("No records matching query \"{}\".", self.query),
            ));
        }
        out
    }

    fn records(&mut self, view: &View) -> Vec<DetailRecord> {
        let plain = |text: String| (Style::default(), text);
        let label = |text: &str| {
            (
                Style::default().add_modifier(Modifier::BOLD),
                text.to_owned(),
            )
        };
        let is_conv = self.key == "conversation";
        let agent = view.agents.get(&self.key);
        let task = self
            .task_id()
            .and_then(|id| view.tasks.iter().find(|task| task.id == id));
        let verifier = self.key.starts_with("verifier:");
        let mut records = vec![];

        match self.tab {
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
                    records.push(DetailRecord {
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
                    });
                } else {
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
                        records.push(DetailRecord {
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
                        });
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
                            "Integration: {} · {}",
                            taskinfo::integration_label(task),
                            taskinfo::integration_detail(task)
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
                        if verifier && !task.risks.is_empty() {
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
                        records.push(DetailRecord {
                            lines: task_lines,
                            search_text: text,
                            raw_value: self.task.clone(),
                        });
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
                        records.push(DetailRecord {
                            lines: act_lines,
                            search_text: text,
                            raw_value: Some(serde_json::json!({
                                "running": agent.running,
                                "activity": agent.activity,
                            })),
                        });
                    } else if task.is_none() {
                        records.push(DetailRecord {
                            lines: vec![plain("This agent has not worked yet.".into())],
                            search_text: "This agent has not worked yet.".into(),
                            raw_value: None,
                        });
                    }
                }
            }
            1 => {
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
                    records.push(DetailRecord {
                        lines: vec![(
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                            header.clone(),
                        )],
                        search_text: format!("session {header}"),
                        raw_value: Some(serde_json::json!({
                            "agent": self.key,
                            "status": state,
                            "pid": agent.pid,
                        })),
                    });
                    if agent.feed_dropped > 0 {
                        records.push(DetailRecord {
                            lines: vec![(
                                dim(),
                                format!("… {} earlier entries are not shown", agent.feed_dropped),
                            )],
                            search_text: "earlier entries".into(),
                            raw_value: None,
                        });
                    }
                    if agent.feed.is_empty() {
                        // Older event logs only kept the short summary lines.
                        for line in &agent.session {
                            records.push(DetailRecord {
                                lines: vec![(Style::default(), line.clone())],
                                search_text: line.clone(),
                                raw_value: Some(serde_json::json!({ "session": line })),
                            });
                        }
                    }
                    for block in &agent.feed {
                        let lines = super::session::render(provider, std::slice::from_ref(block), self.expand_prompts);
                        let search_text = lines
                            .iter()
                            .map(|(_, line)| line.as_str())
                            .collect::<Vec<_>>()
                            .join(" ");
                        records.push(DetailRecord {
                            raw_value: Some(serde_json::json!({ "session": search_text })),
                            lines,
                            search_text,
                        });
                    }
                } else {
                    records.push(DetailRecord {
                        lines: vec![(
                            dim(),
                            "No session events for this agent. [historical data unavailable or agent has not worked yet]. Tool events will appear during work.".into(),
                        )],
                        search_text: "no session events historical data unavailable".into(),
                        raw_value: None,
                    });
                }
            }
            2 => {
                let mailbox_items = view.mailbox.get(&self.key);
                match mailbox_items {
                    Some(mail) if !mail.is_empty() => {
                        for item in mail {
                            records.push(DetailRecord {
                                lines: vec![plain(item.clone())],
                                search_text: item.clone(),
                                raw_value: Some(serde_json::json!({ "mail": item })),
                            });
                        }
                    }
                    _ => {
                        let id = self.task_id().unwrap_or(&self.key);
                        records.push(DetailRecord {
                            lines: vec![(
                                dim(),
                                format!("No messages. [no recorded messages in history for this agent]. Use '@{id} message' in chat to write to this agent."),
                            )],
                            search_text: "no messages".into(),
                            raw_value: None,
                        });
                    }
                }
            }
            REVIEW => match view.reviews.get(&self.key) {
                Some(review) if !review.is_empty() => {
                    records.push(DetailRecord {
                        lines: review.iter().cloned().map(plain).collect(),
                        search_text: review.join(" "),
                        raw_value: Some(serde_json::json!({ "review": review })),
                    });
                }
                _ => records.push(DetailRecord {
                    lines: vec![(
                        dim(),
                        "No review yet. The latest verdict appears here once this agent has reviewed something."
                            .into(),
                    )],
                    search_text: "no review".into(),
                    raw_value: None,
                }),
            },
            COMMENTS => {
                let comments = view.comments_on(&self.key);
                if comments.is_empty() {
                    records.push(DetailRecord {
                        lines: vec![(
                            dim(),
                            "No comments. [no comments in history for this element]. Press 'c' to add a comment to this element.".into(),
                        )],
                        search_text: "no comments".into(),
                        raw_value: None,
                    });
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
                        let header_str = format!(
                            "#{} {} · {state}{stale}",
                            comment.id,
                            super::view::target_name(&comment.target)
                        );
                        let text_str = format!("  {}", comment.text);
                        let search_text = format!("{header_str} {text_str}");
                        records.push(DetailRecord {
                            lines: vec![(style, header_str), (Style::default(), text_str)],
                            search_text,
                            raw_value: Some(serde_json::to_value(comment).unwrap_or_default()),
                        });
                    }
                }
            }
            CHECKS => match (&self.task, task) {
                (_, None) => records.push(DetailRecord {
                    lines: vec![(
                        dim(),
                        "Tasks have checks: task risks and its kind's checklist, accounted for by the verifier. [no check data in history]".into(),
                    )],
                    search_text: "tasks have checks".into(),
                    raw_value: None,
                }),
                (None, Some(_)) => records.push(DetailRecord {
                    lines: vec![(dim(), "Loading…".into())],
                    search_text: "loading".into(),
                raw_value: None,
                }),
                (Some(stored), Some(_)) => {
                    let check_lines = checks(stored);
                    for line in check_lines {
                        records.push(DetailRecord {
                            search_text: line.1.clone(),
                            lines: vec![line],
                            raw_value: Some(stored["verdict"]["coverage"].clone()),
                        });
                    }
                }
            },
            _ => match (&self.task, task) {
                (_, None) => records.push(DetailRecord {
                    lines: vec![(
                        dim(),
                        "Task workers and verifiers have results; plan verifier and auditor results are in Overview. [no historical result data]".into(),
                    )],
                    search_text: "workers and verifiers have results".into(),
                    raw_value: None,
                }),
                (None, Some(_)) => records.push(DetailRecord {
                    lines: vec![(dim(), "Loading…".into())],
                    search_text: "loading".into(),
                    raw_value: None,
                }),
                (Some(stored), Some(_)) if verifier => match &stored["verdict"] {
                    Value::Null => records.push(DetailRecord {
                        lines: vec![(
                            dim(),
                            "Verifier has not evaluated the task yet (verdict will appear after verification). [no historical verdict data]".into(),
                        )],
                        search_text: "verifier has not evaluated".into(),
                        raw_value: None,
                    }),
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
                        records.push(DetailRecord {
                            lines,
                            search_text: text,
                            raw_value: Some(verdict.clone()),
                        });
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
                    records.push(DetailRecord {
                        lines: art_lines,
                        search_text: art_text,
                        raw_value: Some(serde_json::json!({
                            "base": stored["base"],
                            "result": stored["result"],
                        })),
                    });

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
                    records.push(DetailRecord {
                        lines: rep_lines,
                        search_text: rep_text,
                        raw_value: Some(serde_json::json!({
                            "report": stored["report"],
                        })),
                    });

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
                    records.push(DetailRecord {
                        lines: ev_lines,
                        search_text: ev_text,
                        raw_value: Some(stored["verdict"].clone()),
                    });
                }
            },
        }
        records
    }

    pub fn draw(&mut self, frame: &mut Frame, view: &View) {
        if !tabs_for(&self.key).contains(&self.tab) {
            self.tab = 0;
        }
        let area = frame.area();
        if area.width == 0 || area.height == 0 {
            return;
        }
        let [header, body, footer] = Layout::vertical([
            Constraint::Length(1),
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
        frame.render_widget(Paragraph::new(tabs_line), header);

        let block = Block::default().borders(Borders::TOP);
        let inner = block.inner(body);
        frame.render_widget(block, body);
        let width = usize::from(inner.width.saturating_sub(1));
        let lines: Vec<Line> = self
            .lines(view)
            .into_iter()
            .flat_map(|(style, text)| {
                wrap(&text, width)
                    .into_iter()
                    .map(move |line| Line::styled(line, style))
            })
            .collect();
        let height = usize::from(inner.height);
        let bottom = lines.len().saturating_sub(height);
        if self.tab == 1 && self.follow {
            self.scroll = bottom;
        } else {
            self.scroll = self.scroll.min(bottom);
            // Scrolling back down to the end picks the tail up again.
            self.follow = self.scroll >= bottom;
        }
        let end = (self.scroll + height).min(lines.len());
        frame.render_widget(Paragraph::new(lines[self.scroll..end].to_vec()), inner);

        let hint = if self.searching {
            format!(
                "Search records: {}_ (Enter: confirm · Esc: cancel)",
                self.query
            )
        } else {
            details_footer_hint(self.tab, usize::from(footer.width)).to_string()
        };
        frame.render_widget(Paragraph::new(Span::styled(hint, dim())), footer);
    }
}

pub fn details_footer_hint(tab: usize, width: usize) -> &'static str {
    if tab == COMMENTS {
        if width >= 95 {
            "←/→ tab · ↑/↓ select · / search · v raw · c comment · r resolve · o reopen · x delete · d diff · ? help · Esc back"
        } else if width >= 65 {
            "←/→ tabs · ↑/↓ select · / search · v raw · c add · r/o/x status · d diff · Esc back"
        } else if width >= 45 {
            "←/→ tab · ↑/↓ select · c add · r/o/x · Esc"
        } else {
            "c com · Esc back"
        }
    } else {
        if width >= 90 {
            "←/→ tab · ↑/↓ PgUp/PgDn scroll · / search · v raw · c comment · d diff · ? help · Esc back to graph"
        } else if width >= 65 {
            "←/→ tabs · ↑/↓ scroll · / search · v raw · c comment · d diff · Esc back"
        } else if width >= 45 {
            "←/→ tab · ↑/↓ scroll · c com · d diff · Esc"
        } else {
            "↑/↓ · Esc back"
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

/// The verifier's obligations (the task's risks, then its kind's checklist)
/// and how the latest verdict accounted for each.
fn checks(stored: &Value) -> Vec<(Style, String)> {
    let Ok(spec) = serde_json::from_value::<TaskSpec>(stored["spec"].clone()) else {
        return vec![(dim(), "Cannot read task.".into())];
    };
    let coverage = &stored["verdict"]["coverage"];
    let mut lines = vec![];
    if stored["verdict"].is_null() {
        lines.push((
            dim(),
            "Verifier has not evaluated yet. Risks and checklist will be accounted for after the task finishes [no historical data].".into(),
        ));
        lines.push((Style::default(), String::new()));
    }
    for (id, text) in obligations(&spec) {
        let entry = &coverage[id.as_str()];
        let (mark, style) = match entry["status"]
            .as_str()
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("ok") => ("✓", Style::default().fg(Color::Green)),
            Some("issue") => ("✗", Style::default().fg(Color::Red)),
            Some("blocked") => ("!", Style::default().fg(Color::Yellow)),
            Some("n/a") => ("–", dim()),
            _ if stored["verdict"].is_null() => ("○", dim()),
            _ => ("?", Style::default().fg(Color::Yellow)),
        };
        let kind = if id.starts_with('R') {
            "risk"
        } else if id.starts_with('V') {
            "common"
        } else {
            "check"
        };
        lines.push((style, format!("{mark} {id} {kind}: {text}")));
        if let Some(note) = entry["note"].as_str() {
            lines.push((Style::default(), format!("    {note}")));
        } else if !stored["verdict"].is_null() {
            lines.push((dim(), "    unaccounted for [no evidence in history]".into()));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let line = format_tabs("worker:t1", 3, 50);
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

        // Tab 1: Session empty state
        details.tab = 1;
        let lines = details.lines(&view);
        assert!(
            lines
                .iter()
                .any(|(_, text)| text.contains("No session events"))
        );

        // Tab 2: Messages empty state
        details.tab = 2;
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
            ["Overview", "Session", "Messages", "Result", "Comments"]
        );
        assert_eq!(
            names("verifier:t1"),
            ["Overview", "Session", "Messages", "Checks", "Comments"]
        );
        assert_eq!(
            names("auditor"),
            ["Overview", "Session", "Review", "Comments"]
        );
        assert_eq!(
            names("plan-verifier"),
            ["Overview", "Session", "Review", "Comments"]
        );
        assert_eq!(names("conversation"), ["Overview", "Messages", "Comments"]);

        // Navigation skips what is not there and wraps around.
        let mut details = Details::new("worker:t1".into());
        details.switch_tab(details.neighbour_tab(1));
        assert_eq!(details.tab, 1);
        details.switch_tab(RESULT);
        details.switch_tab(details.neighbour_tab(1));
        assert_eq!(details.tab, COMMENTS);
        details.switch_tab(details.neighbour_tab(1));
        assert_eq!(details.tab, 0);
        details.switch_tab(details.neighbour_tab(-1));
        assert_eq!(details.tab, COMMENTS);
        // Number keys count the visible tabs: 4 is Result for a worker.
        assert_eq!(details.nth_tab(3), Some(RESULT));
        assert_eq!(details.nth_tab(5), None);
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

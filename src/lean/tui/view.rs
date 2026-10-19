//! What the screen shows, built only from the conversation's events, so a
//! frontend that attaches later rebuilds the same state.

use super::super::tasks::status;
use super::session::Block;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::time::Instant;

pub const MIN_TERM_WIDTH: u16 = 12;
pub const MIN_TERM_HEIGHT: u16 = 5;
pub const SIDE_BY_SIDE_BREAKPOINT: u16 = 85;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelFocus {
    Chat,
    Tasks,
    Graph,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalKind {
    None,
    Help(super::palette::Help),
    Picker(super::palette::Kind),
    CommentEditor,
    Diff,
    Details,
}

pub const ORCHESTRATOR: &str = "orchestrator";
pub const PLAN_VERIFIER: &str = "plan-verifier";
pub const AUDITOR: &str = "auditor";
/// Lines of session activity kept per agent.
const SESSION_LINES: usize = 1500;
/// Turns of one agent kept in its session view.
const FEED_TURNS: usize = 12;
/// Lines kept in a progress card before the oldest go.
const PROGRESS_LINES: usize = 8;

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub style: Style,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MessageEntry {
    pub id: String,
    pub sender: String,
    pub target: String,
    pub text: String,
    pub state: String,
    pub kind: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationAction {
    AcceptPlan,
    ResumePlan,
    ReviewDiff,
    CancelWork,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationStatus {
    Pending,
    Confirmed,
    Stale,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ConfirmationCard {
    pub id: String,
    pub title: String,
    pub action: ConfirmationAction,
    pub plan_id: Option<String>,
    pub task_id: Option<String>,
    pub candidate: Option<String>,
    pub scope_hash: String,
    pub status: ConfirmationStatus,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TranscriptCard {
    Message(MessageEntry),
    Plan {
        plan: String,
        tasks: usize,
        text: String,
    },
    System {
        title: String,
        meta: String,
        text: String,
        tone: super::theme::Tone,
    },
    Comment {
        id: u64,
        target: String,
        text: String,
        status: String,
    },
    Confirmation(ConfirmationCard),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TaskRow {
    pub id: String,
    /// The task's database id, once it is queued.
    pub uuid: Option<String>,
    pub title: String,
    pub summary: Option<String>,
    pub verifier_summary: Option<String>,
    /// Optional section heading in the plan document this task is assigned to.
    pub section: Option<String>,
    pub kind: String,
    pub goal: String,
    pub scope: Vec<String>,
    pub depends_on: Vec<String>,
    /// Peer task ids that can execute in parallel with this task.
    pub parallel_with: Vec<String>,
    pub writes: bool,
    pub focus: String,
    pub risks: Vec<String>,
    pub status: String,
    pub round: u64,
    /// What its worker or verifier is doing right now.
    pub activity: String,
    /// Why it stopped, when it stopped short of approval.
    pub reason: Option<String>,
    pub scope_hash: Option<String>,
}

/// One agent on the team graph: the orchestrator, the plan verifier, the
/// auditor, and each task's worker and verifier. Keys are `orchestrator`,
/// `plan-verifier`, `auditor`, `worker:t1`, `verifier:t1`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Agent {
    pub provider: String,
    /// The model and native effort of its latest turn, when configured.
    pub model: Option<String>,
    pub effort: Option<String>,
    pub running: bool,
    /// Process ID of the running or latest turn, if reported.
    pub pid: Option<u32>,
    /// The current tool call while running, else the last outcome.
    pub activity: String,
    pub session: Vec<String>,
    /// What the agent's CLI did and printed, drawn like that CLI does.
    pub feed: Vec<Block>,
    /// Entries dropped from the start of `feed` to keep it bounded.
    pub feed_dropped: usize,
    /// Finished turns, and how many of them the CLI reported tokens for.
    pub turns: u32,
    pub reported: u32,
    /// Time spent in finished turns whose start and end times are known,
    /// and the last such turn's.
    pub busy_ms: u64,
    pub last_ms: Option<u64>,
    /// When the running turn started, if known.
    pub started_ms: Option<i64>,
    /// Sums of what was reported; `None` while nothing was.
    pub tokens_in: Option<u64>,
    pub tokens_out: Option<u64>,
}

impl Agent {
    /// Token use as the CLI reported it; a turn it reported nothing for is
    /// unknown, never zero.
    pub fn tokens(&self) -> String {
        if self.reported == 0 {
            return "unavailable (CLI did not report them)".into();
        }
        let whole = if self.reported >= self.turns {
            String::new()
        } else {
            format!(
                " · reported in {} of {} turns, rest unknown",
                self.reported, self.turns
            )
        };
        let side = |sum: Option<u64>| sum.map_or("unknown".to_owned(), |sum| sum.to_string());
        format!(
            "in {} / out {}{whole}",
            side(self.tokens_in),
            side(self.tokens_out)
        )
    }
}

/// Messages that passed between two agents.
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub count: u32,
    pub last: Instant,
}

/// A comment as the daemon stores it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Comment {
    pub id: u64,
    pub plan_id: Option<String>,
    pub target: String,
    pub text: String,
    pub status: String,
}

#[derive(Debug, Clone)]
pub struct CommentProgress {
    pub state: String,
    pub error: Option<String>,
}

#[derive(Debug, Default)]
pub struct View {
    pub transcript: Vec<Entry>,
    pub tasks: Vec<TaskRow>,
    /// The plan the task rows show.
    pub plan: Option<String>,
    /// Full Markdown document of the latest proposed plan.
    pub plan_doc: String,
    /// The orchestrator's current activity, while it works.
    pub activity: Option<String>,
    pub idle: bool,
    pub after: u64,
    pub agents: HashMap<String, Agent>,
    /// Messages between agents by (sender key, recipient key).
    pub links: HashMap<(String, String), Link>,
    /// Every message to or from an agent, by its key, for its details.
    pub mailbox: HashMap<String, Vec<String>>,
    /// The user's comments by id, deleted ones included.
    pub comments: BTreeMap<u64, Comment>,
    pub comment_progress: BTreeMap<u64, CommentProgress>,
    /// The latest review of the plan verifier and the auditor, as lines.
    pub reviews: HashMap<String, Vec<String>>,
    /// The user paused new plan work.
    pub paused: bool,
    /// The conversation's mode, once the daemon said it.
    pub autonomous: Option<bool>,
    /// Structured messages with observed actual states.
    pub messages: Vec<MessageEntry>,
    /// Structured cards for rendering the conversation.
    pub cards: Vec<TranscriptCard>,
    pub plan_scope_hash: Option<String>,
    pub conversation_scope_hash: Option<String>,
    /// Agent key of each running turn.
    roles: HashMap<String, String>,
    comment_turns: HashMap<String, Vec<u64>>,
    /// Recipient of each shown message to an agent, by mail id.
    mail: HashMap<u64, String>,
    /// Task row index by the task's database id.
    task_rows: HashMap<String, usize>,
}

/// The conversation's step, shown in the header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Fresh,
    Interview,
    Planning,
    Review,
    Executing,
    Blocked,
    Done,
}

impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Fresh => "new",
            Self::Interview => "interview",
            Self::Planning => "planning",
            Self::Review => "plan review",
            Self::Executing => "executing",
            Self::Blocked => "blocked",
            Self::Done => "done",
        }
    }
}

pub fn dim() -> Style {
    Style::default().fg(Color::DarkGray)
}

impl View {
    pub fn comment_state(&self, id: u64) -> &str {
        match self.comments.get(&id) {
            Some(comment) if comment.status != "open" => &comment.status,
            _ => self
                .comment_progress
                .get(&id)
                .map_or("queued", |progress| progress.state.as_str()),
        }
    }

    pub fn comment_error(&self, id: u64) -> Option<&str> {
        self.comment_progress.get(&id)?.error.as_deref()
    }

    /// Keep the comment identity visible even when tool activity changes.
    pub fn comment_activity(&self) -> Option<String> {
        let active: Vec<_> = self
            .comment_progress
            .iter()
            .filter(|(_, progress)| progress.state == "addressing")
            .filter_map(|(id, _)| self.comments.get(id))
            .collect();
        let first = active.first()?;
        let ids = active
            .iter()
            .map(|comment| format!("#{}", comment.id))
            .collect::<Vec<_>>()
            .join(", ");
        Some(format!(
            "Addressing comment {ids} (comment agent): {}",
            clean(&first.text)
        ))
    }

    pub fn orchestrator_activity(&self) -> Option<String> {
        self.comment_activity().or_else(|| self.activity.clone())
    }

    fn push(&mut self, style: Style, text: impl Into<String>) {
        self.transcript.push(Entry {
            style,
            text: clean(&text.into()),
        });
    }

    pub fn plan_proposed(&self) -> bool {
        !self.tasks.is_empty() && self.tasks.iter().all(|task| task.status == "proposed")
    }

    /// Where the conversation is: the graph only makes sense once work runs.
    pub fn stage(&self) -> Stage {
        let started = self.activity.is_some()
            || !self.transcript.is_empty()
            || !self.messages.is_empty()
            || !self.cards.is_empty();
        if self.plan_proposed() {
            return Stage::Review;
        }
        let running = |status: &str| matches!(status, "queued" | "working" | "verifying");
        if self.tasks.iter().any(|task| running(&task.status)) {
            return Stage::Executing;
        }
        if self.activity.is_some() && !self.tasks.iter().all(|t| t.status == "approved") {
            // The orchestrator is on its own again: asking, or revising a plan.
            return if self.tasks.is_empty() {
                Stage::Interview
            } else {
                Stage::Planning
            };
        }
        if self.tasks.is_empty() {
            return if started {
                Stage::Interview
            } else {
                Stage::Fresh
            };
        }
        if self.tasks.iter().all(|task| task.status == "approved") {
            Stage::Done
        } else {
            Stage::Blocked
        }
    }

    /// The agent graph is hidden while the user is interviewed and while a
    /// plan is written or reviewed, whichever panel has the focus.
    pub fn graph_hidden(&self) -> bool {
        matches!(
            self.stage(),
            Stage::Interview | Stage::Planning | Stage::Review
        )
    }

    /// Unresolved comments targeting the current plan document or whole plan.
    pub fn open_plan_comments(&self) -> Vec<&Comment> {
        let Some(plan_id) = self.plan.as_deref() else {
            return Vec::new();
        };
        let prefix = format!("plan:{plan_id}:");
        let exact = format!("plan:{plan_id}");
        self.comments
            .values()
            .filter(|c| c.status == "open")
            .filter(|c| {
                (c.plan_id.is_none() || c.plan_id.as_deref() == Some(plan_id))
                    && (c.target == "plan"
                        || c.target == exact
                        || c.target.starts_with(&prefix)
                        || c.target.starts_with("plan:line:"))
            })
            .collect()
    }

    /// Whether the plan ended with tasks that can run again.
    pub fn plan_resumable(&self) -> bool {
        self.tasks.iter().all(|task| status::finished(&task.status))
            && self.tasks.iter().any(|task| {
                matches!(
                    task.status.as_str(),
                    "cancelled" | "failed" | "needs_user" | "skipped" | "waiting"
                )
            })
    }

    /// The agent key of a session role (`worker:<uuid>` → `worker:t1`).
    fn agent_key(&self, role: &str) -> String {
        if role == super::super::COMMENT_ORCHESTRATOR {
            return ORCHESTRATOR.into();
        }
        match role.split_once(':') {
            Some((who, uuid)) => match self.task_rows.get(uuid) {
                Some(&row) => format!("{who}:{}", self.tasks[row].id),
                None => role.to_owned(),
            },
            None => role.to_owned(),
        }
    }

    /// The agent key of a message's sender or recipient display name.
    fn mail_key(name: &str) -> String {
        match name {
            ORCHESTRATOR | PLAN_VERIFIER | AUDITOR | "user" | "runtime" => name.to_owned(),
            super::super::COMMENT_ORCHESTRATOR => ORCHESTRATOR.to_owned(),
            name => match name.split_once('/') {
                Some((task, _)) => format!("verifier:{task}"),
                None => format!("worker:{name}"),
            },
        }
    }

    fn session(&mut self, key: &str, line: String) {
        let session = &mut self.agents.entry(key.to_owned()).or_default().session;
        session.push(clean(&line));
        if session.len() > SESSION_LINES {
            session.drain(..session.len() - SESSION_LINES);
        }
    }

    fn feed(&mut self, key: &str, block: Block) {
        let agent = self.agents.entry(key.to_owned()).or_default();
        agent.feed.push(block);
        // Keep the latest turns whole: drop from the oldest turn's start.
        let turns = |feed: &[Block]| feed.iter().filter(|b| matches!(b, Block::Turn(_))).count();
        while agent.feed.len() > SESSION_LINES || turns(&agent.feed) > FEED_TURNS {
            let end = agent
                .feed
                .iter()
                .skip(1)
                .position(|b| matches!(b, Block::Turn(_)))
                .map_or(agent.feed.len().saturating_sub(1), |at| at + 1);
            if end == 0 {
                break;
            }
            agent.feed.drain(..end);
            agent.feed_dropped += end;
        }
    }

    /// A short line for what just happened to a task, kept in one card.
    fn report_progress(
        &mut self,
        id: &str,
        title: &str,
        was: (&str, u64),
        now: (&str, u64),
        reason: Option<String>,
    ) {
        use super::theme::Tone;
        let why = reason
            .map(|reason| format!(" — {}", clean(&reason).replace('\n', " ")))
            .unwrap_or_default();
        let name = if title.is_empty() {
            id.to_owned()
        } else {
            format!("{id} {title}")
        };
        let done = self.tasks.iter().filter(|t| t.status == "approved").count();
        let (line, tone) = match now.0 {
            _ if was.0 == now.0 && was.1 == now.1 => return,
            "approved" => (
                format!("✓ {name} approved ({done}/{})", self.tasks.len()),
                Tone::Done,
            ),
            "failed" => (format!("✗ {name} failed{why}"), Tone::Failed),
            "needs_user" => (format!("! {name} needs your decision{why}"), Tone::Waiting),
            "waiting" => (
                format!("‖ {name} waits for login or limit{why}"),
                Tone::Waiting,
            ),
            "skipped" => (format!("– {name} skipped{why}"), Tone::Queued),
            "working" if now.1 > was.1 && was.1 > 0 => (
                format!(
                    "↺ {name}: round {} after the verifier asked for changes",
                    now.1
                ),
                Tone::Active,
            ),
            _ => return,
        };
        let merged = match self.cards.last_mut() {
            Some(TranscriptCard::System {
                title,
                text,
                tone: card_tone,
                ..
            }) if title == "Progress" => {
                text.push('\n');
                text.push_str(&line);
                let skip = text.lines().count().saturating_sub(PROGRESS_LINES);
                *text = text.lines().skip(skip).collect::<Vec<_>>().join("\n");
                if tone != Tone::Done {
                    *card_tone = tone;
                }
                true
            }
            _ => false,
        };
        if !merged {
            self.cards.push(TranscriptCard::System {
                title: "Progress".into(),
                meta: String::new(),
                text: line,
                tone,
            });
        }
    }

    fn row_of(&mut self, key: &str) -> Option<&mut TaskRow> {
        let id = key.split_once(':')?.1;
        self.tasks.iter_mut().find(|task| task.id == id)
    }

    pub fn apply(&mut self, event: &Value) {
        self.after = event["seq"].as_u64().unwrap_or(self.after);
        let payload = &event["payload"];
        let text = |key: &str| payload[key].as_str().unwrap_or_default().to_owned();
        let turn = event["turn_id"].as_str().unwrap_or_default();
        let key = self.roles.get(turn).cloned().unwrap_or_default();
        match event["kind"].as_str().unwrap_or_default() {
            "message" => match payload["author"].as_str().unwrap_or_default() {
                // Shown by their `comment` and `code_review` events.
                "user" if text("text").starts_with(super::super::COMMENT_PREFIX) => {}
                "code-review" => {}
                "user" => {
                    let msg = MessageEntry {
                        id: format!("m{}", self.after),
                        sender: "You".to_owned(),
                        target: "orchestrator".to_owned(),
                        text: text("text"),
                        state: "sent".to_owned(),
                        kind: "user".to_owned(),
                    };
                    self.messages.push(msg.clone());
                    self.cards.push(TranscriptCard::Message(msg));
                    self.feed(ORCHESTRATOR, Block::User(text("text")));
                    self.push(
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                        "You",
                    );
                    self.push(Style::default(), text("text"));
                    self.push(Style::default(), "");
                }
                "runtime" => {
                    self.cards.push(TranscriptCard::System {
                        title: "Runtime".into(),
                        meta: String::new(),
                        text: "· task results handed to orchestrator".into(),
                        tone: crate::lean::tui::Tone::Queued,
                    });
                    self.push(dim(), "· task results handed to orchestrator");
                    self.push(Style::default(), "");
                }
                author => {
                    let display_author = match author {
                        ORCHESTRATOR => "orchestrator".to_owned(),
                        PLAN_VERIFIER => "plan verifier".to_owned(),
                        author => author.to_owned(),
                    };
                    let msg = MessageEntry {
                        id: format!("m{}", self.after),
                        sender: display_author.clone(),
                        target: "You".to_owned(),
                        text: text("text"),
                        state: "delivered".to_owned(),
                        kind: "agent".to_owned(),
                    };
                    self.messages.push(msg.clone());
                    self.cards.push(TranscriptCard::Message(msg));
                    self.push(
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                        display_author,
                    );
                    self.push(Style::default(), text("text"));
                    self.push(Style::default(), "");
                }
            },
            "turn_started" => {
                let key = self.agent_key(&text("role"));
                let provider = text("provider");
                if key == ORCHESTRATOR {
                    let comments: Vec<_> = payload["comments"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_u64)
                        .collect();
                    for id in &comments {
                        self.comment_progress.insert(
                            *id,
                            CommentProgress {
                                state: "addressing".into(),
                                error: None,
                            },
                        );
                    }
                    self.comment_turns.insert(turn.to_owned(), comments);
                    self.activity = Some(format!("orchestrator ({provider}) is working…"));
                } else if let Some(row) = self.row_of(&key) {
                    let who = key.split(':').next().unwrap_or_default();
                    row.activity = format!("{who} ({provider})");
                }
                let model = payload["model"].as_str().map(str::to_owned);
                let effort = payload["effort"].as_str().map(str::to_owned);
                let agent = self.agents.entry(key.clone()).or_default();
                agent.provider = provider.clone();
                agent.model = model.clone();
                agent.effort = effort.clone();
                agent.running = true;
                agent.started_ms = event["created_ms"].as_i64().filter(|ms| *ms > 0);
                agent.activity = "starting turn".into();
                self.session(&key, "── turn".into());
                self.feed(
                    &key,
                    Block::Turn(format!(
                        "{provider} · {} · effort {}",
                        model.as_deref().unwrap_or("default model"),
                        effort.as_deref().unwrap_or("default")
                    )),
                );
                self.roles.insert(turn.to_owned(), key);
            }
            "agent" if payload["type"] == "process" => {
                if !key.is_empty()
                    && let Some(pid) = payload["pid"].as_u64()
                {
                    self.agents.entry(key.clone()).or_default().pid = Some(pid as u32);
                    self.feed(&key, Block::Note(format!("process {pid} started")));
                }
            }
            "agent" if payload["type"] == "session" => {
                if !key.is_empty() {
                    let sid = text("id");
                    self.feed(&key, Block::Note(format!("session {sid}")));
                }
            }
            "agent" if payload["type"] == "text" => {
                if !key.is_empty() && !text("delta").trim().is_empty() {
                    self.feed(&key, Block::Text(text("delta")));
                }
            }
            "agent" if payload["type"] == "reasoning" => {
                if !key.is_empty() && !text("delta").trim().is_empty() {
                    self.feed(&key, Block::Reasoning(text("delta")));
                }
            }
            "agent" if payload["type"] == "prompt" => {
                if !key.is_empty() {
                    self.feed(&key, Block::Prompt(text("text")));
                }
            }
            "agent" if payload["type"] == "output" => {
                if !key.is_empty() {
                    self.feed(
                        &key,
                        Block::Output {
                            name: text("name"),
                            text: text("text"),
                            exit_code: payload["exit_code"].as_i64(),
                        },
                    );
                }
            }
            "agent" if payload["type"] == "tool" => {
                let detail = format!("{} {}", text("name"), text("detail"));
                if key == ORCHESTRATOR {
                    self.activity = Some(format!("orchestrator: {detail}"));
                    self.push(dim(), format!("  ↳ {detail}"));
                } else if let Some(row) = self.row_of(&key) {
                    let who = key.split(':').next().unwrap_or_default().to_owned();
                    row.activity = format!("{who}: {detail}");
                }
                if !key.is_empty() {
                    self.agents.entry(key.clone()).or_default().activity = detail.clone();
                    self.session(&key, format!("↳ {detail}"));
                    self.feed(
                        &key,
                        Block::Tool {
                            name: text("name"),
                            detail: text("detail"),
                        },
                    );
                }
            }
            "agent" if payload["type"] == "usage" => {
                let (input, output) = (payload["input"].as_u64(), payload["output"].as_u64());
                if !key.is_empty() && (input.is_some() || output.is_some()) {
                    let agent = self.agents.entry(key.clone()).or_default();
                    agent.reported += 1;
                    let add = |sum: &mut Option<u64>, value: Option<u64>| {
                        if let Some(value) = value {
                            *sum = Some(sum.unwrap_or(0) + value);
                        }
                    };
                    add(&mut agent.tokens_in, input);
                    add(&mut agent.tokens_out, output);
                    self.feed(&key, Block::Tokens { input, output });
                }
            }
            "turn_finished" => {
                let failed = payload["status"] != "done";
                if key == ORCHESTRATOR {
                    self.activity = None;
                    if let Some(comments) = self.comment_turns.remove(turn) {
                        for id in comments {
                            // Reopening while this turn ran queued a newer
                            // request; the old completion must not consume it.
                            if self
                                .comment_progress
                                .get(&id)
                                .is_some_and(|progress| progress.state == "queued")
                            {
                                continue;
                            }
                            self.comment_progress.insert(
                                id,
                                CommentProgress {
                                    state: if failed {
                                        text("status")
                                    } else {
                                        "addressed".into()
                                    },
                                    error: payload["error"].as_str().map(str::to_owned),
                                },
                            );
                        }
                    }
                    if failed {
                        self.push(
                            Style::default().fg(Color::Red),
                            format!("✗ orchestrator: {} — {}", text("status"), text("error")),
                        );
                    }
                } else if let Some(row) = self.row_of(&key) {
                    row.activity = if failed {
                        format!("✗ {}", text("error"))
                    } else {
                        String::new()
                    };
                }
                if !key.is_empty() {
                    let outcome = if failed {
                        format!("✗ {}: {}", text("status"), text("error"))
                    } else {
                        "✓ turn finished".into()
                    };
                    let agent = self.agents.entry(key.clone()).or_default();
                    agent.running = false;
                    agent.turns += 1;
                    let ended = event["created_ms"].as_i64().filter(|ms| *ms > 0);
                    if let (Some(start), Some(end)) = (agent.started_ms.take(), ended) {
                        let took = end.saturating_sub(start).max(0) as u64;
                        agent.busy_ms += took;
                        agent.last_ms = Some(took);
                    }
                    agent.activity = outcome.clone();
                    self.session(&key, outcome.clone());
                    self.feed(
                        &key,
                        Block::End {
                            ok: !failed,
                            text: {
                                let mut text =
                                    outcome.trim_start_matches(['✓', '✗', ' ']).to_owned();
                                let agent = &self.agents[&key];
                                if let Some(last) = agent.last_ms.filter(|_| !failed) {
                                    text.push_str(&format!(" · {}", duration(last)));
                                }
                                text
                            },
                        },
                    );
                }
            }
            "plan_proposed" => {
                self.plan = payload["plan"].as_str().map(str::to_owned);
                self.plan_doc = payload["document"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_default();
                let strings = |value: &Value| -> Vec<String> {
                    value
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|item| item.as_str().map(str::to_owned))
                        .collect()
                };
                self.tasks = payload["tasks"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|task| {
                        let id = task["id"].as_str().unwrap_or_default().to_owned();
                        let parallel_with = strings(&payload["parallel"][&id]);
                        TaskRow {
                            id,
                            uuid: None,
                            title: task["title"].as_str().unwrap_or_default().to_owned(),
                            summary: task["summary"].as_str().map(str::to_owned),
                            verifier_summary: task["verifier"]["summary"]
                                .as_str()
                                .map(str::to_owned),
                            section: task["section"].as_str().map(str::to_owned),
                            kind: task["kind"].as_str().unwrap_or_default().to_owned(),
                            goal: task["goal"].as_str().unwrap_or_default().to_owned(),
                            scope: strings(&task["scope"]),
                            depends_on: strings(&task["depends_on"]),
                            parallel_with,
                            writes: task["writes"] == true,
                            focus: task["verifier"]["focus"]
                                .as_str()
                                .unwrap_or_default()
                                .to_owned(),
                            risks: strings(&task["verifier"]["risks"]),
                            status: "proposed".into(),
                            round: 0,
                            activity: if task["writes"] == true {
                                "modifies files".into()
                            } else {
                                String::new()
                            },
                            reason: None,
                            scope_hash: task["scope_hash"].as_str().map(str::to_owned),
                        }
                    })
                    .collect();
                // Compute parallel_with fallback when event did not include payload["parallel"]
                if payload["parallel"].is_null() && self.tasks.len() > 1 {
                    let ids_and_deps: Vec<(String, Vec<String>, bool, Vec<String>)> = self
                        .tasks
                        .iter()
                        .map(|t| {
                            (
                                t.id.clone(),
                                t.depends_on.clone(),
                                t.writes,
                                t.scope.clone(),
                            )
                        })
                        .collect();
                    for row in &mut self.tasks {
                        row.parallel_with = ids_and_deps
                            .iter()
                            .filter(|(oid, odeps, owrites, oscope)| {
                                *oid != row.id
                                    && !row.depends_on.contains(oid)
                                    && !odeps.contains(&row.id)
                                    && !(row.writes
                                        && *owrites
                                        && super::super::tasks::scopes_overlap(&row.scope, oscope))
                            })
                            .map(|(oid, _, _, _)| oid.clone())
                            .collect();
                    }
                }
                self.task_rows.clear();
                self.agents
                    .retain(|key, _| !key.starts_with("worker:") && !key.starts_with("verifier:"));
                self.links.clear();
                self.mailbox.clear();
                for line in super::super::cli::review_lines("plan verifier", &payload["review"]) {
                    self.push(Style::default().fg(Color::Yellow), line);
                }
                let review = &payload["review"];
                if !review.is_null() {
                    self.review(PLAN_VERIFIER, review);
                }
                self.plan_scope_hash = payload["scope_hash"].as_str().map(str::to_owned);
                if let Some(sh) = &self.plan_scope_hash {
                    self.conversation_scope_hash = Some(sh.clone());
                }
                let next = if payload["automatic"] == true {
                    "running (autonomous mode).".to_owned()
                } else {
                    let waiting = payload["waiting"]
                        .as_str()
                        .map(|reason| format!("Waiting for you: {reason}. "))
                        .unwrap_or_default();
                    format!("{waiting}Ctrl+A runs it, or type what to change.")
                };
                let plan_text = format!("Plan: {} tasks. {next}", self.tasks.len());
                self.cards.push(TranscriptCard::Plan {
                    plan: self.plan.clone().unwrap_or_default(),
                    tasks: self.tasks.len(),
                    text: plan_text.clone(),
                });
                self.push(Style::default().fg(Color::Yellow), plan_text);
                self.push(Style::default(), "");
            }
            "plan_rejected" => {
                for task in &mut self.tasks {
                    if task.status == "proposed" {
                        task.status = "cancelled".into();
                    }
                }
                let reason = text("reason");
                self.cards.push(TranscriptCard::System {
                    title: "Plan rejected".into(),
                    meta: String::new(),
                    text: format!("✗ {reason}"),
                    tone: super::theme::Tone::Failed,
                });
                self.push(
                    Style::default().fg(Color::Red),
                    format!("✗ plan rejected: {reason}"),
                );
            }
            "plan_review" => self.review(PLAN_VERIFIER, payload),
            "plan_invalid" => {
                let err = text("error");
                self.cards.push(TranscriptCard::System {
                    title: "Plan rejected".into(),
                    meta: String::new(),
                    text: format!("✗ {err}"),
                    tone: super::theme::Tone::Failed,
                });
                self.push(
                    Style::default().fg(Color::Red),
                    format!("✗ plan rejected: {err}"),
                );
            }
            "task" => {
                let id = text("id");
                let index = self.tasks.iter().position(|row| row.id == id);
                if let Some(index) = index {
                    self.task_rows.insert(text("task"), index);
                    let row = &mut self.tasks[index];
                    let (was, was_round) = (row.status.clone(), row.round);
                    row.uuid = Some(text("task"));
                    row.status = text("status");
                    row.round = payload["round"].as_u64().unwrap_or_default();
                    row.reason = payload["reason"].as_str().map(str::to_owned);
                    let (title, status, round, reason) = (
                        row.title.clone(),
                        row.status.clone(),
                        row.round,
                        row.reason.clone(),
                    );
                    let progress = (title, was, was_round, status, round);
                    if let Some(sh) = payload["scope_hash"].as_str() {
                        row.scope_hash = Some(sh.to_owned());
                        self.conversation_scope_hash = Some(sh.to_owned());
                    }
                    if status::finished(&row.status) {
                        row.activity.clear();
                    }
                    if let Some(reason) = row.reason.clone() {
                        let line =
                            format!("! {} {}: {} — {reason}", id, row.title, polish(&row.status));
                        self.push(Style::default().fg(Color::Yellow), line);
                    }
                    let (title, was, was_round, status, round) = progress;
                    self.report_progress(&id, &title, (&was, was_round), (&status, round), reason);
                }
            }
            "paused" => {
                self.paused = payload["paused"] == true;
                let txt = if self.paused {
                    "⏸ paused: new steps wait (/continue resumes)"
                } else {
                    "▶ resumed"
                };
                self.cards.push(TranscriptCard::System {
                    title: "Pause".into(),
                    meta: String::new(),
                    text: txt.into(),
                    tone: super::theme::Tone::Waiting,
                });
                self.push(Style::default().fg(Color::Yellow), txt);
            }
            "notice" => {
                let txt = text("text");
                self.cards.push(TranscriptCard::System {
                    title: "Notice".into(),
                    meta: String::new(),
                    text: txt.clone(),
                    tone: super::theme::Tone::Waiting,
                });
                self.push(Style::default().fg(Color::Yellow), format!("⚠ {txt}"));
            }
            "mode" => {
                self.autonomous = Some(payload["autonomous"] == true);
                self.push(
                    Style::default().fg(Color::Yellow),
                    if payload["autonomous"] == true {
                        "⚙ autonomous mode: next plans and rounds start automatically"
                    } else {
                        "⚙ supervised mode: next plans wait for approval (Ctrl+A)"
                    },
                );
            }
            "integration" => {
                let approved = payload["status"] == "approved";
                self.cards.push(TranscriptCard::System {
                    title: format!("[{}] {}", text("id"), text("title")),
                    meta: if approved {
                        "approved".into()
                    } else {
                        "verification".into()
                    },
                    text: text("detail"),
                    tone: if approved {
                        super::theme::Tone::Done
                    } else {
                        super::theme::Tone::Waiting
                    },
                });
                self.push(
                    if approved {
                        dim()
                    } else {
                        Style::default().fg(Color::Yellow)
                    },
                    format!("· [{}] {}: {}", text("id"), text("title"), text("detail")),
                );
            }
            "mail" if payload["kind"] != "copy" => {
                let recipient = text("recipient");
                let mail_id = payload["id"].as_u64().unwrap_or_default();
                self.mail.insert(mail_id, recipient.clone());
                let sender = match text("sender").as_str() {
                    "user" => "You".to_owned(),
                    ORCHESTRATOR => "orchestrator".to_owned(),
                    sender => sender.to_owned(),
                };
                let id_str = if mail_id > 0 {
                    format!("{mail_id}")
                } else {
                    format!("m{}", self.after)
                };
                let msg = MessageEntry {
                    id: id_str,
                    sender: sender.clone(),
                    target: recipient.clone(),
                    text: text("text"),
                    state: "waiting for turn".to_owned(),
                    kind: text("kind"),
                };
                self.messages.push(msg.clone());
                self.cards.push(TranscriptCard::Message(msg));
                let line = format!(
                    "{sender} → {recipient} ({}): {}",
                    text("kind"),
                    text("text")
                );
                self.push(
                    Style::default().fg(Color::Magenta),
                    format!("✉ {line} · waiting for recipient's next turn"),
                );
                let from = Self::mail_key(&text("sender"));
                let to = Self::mail_key(&recipient);
                for key in [&from, &to] {
                    self.mailbox
                        .entry(key.clone())
                        .or_default()
                        .push(clean(&line));
                    self.feed(key, Block::Mail(line.clone()));
                }
                let link = self.links.entry((from, to)).or_insert(Link {
                    count: 0,
                    last: Instant::now(),
                });
                link.count += 1;
                link.last = Instant::now();
            }
            "mail_delivered" => {
                for id in payload["ids"].as_array().into_iter().flatten() {
                    if let Some(mail_id) = id.as_u64() {
                        let id_str = format!("{mail_id}");
                        for msg in &mut self.messages {
                            if msg.id == id_str {
                                msg.state = "delivered".to_owned();
                            }
                        }
                        for card in &mut self.cards {
                            if let TranscriptCard::Message(msg) = card
                                && msg.id == id_str
                            {
                                msg.state = "delivered".to_owned();
                            }
                        }
                        if let Some(recipient) = self.mail.remove(&mail_id) {
                            self.push(dim(), format!("✓ {recipient} received message"));
                        }
                    }
                }
            }
            "mail_rejected" => {
                let sender = text("sender");
                let recipient = text("recipient");
                let norm_sender = match sender.as_str() {
                    "user" => "You",
                    ORCHESTRATOR => "orchestrator",
                    s => s,
                };
                for msg in self.messages.iter_mut().rev() {
                    if (msg.sender == sender || msg.sender == norm_sender)
                        && msg.target == recipient
                    {
                        msg.state = "rejected".to_owned();
                        break;
                    }
                }
                for card in self.cards.iter_mut().rev() {
                    if let TranscriptCard::Message(msg) = card
                        && (msg.sender == sender || msg.sender == norm_sender)
                        && msg.target == recipient
                    {
                        msg.state = "rejected".to_owned();
                        break;
                    }
                }
                self.push(
                    Style::default().fg(Color::Red),
                    format!(
                        "✗ message from {} to {} undeliverable: {}",
                        text("sender"),
                        text("recipient"),
                        text("reason")
                    ),
                );
            }
            "audit" => {
                for line in super::super::cli::review_lines("audit", payload) {
                    self.push(Style::default().fg(Color::Yellow), line);
                }
                self.review(AUDITOR, payload);
            }
            "code_review" => {
                let task = text("id");
                if payload["status"] == "running" {
                    self.push(dim(), format!("· code review {task} in progress"));
                } else {
                    let who = format!("code review {task}");
                    let lines = super::super::cli::review_lines(&who, payload);
                    for line in &lines {
                        self.push(Style::default().fg(Color::Yellow), line.clone());
                    }
                    self.reviews.insert(format!("worker:{task}"), lines);
                }
            }
            "comment" => {
                let Ok(comment) = serde_json::from_value::<Comment>(payload.clone()) else {
                    return;
                };
                let name = target_name(&comment.target);
                if comment.status == "open" {
                    self.comment_progress.insert(
                        comment.id,
                        CommentProgress {
                            state: "queued".into(),
                            error: None,
                        },
                    );
                }
                let (line, card_status) = match comment.status.as_str() {
                    "open" if !self.comments.contains_key(&comment.id) => (
                        format!("✎ #{} to {name}: {}", comment.id, comment.text),
                        "open".into(),
                    ),
                    "open" => (format!("✎ #{} reopened", comment.id), "reopened".into()),
                    "resolved" => (format!("✎ #{} resolved", comment.id), "resolved".into()),
                    _ => (format!("✎ #{} deleted", comment.id), "deleted".into()),
                };
                self.cards.push(TranscriptCard::Comment {
                    id: comment.id,
                    target: name,
                    text: comment.text.clone(),
                    status: card_status,
                });
                self.push(Style::default().fg(Color::Cyan), line);
                self.comments.insert(comment.id, comment);
            }
            _ => {}
        }
    }

    /// Comments on an element (a diff's comments count for its worker),
    /// newest first, deleted ones left out.
    pub fn comments_on(&self, key: &str) -> Vec<&Comment> {
        let diff = key
            .strip_prefix("worker:")
            .map(|task| format!("diff:{task}"));
        self.comments
            .values()
            .rev()
            .filter(|comment| comment.status != "deleted")
            .filter(|comment| {
                comment.target == key
                    || diff.as_ref().is_some_and(|diff| {
                        comment.target == *diff || comment.target.starts_with(&format!("{diff}:"))
                    })
            })
            .collect()
    }

    /// Whether a comment was made on an earlier plan than the one shown.
    pub fn stale(&self, comment: &Comment) -> bool {
        comment.plan_id.is_some() && comment.plan_id != self.plan
    }

    fn review(&mut self, key: &str, review: &Value) {
        self.agents.entry(key.into()).or_default().activity = review_word(review);
        self.reviews.insert(
            key.into(),
            super::super::cli::review_lines(&agent_name(key), review),
        );
    }

    /// What is happening now, one line per active step, for the progress
    /// overlay. Waiting and idle states are named, never shown as activity.
    pub fn progress(&self) -> Vec<String> {
        let mut lines = vec![];
        if self.paused {
            lines.push("⏸ paused: new steps wait".into());
        }
        if let Some(activity) = self.orchestrator_activity() {
            lines.push(clean(&activity));
        }
        for task in &self.tasks {
            match task.status.as_str() {
                "working" | "verifying" => {
                    let act = if task.activity.is_empty() {
                        "…".to_string()
                    } else {
                        clean(&task.activity)
                    };
                    lines.push(format!("{} {}: {}", task.id, polish(&task.status), act));
                }
                "waiting" => lines.push(format!("{} waiting for login/limit", task.id)),
                _ => {}
            }
        }
        for key in [PLAN_VERIFIER, AUDITOR] {
            if let Some(agent) = self.agents.get(key).filter(|agent| agent.running) {
                lines.push(format!("{}: {}", agent_name(key), clean(&agent.activity)));
            }
        }
        if lines.is_empty() {
            lines.push(if self.plan_proposed() {
                "waiting for you: plan to accept".into()
            } else if self.plan_resumable() {
                "waiting for you: plan to resume".into()
            } else {
                "idle".into()
            });
        }
        lines
    }
}

fn review_word(review: &Value) -> String {
    if review["error"].is_string() {
        "✗ failed".into()
    } else if review["approved"] == true {
        "✓ approved".into()
    } else {
        "⚠ issues".into()
    }
}

/// A human name for a comment's target.
pub fn target_name(target: &str) -> String {
    if let Some(diff) = super::super::diff::target(target) {
        return match (diff.file, diff.line) {
            (Some(file), Some(line)) => format!("{} diff {file}:{line}", diff.task),
            (Some(file), None) => format!("{} diff {file}", diff.task),
            _ => format!("{} diff", diff.task),
        };
    }
    if let Some((kind, rest)) = target.split_once(':') {
        match kind {
            "plan" => {
                if let Some((plan_id, line)) = rest.split_once(":line:") {
                    return format!("Plan {plan_id} line {line}");
                }
                if let Some(line) = rest.strip_prefix("line:") {
                    return format!("Plan line {line}");
                }
                return format!("Plan {rest}");
            }
            "task" => return format!("Task {rest}"),
            "check" => {
                if let Some((task_id, check_id)) = rest.split_once(':') {
                    return format!("Task {task_id} check {check_id}");
                }
                return format!("Check {rest}");
            }
            "finding" => {
                if let Some((task_id, finding_id)) = rest.split_once(':') {
                    return format!("Task {task_id} finding #{finding_id}");
                }
                return format!("Finding {rest}");
            }
            "report" => return format!("Task {rest} report"),
            "artifact" => {
                if let Some((task_id, path)) = rest.split_once(':') {
                    return format!("Task {task_id} artifact {path}");
                }
                return format!("Artifact {rest}");
            }
            _ => {}
        }
    }
    agent_name(target)
}

/// A human name for an agent key.
pub fn agent_name(key: &str) -> String {
    match key.split_once(':') {
        Some(("worker", id)) => format!("{id} worker"),
        Some(("verifier", id)) => format!("{id} verifier"),
        _ => match key {
            ORCHESTRATOR => "orchestrator".into(),
            PLAN_VERIFIER => "plan verifier".into(),
            AUDITOR => "auditor".into(),
            "conversation" | "rozmowa" => "conversation".into(),
            other => other.into(),
        },
    }
}

pub fn polish(status: &str) -> &str {
    match status {
        "proposed" => "proposed",
        "queued" => "queued",
        "working" => "working",
        "verifying" => "verifying",
        "approved" => "approved",
        "needs_user" => "needs decision",
        "failed" => "failed",
        "skipped" => "skipped",
        "cancelled" => "cancelled",
        "waiting" => "waiting for login/limit",
        "revised" => "sent back",
        other => other,
    }
}

/// A duration as people say it: `45 s`, `3 min 12 s`, `2 h 5 min`.
pub fn duration(ms: u64) -> String {
    let seconds = ms / 1000;
    match seconds {
        0..60 => format!("{seconds} s"),
        60..3600 => format!("{} min {} s", seconds / 60, seconds % 60),
        _ => format!("{} h {} min", seconds / 3600, seconds % 3600 / 60),
    }
}

/// Agents' text may carry terminal escapes; keep it printable.
pub fn clean(text: &str) -> String {
    super::sanitize::sanitize(text)
}

/// Word-wraps `text` to `width` columns, breaking long words safely by grapheme clusters.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    super::theme::wrap(text, width)
}

/// One line of at most `width` columns.
pub fn truncate(text: &str, width: usize) -> String {
    let t = super::theme::Theme::new(true, false);
    t.fit(text, width)
}

/// Renders the minimum terminal size warning guard when terminal is smaller than 12x5.
/// Returns true if the guard was triggered and drawn.
pub fn draw_min_term_guard(frame: &mut ratatui::Frame, area: Rect) -> bool {
    if area.width < MIN_TERM_WIDTH || area.height < MIN_TERM_HEIGHT {
        let msg = format!("Terminal too small (min. {MIN_TERM_WIDTH}x{MIN_TERM_HEIGHT} required)");
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(msg)
                .wrap(ratatui::widgets::Wrap { trim: false })
                .style(
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
            area,
        );
        true
    } else {
        false
    }
}

/// Builds the responsive header line according to terminal width, mode, status, and compact panel toggle badge.
pub fn render_header(
    project: &str,
    conversation: &str,
    mode: Option<bool>,
    status: &str,
    focus: PanelFocus,
    width: u16,
) -> Line<'static> {
    if width < 25 {
        return Line::from(vec![Span::styled(
            " antislop ",
            Style::default().add_modifier(Modifier::BOLD),
        )]);
    }

    let short = conversation.get(..8).unwrap_or(conversation);
    let mode_str = match mode {
        Some(true) => "[Mode: autonomous] · ",
        Some(false) => "[Mode: supervised] · ",
        None => "",
    };

    let mut spans = vec![Span::styled(
        " antislop ",
        Style::default().add_modifier(Modifier::BOLD),
    )];

    if width < SIDE_BY_SIDE_BREAKPOINT {
        let (chat_style, tasks_style, graph_style) = match focus {
            PanelFocus::Chat => (
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
                dim(),
                dim(),
            ),
            PanelFocus::Tasks => (
                dim(),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
                dim(),
            ),
            PanelFocus::Graph => (
                dim(),
                dim(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        };
        let (chat_label, tasks_label, graph_label) = match focus {
            PanelFocus::Chat => ("[Chat]", "Tasks", "Team"),
            PanelFocus::Tasks => ("Chat", "[Tasks]", "Team"),
            PanelFocus::Graph => ("Chat", "Tasks", "[Team]"),
        };
        spans.push(Span::styled(format!(" {chat_label} "), chat_style));
        spans.push(Span::styled(format!("{tasks_label} "), tasks_style));
        spans.push(Span::styled(format!("{graph_label} "), graph_style));
        let switch_hint = match focus {
            PanelFocus::Chat => "(Tab: switch) · (t: tasks) · ",
            PanelFocus::Tasks => "(Tab: switch) · (Esc: chat) · ",
            PanelFocus::Graph => "(Tab: switch) · (t: tasks) · ",
        };
        if width >= 75 {
            spans.push(Span::styled(switch_hint, dim()));
        } else if width >= 65 {
            spans.push(Span::styled("(Tab: switch) · ", dim()));
        } else if width >= 40 {
            spans.push(Span::styled("· ", dim()));
        }
    }

    if width >= SIDE_BY_SIDE_BREAKPOINT {
        spans.push(Span::styled(
            format!("{project} · {short} · {mode_str}"),
            dim(),
        ));
    } else if width >= 50 {
        spans.push(Span::styled(format!("{short} · {mode_str}"), dim()));
    }

    if (50..95).contains(&width) {
        // The wide header lists it with the other buttons.
        spans.push(Span::styled("[Plan] ", Style::default().fg(Color::Cyan)));
    }
    if width >= 40 {
        spans.push(Span::styled(
            clean(status),
            Style::default().fg(Color::Yellow),
        ));
    }
    if width >= 95 {
        spans.push(Span::styled(
            " · [/pause] [/continue] [/resume] [/cancel] [Plan]",
            Style::default().fg(Color::Cyan),
        ));
    }
    Line::from(spans)
}

/// Prioritizes action hints and notices by available footer width.
pub fn main_footer_hint(
    focus: PanelFocus,
    plan_proposed: bool,
    plan_resumable: bool,
    notice: Option<&str>,
    width: usize,
) -> String {
    if let Some(notice) = notice {
        return truncate(notice, width);
    }
    let plan = if plan_proposed {
        "Ctrl+A accept plan · "
    } else if plan_resumable {
        "Ctrl+R resume plan · "
    } else {
        ""
    };
    match focus {
        PanelFocus::Chat => {
            if width >= 115 {
                format!(
                    "Enter send (@t2 … to agent) · Alt+Enter newline · {plan}Ctrl+P commands · F1 help · Tab team · Ctrl+C/q quit"
                )
            } else if width >= 75 {
                format!("Enter send · Alt+Enter newline · {plan}Ctrl+P · F1 help · Tab team · Esc")
            } else if width >= 50 {
                format!("Enter send · {plan}Tab team · Esc")
            } else {
                "Enter · Esc".into()
            }
        }
        PanelFocus::Tasks => {
            if width >= 115 {
                format!(
                    "←/→ task · w worker · v verifier · Enter details · d diff · c comment · {plan}Tab team · Esc chat"
                )
            } else if width >= 75 {
                "←/→ task · w worker · v ver. · Enter details · d diff · Tab team · Esc".into()
            } else if width >= 50 {
                "←/→ task · w worker · v ver. · Tab team".into()
            } else {
                "←/→ task · w · v".into()
            }
        }
        PanelFocus::Graph => {
            if width >= 115 {
                format!(
                    "arrows select · PgUp/PgDn scroll text · Enter details · / find · d diff · c comment · {plan}? help · q quit"
                )
            } else if width >= 75 {
                "arrows select · PgUp/PgDn text · Enter details · / find · d diff · ? help · q"
                    .into()
            } else if width >= 50 {
                "arrows · Enter details · Tab chat · q".into()
            } else {
                "arrows · q".into()
            }
        }
    }
}

/// Empty state displayed when transcript contains no messages.
pub fn transcript_empty_state(width: usize) -> Vec<Line<'static>> {
    vec![
        Line::styled("No messages in this conversation.", dim()),
        Line::styled(
            truncate(
                "Type a message to the orchestrator below or press Ctrl+P to open the command palette.",
                width,
            ),
            dim(),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(seq: u64, turn: Option<&str>, kind: &str, payload: Value) -> Value {
        json!({"seq": seq, "turn_id": turn, "kind": kind, "payload": payload})
    }

    fn staged(statuses: &[&str], activity: bool) -> View {
        let mut view = View::default();
        view.cards.push(TranscriptCard::System {
            title: "t".into(),
            meta: String::new(),
            text: "x".into(),
            tone: crate::lean::tui::Tone::Queued,
        });
        view.tasks = statuses
            .iter()
            .enumerate()
            .map(|(i, status)| TaskRow {
                id: format!("t{i}"),
                status: (*status).into(),
                ..TaskRow::default()
            })
            .collect();
        view.activity = activity.then(|| "working".to_owned());
        view
    }

    #[test]
    fn the_session_keeps_the_latest_turns_whole_and_counts_what_it_dropped() {
        let mut view = View::default();
        for turn in 0..(FEED_TURNS + 3) {
            view.feed("worker:t1", Block::Turn(format!("turn {turn}")));
            view.feed("worker:t1", Block::Text(format!("answer {turn}")));
        }
        let agent = &view.agents["worker:t1"];
        assert_eq!(
            agent
                .feed
                .iter()
                .filter(|b| matches!(b, Block::Turn(_)))
                .count(),
            FEED_TURNS
        );
        // Whole turns go: the feed still starts with a turn header.
        assert_eq!(agent.feed[0], Block::Turn("turn 3".into()));
        assert_eq!(agent.feed_dropped, 6);
        assert_eq!(
            agent.feed.last(),
            Some(&Block::Text(format!("answer {}", FEED_TURNS + 2)))
        );
    }

    #[test]
    fn task_changes_become_short_lines_in_one_progress_card() {
        let mut view = View::default();
        view.apply(&event(
            1,
            None,
            "plan_proposed",
            json!({"tasks": [{"id": "t1", "title": "Storage"}, {"id": "t2", "title": "API"}]}),
        ));
        let task = |seq, id: &str, status: &str, round, reason: Option<&str>| {
            event(
                seq,
                None,
                "task",
                json!({"id": id, "task": format!("u-{id}"), "status": status,
                       "round": round, "reason": reason}),
            )
        };
        for e in [
            task(2, "t1", "queued", 0, None),
            task(3, "t1", "working", 1, None),
            task(4, "t1", "verifying", 1, None),
            task(5, "t1", "working", 2, None),
            // The same state again says nothing new.
            task(6, "t1", "working", 2, None),
            task(7, "t1", "approved", 2, None),
            task(
                8,
                "t2",
                "needs_user",
                3,
                Some("the verifier repeated itself"),
            ),
        ] {
            view.apply(&e);
        }
        let progress: Vec<&TranscriptCard> = view
            .cards
            .iter()
            .filter(|c| matches!(c, TranscriptCard::System { title, .. } if title == "Progress"))
            .collect();
        assert_eq!(progress.len(), 1);
        let TranscriptCard::System { text, tone, .. } = progress[0] else {
            unreachable!()
        };
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines,
            [
                "↺ t1 Storage: round 2 after the verifier asked for changes",
                "✓ t1 Storage approved (1/2)",
                "! t2 API needs your decision — the verifier repeated itself",
            ]
        );
        assert_eq!(*tone, crate::lean::tui::Tone::Waiting);

        // Another card in between starts a new progress card.
        view.cards.push(TranscriptCard::System {
            title: "Notice".into(),
            meta: String::new(),
            text: "x".into(),
            tone: crate::lean::tui::Tone::Queued,
        });
        view.apply(&task(9, "t2", "failed", 3, None));
        let count = view
            .cards
            .iter()
            .filter(|c| matches!(c, TranscriptCard::System { title, .. } if title == "Progress"))
            .count();
        assert_eq!(count, 2);
    }

    #[test]
    fn mail_for_the_reviewers_stays_with_the_reviewers() {
        assert_eq!(View::mail_key("plan-verifier"), "plan-verifier");
        assert_eq!(View::mail_key("auditor"), "auditor");
        assert_eq!(View::mail_key("runtime"), "runtime");
        assert_eq!(View::mail_key("comment-orchestrator"), "orchestrator");
        assert_eq!(View::mail_key("t1"), "worker:t1");
        assert_eq!(View::mail_key("t1/verifier"), "verifier:t1");
    }

    #[test]
    fn the_session_keeps_prompts_reasoning_output_and_whole_answers() {
        let mut view = View::default();
        let key = "orchestrator";
        let turn = Some("t");
        let answer = "line one\nline two\nline three\nline four\nline five\nline six\nline seven";
        for (seq, kind, payload) in [
            (1, "turn_started", json!({"role": key, "provider": "codex"})),
            (2, "agent", json!({"type": "prompt", "text": "Do it"})),
            (3, "agent", json!({"type": "reasoning", "delta": "Think"})),
            (
                4,
                "agent",
                json!({"type": "tool", "name": "shell", "detail": "ls"}),
            ),
            (
                5,
                "agent",
                json!({"type": "output", "name": "shell", "text": "a.rs", "exit_code": 0}),
            ),
            (6, "agent", json!({"type": "text", "delta": answer})),
            (7, "turn_finished", json!({"role": key, "status": "done"})),
        ] {
            view.apply(&event(
                seq,
                turn.map(str::to_owned).as_deref(),
                kind,
                payload,
            ));
        }
        let feed = &view.agents[key].feed;
        assert!(matches!(feed[0], Block::Turn(_)));
        assert_eq!(feed[1], Block::Prompt("Do it".into()));
        assert_eq!(feed[2], Block::Reasoning("Think".into()));
        assert!(matches!(&feed[3], Block::Tool { name, .. } if name == "shell"));
        assert_eq!(
            feed[4],
            Block::Output {
                name: "shell".into(),
                text: "a.rs".into(),
                exit_code: Some(0)
            }
        );
        // The answer is kept whole, not cut to its first lines.
        assert_eq!(feed[5], Block::Text(answer.into()));
        assert!(matches!(feed[6], Block::End { ok: true, .. }));
    }

    #[test]
    fn the_stage_hides_the_graph_until_work_runs_and_again_when_replanning() {
        assert_eq!(View::default().stage(), Stage::Fresh);
        assert!(!View::default().graph_hidden());
        for (statuses, activity, stage) in [
            (vec![], false, Stage::Interview),
            (vec![], true, Stage::Interview),
            (vec!["proposed"], false, Stage::Review),
            (vec!["working", "queued"], true, Stage::Executing),
            // A rejected plan is revised: the graph goes away again.
            (vec!["cancelled"], true, Stage::Planning),
            (vec!["cancelled"], false, Stage::Blocked),
            (vec!["approved", "needs_user"], false, Stage::Blocked),
            (vec!["approved"], false, Stage::Done),
            // The orchestrator reporting on a finished plan keeps the graph.
            (vec!["approved"], true, Stage::Done),
        ] {
            let view = staged(&statuses, activity);
            assert_eq!(view.stage(), stage, "{statuses:?} {activity}");
            assert_eq!(
                view.graph_hidden(),
                matches!(stage, Stage::Interview | Stage::Planning | Stage::Review)
            );
        }
    }

    #[test]
    fn plan_projection_preserves_summaries_and_legacy_empty_values() {
        for summary in [
            Value::Null,
            json!(" \n\t "),
            json!("\x1b[31m\x1b[0m"),
            json!("Popraw zapis danych."),
        ] {
            let mut view = View::default();
            view.apply(&event(
                1,
                None,
                "plan_proposed",
                json!({"tasks": [{
                    "id": "t1", "title": "Storage", "summary": summary, "goal": "Full goal",
                    "verifier": {"summary": summary, "focus": "Full focus", "risks": ["Data loss"]}
                }]}),
            ));
            assert_eq!(view.tasks[0].summary.as_deref(), summary.as_str());
            assert_eq!(view.tasks[0].verifier_summary.as_deref(), summary.as_str());
            assert_eq!(view.tasks[0].goal, "Full goal");
            assert_eq!(view.tasks[0].focus, "Full focus");
            assert_eq!(view.tasks[0].risks, ["Data loss"]);
        }
        let mut view = View::default();
        view.apply(&event(
            1,
            None,
            "plan_proposed",
            json!({"tasks": [{"id": "old"}]}),
        ));
        assert!(view.tasks[0].summary.is_none());
        assert!(view.tasks[0].verifier_summary.is_none());
    }

    #[test]
    fn the_view_follows_a_conversation_and_its_tasks() {
        let mut view = View::default();
        for event in [
            event(
                1,
                None,
                "message",
                json!({"author": "user", "text": "analiza"}),
            ),
            event(
                2,
                Some("o1"),
                "turn_started",
                json!({"role": "orchestrator", "provider": "codex"}),
            ),
            event(
                3,
                Some("o1"),
                "agent",
                json!({"type": "tool", "name": "exec", "detail": "ls"}),
            ),
            event(
                4,
                None,
                "plan_proposed",
                json!({"tasks": [{"id": "t1", "title": "Store", "writes": false}]}),
            ),
            event(
                5,
                None,
                "task",
                json!({"id": "t1", "task": "uuid1", "title": "Store", "status": "working", "round": 1}),
            ),
            event(
                6,
                Some("w1"),
                "turn_started",
                json!({"role": "worker:uuid1", "provider": "agy"}),
            ),
            event(
                7,
                Some("w1"),
                "agent",
                json!({"type": "tool", "name": "view_file", "detail": "src/x.rs"}),
            ),
        ] {
            view.apply(&event);
        }
        assert_eq!(view.after, 7);
        assert_eq!(view.activity.as_deref(), Some("orchestrator: exec ls"));
        assert_eq!(view.tasks[0].status, "working");
        assert_eq!(view.tasks[0].activity, "worker: view_file src/x.rs");
        assert!(!view.plan_proposed());
        view.apply(&event(
            8,
            Some("o1"),
            "turn_finished",
            json!({"status": "failed", "error": "limit"}),
        ));
        assert_eq!(view.activity, None);
        assert!(view.transcript.last().unwrap().text.contains("limit"));
    }

    #[test]
    fn reviews_and_audits_are_shown_with_their_findings() {
        let mut view = View::default();
        view.apply(&event(
            1,
            None,
            "plan_proposed",
            json!({"plan": "p1", "tasks": [{"id": "t1", "title": "A"}],
                   "review": {"approved": false, "error": "login required"},
                   "automatic": false, "waiting": "the plan verifier did not approve this plan"}),
        ));
        view.apply(&event(
            2,
            None,
            "audit",
            json!({"plan": "p1", "approved": false,
                   "findings": [{"severity": "high", "issue": "README missing"}]}),
        ));
        let text: Vec<&str> = view
            .transcript
            .iter()
            .map(|entry| entry.text.as_str())
            .collect();
        assert!(text.contains(&"⚠ plan verifier failed: login required"));
        assert!(
            text.iter()
                .any(|line| line.contains("Waiting for you") && line.contains("Ctrl+A"))
        );
        assert!(text.contains(&"⚠ audit: issues found"));
        assert!(text.contains(&"  - [high] README missing"));
        assert!(view.plan_proposed());
    }

    #[test]
    fn messages_to_agents_show_their_delivery() {
        let mut view = View::default();
        view.apply(&event(
            1,
            None,
            "mail",
            json!({"id": 7, "sender": "user", "recipient": "t2", "kind": "user_instruction", "text": "użyj UTF-8"}),
        ));
        view.apply(&event(
            2,
            None,
            "mail",
            json!({"id": 8, "sender": "user", "recipient": "orchestrator", "kind": "copy", "text": "x"}),
        ));
        view.apply(&event(
            3,
            Some("turn"),
            "mail_delivered",
            json!({"ids": [7, 8]}),
        ));
        let text: Vec<&str> = view
            .transcript
            .iter()
            .map(|entry| entry.text.as_str())
            .collect();
        assert_eq!(
            text,
            [
                "✉ You → t2 (user_instruction): użyj UTF-8 · waiting for recipient's next turn",
                "✓ t2 received message"
            ]
        );
    }

    #[test]
    fn wrapping_breaks_words_and_long_runs() {
        assert_eq!(wrap("ala ma kota", 7), ["ala ma ", "kota"]);
        assert_eq!(wrap("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(wrap("a\n\nb", 10), ["a", "", "b"]);
        assert_eq!(clean("a\u{1b}[31mb\tc"), "ab    c");
        assert_eq!(wrap("a\u{1b}]0;x\u{7}b", 10), ["ab"]);
    }

    #[test]
    fn messages_and_cards_are_stored_structurally_with_actual_states() {
        let mut view = View::default();
        view.apply(&event(
            1,
            None,
            "message",
            json!({"author": "user", "text": "start"}),
        ));
        assert_eq!(view.messages.len(), 1);
        assert_eq!(view.messages[0].sender, "You");
        assert_eq!(view.messages[0].target, "orchestrator");
        assert_eq!(view.messages[0].state, "sent");

        view.apply(&event(
            2,
            None,
            "mail",
            json!({"id": 101, "sender": "user", "recipient": "t1", "kind": "instruction", "text": "do it"}),
        ));
        assert_eq!(view.messages.len(), 2);
        assert_eq!(view.messages[1].id, "101");
        assert_eq!(view.messages[1].state, "waiting for turn");

        view.apply(&event(
            3,
            Some("turn"),
            "mail_delivered",
            json!({"ids": [101]}),
        ));
        assert_eq!(view.messages[1].state, "delivered");
        if let TranscriptCard::Message(msg) = &view.cards[1] {
            assert_eq!(msg.state, "delivered");
        } else {
            panic!("Expected message card");
        }
    }

    #[test]
    fn agents_keep_their_sessions_and_messages_between_them() {
        let mut view = View::default();
        for event in [
            event(
                1,
                None,
                "plan_proposed",
                json!({"plan": "p", "tasks": [{"id": "t1", "title": "A"}, {"id": "t2", "title": "B", "depends_on": ["t1"]}]}),
            ),
            event(
                2,
                None,
                "task",
                json!({"id": "t1", "task": "u1", "status": "verifying", "round": 1}),
            ),
            event(
                3,
                Some("v1"),
                "turn_started",
                json!({"role": "verifier:u1", "provider": "agy"}),
            ),
            event(
                4,
                Some("v1"),
                "agent",
                json!({"type": "tool", "name": "read", "detail": "a.rs"}),
            ),
            event(
                5,
                None,
                "mail",
                json!({"id": 1, "sender": "t1/verifier", "recipient": "t2", "kind": "finding", "text": "x"}),
            ),
            event(
                6,
                None,
                "audit",
                json!({"plan": "p", "approved": true, "findings": []}),
            ),
        ] {
            view.apply(&event);
        }
        assert_eq!(view.tasks[1].depends_on, ["t1"]);
        let verifier = &view.agents["verifier:t1"];
        assert!(verifier.running);
        assert_eq!(verifier.provider, "agy");
        assert_eq!(verifier.session, ["── turn", "↳ read a.rs"]);
        let link = &view.links[&("verifier:t1".to_owned(), "worker:t2".to_owned())];
        assert_eq!(link.count, 1);
        assert_eq!(view.mailbox["worker:t2"], ["t1/verifier → t2 (finding): x"]);
        assert_eq!(view.reviews[AUDITOR], ["✓ auditor: no issues"]);
        assert_eq!(view.progress(), ["t1 verifying: verifier: read a.rs"]);
    }

    #[test]
    fn comments_are_listed_on_their_element_and_marked_stale_on_a_new_plan() {
        let mut view = View::default();
        let plan = |seq, id: &str| {
            event(
                seq,
                None,
                "plan_proposed",
                json!({"plan": id, "tasks": [{"id": "t1", "title": "A"}]}),
            )
        };
        view.apply(&plan(1, "p1"));
        view.apply(&event(
            2,
            None,
            "message",
            json!({"author": "user", "text": "[user comment #1 on the worker of task t1] x"}),
        ));
        for (seq, id, target, status) in [
            (3, 1, "worker:t1", "open"),
            (4, 2, "diff:t1:src/a.rs:3", "open"),
            (5, 3, "verifier:t1", "open"),
            (6, 3, "verifier:t1", "deleted"),
        ] {
            view.apply(&event(
                seq,
                None,
                "comment",
                json!({"id": id, "plan_id": "p1", "target": target, "label": "l", "text": "x",
                       "status": status, "message": 2}),
            ));
        }
        let ids = |key| -> Vec<u64> { view.comments_on(key).iter().map(|c| c.id).collect() };
        assert_eq!(
            ids("worker:t1"),
            [2, 1],
            "a diff's comments count for its worker"
        );
        assert!(ids("verifier:t1").is_empty());
        let text: Vec<&str> = view.transcript[2..]
            .iter()
            .map(|e| e.text.as_str())
            .collect();
        assert_eq!(
            text,
            [
                "✎ #1 to t1 worker: x",
                "✎ #2 to t1 diff src/a.rs:3: x",
                "✎ #3 to t1 verifier: x",
                "✎ #3 deleted"
            ],
            "the comment's message is not shown twice"
        );
        assert!(!view.stale(&view.comments[&1]));
        view.apply(&plan(7, "p2"));
        assert!(view.stale(&view.comments[&1]));
    }

    #[test]
    fn turn_times_add_up_only_when_known() {
        let mut view = View::default();
        let at = |seq, id: &str, kind: &str, ms: i64| {
            let mut event = event(
                seq,
                Some(id),
                kind,
                json!({"role": "worker:uuid1", "status": "done"}),
            );
            event["created_ms"] = json!(ms);
            event
        };
        for event in [
            at(1, "a", "turn_started", 0),
            at(2, "a", "turn_finished", 5_000),
            at(3, "b", "turn_started", 10_000),
            at(4, "b", "turn_finished", 85_000),
        ] {
            view.apply(&event);
        }
        let agent = view.agents.values().next().unwrap();
        assert_eq!(agent.turns, 2);
        assert_eq!((agent.busy_ms, agent.last_ms), (75_000, Some(75_000)));
        assert_eq!(duration(agent.busy_ms), "1 min 15 s");
    }

    #[test]
    fn token_use_is_what_the_cli_reported_and_unknown_otherwise() {
        let mut view = View::default();
        let turn = |seq, id: &str, input: Value| {
            [
                event(
                    seq,
                    Some(id),
                    "turn_started",
                    json!({"role": "worker:uuid1", "provider": "codex"}),
                ),
                event(
                    seq + 1,
                    Some(id),
                    "agent",
                    json!({"type": "usage", "input": input, "output": null}),
                ),
                event(
                    seq + 2,
                    Some(id),
                    "turn_finished",
                    json!({"status": "done"}),
                ),
            ]
        };
        for event in turn(1, "a", Value::Null) {
            view.apply(&event);
        }
        let agent = |view: &View| view.agents.values().next().unwrap().tokens();
        assert_eq!(agent(&view), "unavailable (CLI did not report them)");
        for event in turn(4, "b", json!(120)) {
            view.apply(&event);
        }
        assert_eq!(
            agent(&view),
            "in 120 / out unknown · reported in 1 of 2 turns, rest unknown"
        );
    }

    #[test]
    fn ac_resp_01_responsive_breakpoint_compact_switch() {
        // Breakpoint: width 84 (< 85) -> compact mode with banner
        let header_chat = render_header(
            "proj",
            "conv-12345",
            Some(true),
            "ready",
            PanelFocus::Chat,
            84,
        );
        let chat_text: String = header_chat
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(chat_text.contains("[Chat]"));
        assert!(chat_text.contains("Team"));
        assert!(chat_text.contains("(Tab: switch)"));

        let header_graph = render_header(
            "proj",
            "conv-12345",
            Some(true),
            "ready",
            PanelFocus::Graph,
            84,
        );
        let graph_text: String = header_graph
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(graph_text.contains("Chat"));
        assert!(graph_text.contains("[Team]"));
        assert!(graph_text.contains("(Tab: switch)"));
    }

    #[test]
    fn ac_resp_02_wide_header() {
        // Wide: width 85 (>= 85) -> side-by-side mode, no compact toggle badge
        let header_wide = render_header(
            "proj",
            "conv-12345",
            Some(true),
            "ready",
            PanelFocus::Chat,
            85,
        );
        let wide_text: String = header_wide
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(!wide_text.contains("[Chat]"));
        assert!(!wide_text.contains("(Tab: switch)"));
        assert!(wide_text.contains("proj"));
        assert!(wide_text.contains("conv-123"));
        assert!(wide_text.contains("autonomous"));
        assert!(wide_text.contains("ready"));
    }

    #[test]
    fn ac_resp_03_minimum_terminal_guard() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        // Terminal size 11x4 is smaller than 12x5 guard floor
        let backend = TestBackend::new(11, 4);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut guard_triggered = false;
        terminal
            .draw(|frame| {
                guard_triggered = draw_min_term_guard(frame, frame.area());
            })
            .unwrap();
        assert!(guard_triggered);
        let buffer = terminal.backend().buffer();
        let text: String = buffer.content.iter().map(|c| c.symbol()).collect();
        let norm = text.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(norm.contains("Terminal too small (min. 12x5 required)"));

        // Terminal size 12x5 meets minimum guard
        let backend = TestBackend::new(12, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut guard_triggered_ok = false;
        terminal
            .draw(|frame| {
                guard_triggered_ok = draw_min_term_guard(frame, frame.area());
            })
            .unwrap();
        assert!(!guard_triggered_ok);
    }

    #[test]
    fn ac_copy_02_polish_status_strings() {
        let header = render_header(
            "test-proj",
            "conv-12345",
            Some(false),
            "ready",
            PanelFocus::Chat,
            120,
        );
        let text: String = header.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("antislop"));
        assert!(text.contains("supervised"));
        assert!(text.contains("ready"));

        let header_auto = render_header(
            "test-proj",
            "conv-12345",
            Some(true),
            "tasks in progress…",
            PanelFocus::Chat,
            120,
        );
        let text_auto: String = header_auto
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(text_auto.contains("autonomous"));
        assert!(text_auto.contains("tasks in progress…"));
    }

    #[test]
    fn main_footer_hints_prioritized_by_width() {
        // Chat hints
        let wide = main_footer_hint(PanelFocus::Chat, false, false, None, 120);
        assert!(wide.contains("Alt+Enter newline"));
        assert!(wide.contains("F1 help"));

        let medium = main_footer_hint(PanelFocus::Chat, false, false, None, 80);
        assert!(medium.contains("Ctrl+P"));

        let narrow = main_footer_hint(PanelFocus::Chat, false, false, None, 55);
        assert!(narrow.contains("Esc"));
        assert!(!narrow.contains("Alt+Enter"));

        // Graph hints
        let graph_wide = main_footer_hint(PanelFocus::Graph, false, false, None, 120);
        assert!(graph_wide.contains("arrows select"));
        assert!(graph_wide.contains("d diff"));

        let graph_narrow = main_footer_hint(PanelFocus::Graph, false, false, None, 55);
        assert!(graph_narrow.contains("Tab chat"));
    }

    #[test]
    fn transcript_empty_state_polish() {
        let lines = transcript_empty_state(80);
        let text = lines[0].spans[0].content.as_ref();
        assert_eq!(text, "No messages in this conversation.");
    }
}

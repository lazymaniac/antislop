//! What the screen shows, built only from the conversation's events, so a
//! frontend that attaches later rebuilds the same state.

use super::super::tasks::status;
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
const SESSION_LINES: usize = 400;

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
    pub kind: String,
    pub goal: String,
    pub scope: Vec<String>,
    pub depends_on: Vec<String>,
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
    /// The current tool call while running, else the last outcome.
    pub activity: String,
    pub session: Vec<String>,
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
            return "niedostępne (CLI ich nie zgłosił)".into();
        }
        let whole = if self.reported >= self.turns {
            String::new()
        } else {
            format!(
                " · zgłoszone w {} z {} tur, reszta nieznana",
                self.reported, self.turns
            )
        };
        let side = |sum: Option<u64>| sum.map_or("nieznane".to_owned(), |sum| sum.to_string());
        format!(
            "we {} / wy {}{whole}",
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

#[derive(Debug, Default)]
pub struct View {
    pub transcript: Vec<Entry>,
    pub tasks: Vec<TaskRow>,
    /// The plan the task rows show.
    pub plan: Option<String>,
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
    /// Recipient of each shown message to an agent, by mail id.
    mail: HashMap<u64, String>,
    /// Task row index by the task's database id.
    task_rows: HashMap<String, usize>,
}

pub fn dim() -> Style {
    Style::default().fg(Color::DarkGray)
}

impl View {
    fn push(&mut self, style: Style, text: impl Into<String>) {
        self.transcript.push(Entry {
            style,
            text: clean(&text.into()),
        });
    }

    pub fn plan_proposed(&self) -> bool {
        !self.tasks.is_empty() && self.tasks.iter().all(|task| task.status == "proposed")
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
            ORCHESTRATOR | "user" => name.to_owned(),
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
                        sender: "Ty".to_owned(),
                        target: "orkiestrator".to_owned(),
                        text: text("text"),
                        state: "wysłano".to_owned(),
                        kind: "user".to_owned(),
                    };
                    self.messages.push(msg.clone());
                    self.cards.push(TranscriptCard::Message(msg));
                    self.push(
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                        "Ty",
                    );
                    self.push(Style::default(), text("text"));
                    self.push(Style::default(), "");
                }
                "runtime" => {
                    self.cards.push(TranscriptCard::System {
                        title: "Runtime".into(),
                        meta: String::new(),
                        text: "· wyniki zadań przekazane orkiestratorowi".into(),
                        tone: super::theme::Tone::Queued,
                    });
                    self.push(dim(), "· wyniki zadań przekazane orkiestratorowi");
                    self.push(Style::default(), "");
                }
                author => {
                    let display_author = match author {
                        ORCHESTRATOR => "orkiestrator".to_owned(),
                        PLAN_VERIFIER => "weryfikator planu".to_owned(),
                        author => author.to_owned(),
                    };
                    let msg = MessageEntry {
                        id: format!("m{}", self.after),
                        sender: display_author.clone(),
                        target: "Ty".to_owned(),
                        text: text("text"),
                        state: "doręczono".to_owned(),
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
                    self.activity = Some(format!("orkiestrator ({provider}) pracuje…"));
                } else if let Some(row) = self.row_of(&key) {
                    let who = key.split(':').next().unwrap_or_default();
                    row.activity = format!("{who} ({provider})");
                }
                let agent = self.agents.entry(key.clone()).or_default();
                agent.provider = provider;
                agent.model = payload["model"].as_str().map(str::to_owned);
                agent.effort = payload["effort"].as_str().map(str::to_owned);
                agent.running = true;
                agent.started_ms = event["created_ms"].as_i64().filter(|ms| *ms > 0);
                agent.activity = "zaczyna turę".into();
                self.session(&key, "── tura".into());
                self.roles.insert(turn.to_owned(), key);
            }
            "agent" if payload["type"] == "tool" => {
                let detail = format!("{} {}", text("name"), text("detail"));
                if key == ORCHESTRATOR {
                    self.activity = Some(format!("orkiestrator: {detail}"));
                    self.push(dim(), format!("  ↳ {detail}"));
                } else if let Some(row) = self.row_of(&key) {
                    let who = key.split(':').next().unwrap_or_default().to_owned();
                    row.activity = format!("{who}: {detail}");
                }
                if !key.is_empty() {
                    self.agents.entry(key.clone()).or_default().activity = detail.clone();
                    self.session(&key, format!("↳ {detail}"));
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
                }
            }
            "turn_finished" => {
                let failed = payload["status"] != "done";
                if key == ORCHESTRATOR {
                    self.activity = None;
                    if failed {
                        self.push(
                            Style::default().fg(Color::Red),
                            format!("✗ orkiestrator: {} — {}", text("status"), text("error")),
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
                        "✓ tura zakończona".into()
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
                    self.session(&key, outcome);
                }
            }
            "plan_proposed" => {
                self.plan = payload["plan"].as_str().map(str::to_owned);
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
                    .map(|task| TaskRow {
                        id: task["id"].as_str().unwrap_or_default().to_owned(),
                        uuid: None,
                        title: task["title"].as_str().unwrap_or_default().to_owned(),
                        kind: task["kind"].as_str().unwrap_or_default().to_owned(),
                        goal: task["goal"].as_str().unwrap_or_default().to_owned(),
                        scope: strings(&task["scope"]),
                        depends_on: strings(&task["depends_on"]),
                        writes: task["writes"] == true,
                        focus: task["verifier"]["focus"]
                            .as_str()
                            .unwrap_or_default()
                            .to_owned(),
                        risks: strings(&task["verifier"]["risks"]),
                        status: "proposed".into(),
                        round: 0,
                        activity: if task["writes"] == true {
                            "zmienia pliki".into()
                        } else {
                            String::new()
                        },
                        reason: None,
                        scope_hash: task["scope_hash"].as_str().map(str::to_owned),
                    })
                    .collect();
                self.task_rows.clear();
                self.agents
                    .retain(|key, _| !key.starts_with("worker:") && !key.starts_with("verifier:"));
                self.links.clear();
                self.mailbox.clear();
                for line in super::super::cli::review_lines("weryfikator planu", &payload["review"])
                {
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
                    "uruchamiam (tryb autonomiczny).".to_owned()
                } else {
                    let waiting = payload["waiting"]
                        .as_str()
                        .map(|reason| format!("Czeka na Ciebie: {reason}. "))
                        .unwrap_or_default();
                    format!("{waiting}Ctrl+A uruchamia, albo napisz, co zmienić.")
                };
                let plan_text = format!("Plan: {} zadań. {next}", self.tasks.len());
                self.cards.push(TranscriptCard::Plan {
                    plan: self.plan.clone().unwrap_or_default(),
                    tasks: self.tasks.len(),
                    text: plan_text.clone(),
                });
                self.push(Style::default().fg(Color::Yellow), plan_text);
                self.push(Style::default(), "");
            }
            "plan_review" => self.review(PLAN_VERIFIER, payload),
            "plan_invalid" => {
                let err = text("error");
                self.cards.push(TranscriptCard::System {
                    title: "Plan odrzucony".into(),
                    meta: String::new(),
                    text: format!("✗ {err}"),
                    tone: super::theme::Tone::Failed,
                });
                self.push(
                    Style::default().fg(Color::Red),
                    format!("✗ plan odrzucony: {err}"),
                );
            }
            "task" => {
                let id = text("id");
                let index = self.tasks.iter().position(|row| row.id == id);
                if let Some(index) = index {
                    self.task_rows.insert(text("task"), index);
                    let row = &mut self.tasks[index];
                    row.uuid = Some(text("task"));
                    row.status = text("status");
                    row.round = payload["round"].as_u64().unwrap_or_default();
                    row.reason = payload["reason"].as_str().map(str::to_owned);
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
                }
            }
            "paused" => {
                self.paused = payload["paused"] == true;
                let txt = if self.paused {
                    "⏸ wstrzymano: nowe kroki czekają (/continue wznawia)"
                } else {
                    "▶ wznowiono"
                };
                self.cards.push(TranscriptCard::System {
                    title: "Wstrzymanie".into(),
                    meta: String::new(),
                    text: txt.into(),
                    tone: super::theme::Tone::Waiting,
                });
                self.push(Style::default().fg(Color::Yellow), txt);
            }
            "notice" => {
                let txt = text("text");
                self.cards.push(TranscriptCard::System {
                    title: "Uwaga".into(),
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
                        "⚙ tryb autonomiczny: kolejne plany i rundy ruszają same"
                    } else {
                        "⚙ tryb nadzorowany: kolejne plany czekają na zgodę (Ctrl+A)"
                    },
                );
            }
            "integration" => {
                let approved = payload["status"] == "approved";
                self.cards.push(TranscriptCard::System {
                    title: format!("[{}] {}", text("id"), text("title")),
                    meta: if approved {
                        "zatwierdzono".into()
                    } else {
                        "weryfikacja".into()
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
                    "user" => "Ty".to_owned(),
                    ORCHESTRATOR => "orkiestrator".to_owned(),
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
                    state: "czeka na turę".to_owned(),
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
                    format!("✉ {line} · czeka na następną turę odbiorcy"),
                );
                let from = Self::mail_key(&text("sender"));
                let to = Self::mail_key(&recipient);
                for key in [&from, &to] {
                    self.mailbox
                        .entry(key.clone())
                        .or_default()
                        .push(clean(&line));
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
                                msg.state = "dostarczono".to_owned();
                            }
                        }
                        for card in &mut self.cards {
                            if let TranscriptCard::Message(msg) = card
                                && msg.id == id_str
                            {
                                msg.state = "dostarczono".to_owned();
                            }
                        }
                        if let Some(recipient) = self.mail.remove(&mail_id) {
                            self.push(dim(), format!("✓ {recipient} otrzymał wiadomość"));
                        }
                    }
                }
            }
            "mail_rejected" => {
                let sender = text("sender");
                let recipient = text("recipient");
                let norm_sender = match sender.as_str() {
                    "user" => "Ty",
                    ORCHESTRATOR => "orkiestrator",
                    s => s,
                };
                for msg in self.messages.iter_mut().rev() {
                    if (msg.sender == sender || msg.sender == norm_sender)
                        && msg.target == recipient
                    {
                        msg.state = "odrzucono".to_owned();
                        break;
                    }
                }
                for card in self.cards.iter_mut().rev() {
                    if let TranscriptCard::Message(msg) = card
                        && (msg.sender == sender || msg.sender == norm_sender)
                        && msg.target == recipient
                    {
                        msg.state = "odrzucono".to_owned();
                        break;
                    }
                }
                self.push(
                    Style::default().fg(Color::Red),
                    format!(
                        "✗ wiadomość od {} do {} niedostarczalna: {}",
                        text("sender"),
                        text("recipient"),
                        text("reason")
                    ),
                );
            }
            "audit" => {
                for line in super::super::cli::review_lines("audyt", payload) {
                    self.push(Style::default().fg(Color::Yellow), line);
                }
                self.review(AUDITOR, payload);
            }
            "code_review" => {
                let task = text("id");
                if payload["status"] == "running" {
                    self.push(dim(), format!("· recenzja kodu {task} w toku"));
                } else {
                    let who = format!("recenzja kodu {task}");
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
                let (line, card_status) = match comment.status.as_str() {
                    "open" if !self.comments.contains_key(&comment.id) => (
                        format!("✎ #{} do {name}: {}", comment.id, comment.text),
                        "otwarty".into(),
                    ),
                    "open" => (
                        format!("✎ #{} otwarty ponownie", comment.id),
                        "ponownie otwarty".into(),
                    ),
                    "resolved" => (format!("✎ #{} rozwiązany", comment.id), "rozwiązany".into()),
                    _ => (format!("✎ #{} usunięty", comment.id), "usunięty".into()),
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
            lines.push("⏸ wstrzymane: nowe kroki czekają".into());
        }
        if let Some(activity) = &self.activity {
            lines.push(clean(activity));
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
                "waiting" => lines.push(format!("{} czeka na logowanie/limit", task.id)),
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
                "czeka na Ciebie: plan do akceptacji".into()
            } else if self.plan_resumable() {
                "czeka na Ciebie: plan do wznowienia".into()
            } else {
                "bezczynny".into()
            });
        }
        lines
    }
}

fn review_word(review: &Value) -> String {
    if review["error"].is_string() {
        "✗ nie zadziałał".into()
    } else if review["approved"] == true {
        "✓ zatwierdził".into()
    } else {
        "⚠ zastrzeżenia".into()
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
            "plan" => return format!("Plan {rest}"),
            "task" => return format!("Zadanie {rest}"),
            "check" => {
                if let Some((task_id, check_id)) = rest.split_once(':') {
                    return format!("Zadanie {task_id} kontrola {check_id}");
                }
                return format!("Kontrola {rest}");
            }
            "finding" => {
                if let Some((task_id, finding_id)) = rest.split_once(':') {
                    return format!("Zadanie {task_id} ustalenie #{finding_id}");
                }
                return format!("Ustalenie {rest}");
            }
            "report" => return format!("Zadanie {rest} raport"),
            "artifact" => {
                if let Some((task_id, path)) = rest.split_once(':') {
                    return format!("Zadanie {task_id} artefakt {path}");
                }
                return format!("Artefakt {rest}");
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
        Some(("verifier", id)) => format!("{id} weryfikator"),
        _ => match key {
            ORCHESTRATOR => "orkiestrator".into(),
            PLAN_VERIFIER => "weryfikator planu".into(),
            AUDITOR => "audytor".into(),
            "conversation" | "rozmowa" => "rozmowa".into(),
            other => other.into(),
        },
    }
}

pub fn polish(status: &str) -> &str {
    match status {
        "proposed" => "proponowane",
        "queued" => "w kolejce",
        "working" => "worker pracuje",
        "verifying" => "weryfikacja",
        "approved" => "zatwierdzone",
        "needs_user" => "wymaga decyzji",
        "failed" => "nieudane",
        "skipped" => "pominięte",
        "cancelled" => "anulowane",
        "waiting" => "czeka na logowanie/limit",
        "revised" => "odesłane",
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
        let msg = format!("Terminal zbyt mały (wymagane min. {MIN_TERM_WIDTH}x{MIN_TERM_HEIGHT})");
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
        Some(true) => "autonomiczny · ",
        Some(false) => "nadzorowany · ",
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
            PanelFocus::Chat => ("[Czat]", "Zadania", "Zespół"),
            PanelFocus::Tasks => ("Czat", "[Zadania]", "Zespół"),
            PanelFocus::Graph => ("Czat", "Zadania", "[Zespół]"),
        };
        spans.push(Span::styled(format!(" {chat_label} "), chat_style));
        spans.push(Span::styled(format!("{tasks_label} "), tasks_style));
        spans.push(Span::styled(format!("{graph_label} "), graph_style));
        let switch_hint = match focus {
            PanelFocus::Chat => "(Tab: przełącz) · (t: zadania) · ",
            PanelFocus::Tasks => "(Tab: przełącz) · (Esc: czat) · ",
            PanelFocus::Graph => "(Tab: przełącz) · (t: zadania) · ",
        };
        if width >= 75 {
            spans.push(Span::styled(switch_hint, dim()));
        } else if width >= 65 {
            spans.push(Span::styled("(Tab: przełącz) · ", dim()));
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

    if width >= 40 {
        spans.push(Span::styled(
            clean(status),
            Style::default().fg(Color::Yellow),
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
        "Ctrl+A uruchom plan · "
    } else if plan_resumable {
        "Ctrl+R wznów plan · "
    } else {
        ""
    };
    match focus {
        PanelFocus::Chat => {
            if width >= 115 {
                format!(
                    "Enter wyślij (@t2 … do agenta) · Alt+Enter nowa linia · {plan}Ctrl+P polecenia · F1 pomoc · Tab zespół · Esc wyjdź"
                )
            } else if width >= 75 {
                format!(
                    "Enter wyślij · Alt+Enter nowa linia · {plan}Ctrl+P · F1 pomoc · Tab zespół · Esc"
                )
            } else if width >= 50 {
                format!("Enter wyślij · {plan}Tab zespół · Esc")
            } else {
                "Enter · Esc".into()
            }
        }
        PanelFocus::Tasks => {
            if width >= 115 {
                format!(
                    "←/→ zadanie · w worker · v weryfikator · Enter szczegóły · d diff · c skomentuj · {plan}Tab zespół · Esc czat"
                )
            } else if width >= 75 {
                "←/→ zadanie · w worker · v wer. · Enter szczegóły · d diff · Tab zespół · Esc"
                    .into()
            } else if width >= 50 {
                "←/→ zadanie · w worker · v wer. · Tab zespół".into()
            } else {
                "←/→ zadanie · w · v".into()
            }
        }
        PanelFocus::Graph => {
            if width >= 115 {
                format!(
                    "strzałki wybór · Enter szczegóły · / znajdź · d diff · c skomentuj · f filtr · {plan}Ctrl+P polecenia · ? pomoc · q wyjdź"
                )
            } else if width >= 75 {
                "strzałki wybór · Enter szczegóły · / szukaj · d diff · c kom. · f filtr · ? pomoc · q".into()
            } else if width >= 50 {
                "strzałki · Enter szczegóły · Tab czat · q".into()
            } else {
                "strzałki · q".into()
            }
        }
    }
}

/// Polish empty state displayed when transcript contains no messages.
pub fn transcript_empty_state(width: usize) -> Vec<Line<'static>> {
    vec![
        Line::styled("Brak wiadomości w tej rozmowie.", dim()),
        Line::styled(
            truncate(
                "Wpisz wiadomość orkiestratorowi poniżej lub naciśnij Ctrl+P, aby otworzyć paletę.",
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
        assert_eq!(view.activity.as_deref(), Some("orkiestrator: exec ls"));
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
        assert!(text.contains(&"⚠ weryfikator planu nie zadziałał: login required"));
        assert!(
            text.iter()
                .any(|line| line.contains("Czeka na Ciebie") && line.contains("Ctrl+A"))
        );
        assert!(text.contains(&"⚠ audyt: zastrzeżenia"));
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
                "✉ Ty → t2 (user_instruction): użyj UTF-8 · czeka na następną turę odbiorcy",
                "✓ t2 otrzymał wiadomość"
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
        assert_eq!(view.messages[0].sender, "Ty");
        assert_eq!(view.messages[0].target, "orkiestrator");
        assert_eq!(view.messages[0].state, "wysłano");

        view.apply(&event(
            2,
            None,
            "mail",
            json!({"id": 101, "sender": "user", "recipient": "t1", "kind": "instruction", "text": "do it"}),
        ));
        assert_eq!(view.messages.len(), 2);
        assert_eq!(view.messages[1].id, "101");
        assert_eq!(view.messages[1].state, "czeka na turę");

        view.apply(&event(
            3,
            Some("turn"),
            "mail_delivered",
            json!({"ids": [101]}),
        ));
        assert_eq!(view.messages[1].state, "dostarczono");
        if let TranscriptCard::Message(msg) = &view.cards[1] {
            assert_eq!(msg.state, "dostarczono");
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
        assert_eq!(verifier.session, ["── tura", "↳ read a.rs"]);
        let link = &view.links[&("verifier:t1".to_owned(), "worker:t2".to_owned())];
        assert_eq!(link.count, 1);
        assert_eq!(view.mailbox["worker:t2"], ["t1/verifier → t2 (finding): x"]);
        assert_eq!(view.reviews[AUDITOR], ["✓ audytor: bez zastrzeżeń"]);
        assert_eq!(view.progress(), ["t1 weryfikacja: verifier: read a.rs"]);
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
                "✎ #1 do t1 worker: x",
                "✎ #2 do t1 diff src/a.rs:3: x",
                "✎ #3 do t1 weryfikator: x",
                "✎ #3 usunięty"
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
        assert_eq!(agent(&view), "niedostępne (CLI ich nie zgłosił)");
        for event in turn(4, "b", json!(120)) {
            view.apply(&event);
        }
        assert_eq!(
            agent(&view),
            "we 120 / wy nieznane · zgłoszone w 1 z 2 tur, reszta nieznana"
        );
    }

    #[test]
    fn ac_resp_01_responsive_breakpoint_compact_switch() {
        // Breakpoint: width 84 (< 85) -> compact mode with banner
        let header_chat = render_header(
            "proj",
            "conv-12345",
            Some(true),
            "gotowy",
            PanelFocus::Chat,
            84,
        );
        let chat_text: String = header_chat
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(chat_text.contains("[Czat]"));
        assert!(chat_text.contains("Zespół"));
        assert!(chat_text.contains("(Tab: przełącz)"));

        let header_graph = render_header(
            "proj",
            "conv-12345",
            Some(true),
            "gotowy",
            PanelFocus::Graph,
            84,
        );
        let graph_text: String = header_graph
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(graph_text.contains("Czat"));
        assert!(graph_text.contains("[Zespół]"));
        assert!(graph_text.contains("(Tab: przełącz)"));
    }

    #[test]
    fn ac_resp_02_wide_header() {
        // Wide: width 85 (>= 85) -> side-by-side mode, no compact toggle badge
        let header_wide = render_header(
            "proj",
            "conv-12345",
            Some(true),
            "gotowy",
            PanelFocus::Chat,
            85,
        );
        let wide_text: String = header_wide
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(!wide_text.contains("[Czat]"));
        assert!(!wide_text.contains("(Tab: przełącz)"));
        assert!(wide_text.contains("proj"));
        assert!(wide_text.contains("conv-123"));
        assert!(wide_text.contains("autonomiczny"));
        assert!(wide_text.contains("gotowy"));
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
        assert!(norm.contains("Terminal zbyt mały (wymagane min. 12x5)"));

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
            "gotowy",
            PanelFocus::Chat,
            120,
        );
        let text: String = header.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("antislop"));
        assert!(text.contains("nadzorowany"));
        assert!(text.contains("gotowy"));

        let header_auto = render_header(
            "test-proj",
            "conv-12345",
            Some(true),
            "zadania w toku…",
            PanelFocus::Chat,
            120,
        );
        let text_auto: String = header_auto
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(text_auto.contains("autonomiczny"));
        assert!(text_auto.contains("zadania w toku…"));
    }

    #[test]
    fn main_footer_hints_prioritized_by_width() {
        // Chat hints
        let wide = main_footer_hint(PanelFocus::Chat, false, false, None, 120);
        assert!(wide.contains("Alt+Enter nowa linia"));
        assert!(wide.contains("F1 pomoc"));

        let medium = main_footer_hint(PanelFocus::Chat, false, false, None, 80);
        assert!(medium.contains("Ctrl+P"));

        let narrow = main_footer_hint(PanelFocus::Chat, false, false, None, 55);
        assert!(narrow.contains("Esc"));
        assert!(!narrow.contains("Alt+Enter"));

        // Graph hints
        let graph_wide = main_footer_hint(PanelFocus::Graph, false, false, None, 120);
        assert!(graph_wide.contains("strzałki wybór"));
        assert!(graph_wide.contains("d diff"));

        let graph_narrow = main_footer_hint(PanelFocus::Graph, false, false, None, 55);
        assert!(graph_narrow.contains("Tab czat"));
    }

    #[test]
    fn transcript_empty_state_polish() {
        let lines = transcript_empty_state(80);
        let text = lines[0].spans[0].content.as_ref();
        assert_eq!(text, "Brak wiadomości w tej rozmowie.");
    }
}

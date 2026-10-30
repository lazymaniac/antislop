//! Small rows, each written on its own, plus an append-only event log. Nothing
//! here grows with the size of the project the agents work on.

#[path = "plan_revision.rs"]
mod plan_revision;
pub use plan_revision::PlanRevision;
pub(in crate::lean) use plan_revision::RevisionLease;
#[path = "history.rs"]
mod history;
#[path = "run_context.rs"]
mod run_context;
#[path = "workflow.rs"]
mod workflow;
pub use history::{AgentMessage, ContentOwner, HistoryScope};
pub use run_context::{RunAccess, RunContext};
pub use workflow::Workflow;

use crate::{Error, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::sync::Mutex;

#[path = "store/agent_exchange.rs"]
mod agent_exchange;
#[path = "store/plan_reviews.rs"]
mod plan_reviews;
#[path = "store/revision_jobs.rs"]
mod revision_jobs;
#[path = "store/task_progress.rs"]
mod task_progress;

pub struct Store {
    connection: Mutex<Connection>,
    content: super::content::Content,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub project: String,
    pub created_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub seq: u64,
    pub author: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Turn {
    pub id: String,
    pub role: String,
    pub status: TurnStatus,
    pub session_id: Option<String>,
    pub error: Option<String>,
    /// The last user message this turn answers.
    pub upto_message: u64,
    #[serde(default)]
    pub plan_id: Option<String>,
}

/// One normalized mailbox address with pending mail and its owning task, if any.
pub struct MailTarget {
    pub role: String,
    pub newest: u64,
    pub task_id: Option<String>,
    pub plan_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub seq: u64,
    pub turn_id: Option<String>,
    #[serde(default)]
    pub plan_id: Option<String>,
    pub kind: String,
    pub payload: Value,
    /// When it was recorded; 0 for events from before schema 2.
    pub created_ms: i64,
}

/// A message to one agent, delivered into its next turn. `sender` and
/// `recipient` are display names (`user`, `orchestrator`, a task id such as
/// `t2`); `role` is the recipient's session role.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mail {
    pub id: u64,
    pub sender: String,
    pub recipient: String,
    pub kind: String,
    pub text: String,
    #[serde(default)]
    pub created_ms: i64,
    #[serde(default)]
    pub delivered_turn: Option<String>,
    #[serde(default)]
    pub delivered_ms: Option<i64>,
    #[serde(default = "default_mail_state")]
    pub state: String,
}

fn default_mail_state() -> String {
    "queued".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub turn_id: String,
    #[serde(default)]
    pub plan_id: Option<String>,
    pub role: String,
    pub provider: String,
    pub session_id: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub upto_message: u64,
    pub started_ms: i64,
    pub finished_ms: Option<i64>,
    pub pid: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanAgentInfo {
    pub role: String,
    pub provider: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub task_label: Option<String>,
    pub turns_count: usize,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskReportInfo {
    pub task_id: String,
    pub task_label: String,
    pub round: u32,
    pub report: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskVerdictInfo {
    pub task_id: String,
    pub task_label: String,
    pub round: u32,
    pub verdict: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanHistory {
    pub run_id: Option<String>,
    pub id: String,
    pub conversation_id: String,
    pub created_ms: i64,
    pub status: String,
    pub is_active: bool,
    pub tasks: Vec<Task>,
    pub agents: Vec<PlanAgentInfo>,
    pub turns: Vec<SessionInfo>,
    pub reports: Vec<TaskReportInfo>,
    pub verdicts: Vec<TaskVerdictInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlansPage {
    pub plans: Vec<PlanHistory>,
    pub offset: usize,
    pub limit: usize,
    pub total: usize,
    pub has_more: bool,
    pub active_plan_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryPage {
    pub events: Vec<Event>,
    pub after: u64,
    pub next_after: Option<u64>,
    pub has_more: bool,
    pub total: usize,
    pub oldest_seq: u64,
    pub latest_seq: u64,
    pub history_unavailable: bool,
}

/// The user's comment on one element of the team panel or of a diff.
/// `target` is the element's key (`worker:t2`, `diff:t2:src/a.rs:14`), `label`
/// how the orchestrator reads it, `plan_id` the plan it was made on (a later
/// plan makes it stale), `message` the conversation message that carried it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Comment {
    pub id: u64,
    pub plan_id: Option<String>,
    pub target: String,
    pub label: String,
    pub text: String,
    /// `open`, `resolved` or `deleted`.
    pub status: String,
    pub message: u64,
}

/// One task of a plan. `spec` is what the orchestrator asked for; `report`
/// is the worker's latest answer and `verdict` its verifier's latest one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    #[serde(default)]
    pub input_generation: i64,
    pub id: String,
    pub plan_id: String,
    pub spec: Value,
    pub status: String,
    pub round: u32,
    pub report: Option<String>,
    pub verdict: Option<Value>,
    /// A writer's starting commit and its latest squashed result.
    pub base: Option<String>,
    pub result: Option<String>,
    /// The compact feedback a card shows for the current round: a bounded
    /// effect summary, coverage of the real obligations, and one unambiguous
    /// integration outcome. `None` until a round produces any of it; a new
    /// round or retry clears it, so a card never shows an earlier round's data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness_token: Option<String>,
    /// What the verifier has settled so far in the current round, one entry
    /// per check (`id`, `state`, `status`, `note`). Filled when a task is
    /// read for a frontend, never stored on the task row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checks: Option<Value>,
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// The layout of `core.db` this build writes (`PRAGMA user_version`).
/// Incompatible databases are never modified or migrated.
const SCHEMA: u32 = 14;

#[path = "store/comment_mail_state.rs"]
mod comment_mail_state;
#[path = "store/comments_and_mail.rs"]
mod comments_and_mail;
#[path = "store/history_queries.rs"]
mod history_queries;
#[path = "store/mail_and_events.rs"]
mod mail_and_events;
#[path = "store/messages_and_turns.rs"]
mod messages_and_turns;
#[path = "store/plan_history.rs"]
mod plan_history;
#[path = "store/plan_tasks.rs"]
mod plan_tasks;
#[path = "store/sessions_and_plans.rs"]
mod sessions_and_plans;
#[path = "store/setup_and_conversations.rs"]
mod setup_and_conversations;
#[path = "store/turn_status.rs"]
mod turn_status;
pub use turn_status::TurnStatus;

#[cfg(test)]
#[path = "store/tests.rs"]
mod tests;

//! Lean core: native CLI agents working on the project's own files, small
//! rows of state and an event log. See docs/lean-core.md.

pub mod agent;
pub mod cli;
pub mod content;
mod core;
mod core_agent_review;
mod core_agent_turn;
mod core_change_review;
mod core_comment_at;
mod core_comment_helpers;
mod core_config_snapshot;
mod core_integration;
mod core_integration_repair;
mod core_orchestrator_turn;
mod core_plan_answer;
mod core_plan_proposal;
mod core_plan_report;
mod core_plan_resume;
mod core_plan_review;
mod core_plan_revision;
mod core_planner_turn;
mod core_repeat_plan;
mod core_start_plan;
mod core_task_checks;
mod core_task_execution;
mod core_task_freshness;
mod core_task_inputs;
mod core_task_recovery;
mod core_unanswered;
mod core_unblock_dependents;
mod core_writer_base;
pub use core_comment_helpers::comment_label;
mod agent_tools;
mod core_feedback_helpers;
mod core_path_helpers;
mod core_plan_helpers;
mod core_planning_context;
mod core_prompt_helpers;
pub mod daemon;
pub mod diff;
mod feasibility;
pub mod git;
pub mod interview;
mod launch;
pub mod ledger;
mod mail;
pub mod mcp;
pub mod native;
#[cfg(test)]
mod plan_tests;
mod progress;
pub mod projects;
#[cfg(test)]
mod reliability_tests;
pub mod risks;
mod rounds;
pub mod store;
mod task_failure;
pub mod tasks;
pub mod terminal;
pub mod trust;
pub mod tui;
pub mod turn_result;

use crate::{
    Error, Result,
    config::{Config, ResolvedProfile, RoleKind, WorkKind},
    require,
};
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use store::{
    Comment, Conversation, Mail, Message, PlanHistory, PlansPage, Store, Task, TurnStatus,
};
use tasks::{Outcome, TaskContract, Verdict, status};

/// A task role address. Bare task IDs are handled by callers that accept them;
/// storage and UI callers must only recognize explicit worker/verifier roles.
pub(crate) fn task_role(key: &str) -> Option<(&str, bool)> {
    key.strip_prefix("verifier:")
        .map(|id| (id, true))
        .or_else(|| key.strip_prefix("worker:").map(|id| (id, false)))
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ConversationSummary {
    pub id: String,
    pub project: String,
    pub created_ms: i64,
    pub status: String,
    pub active_plan: Option<String>,
    pub tasks_total: usize,
    pub tasks_done: usize,
    pub scope_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryAction {
    IntegrateWithConsent,
    KeepOnBranch,
    VerifyManualSnapshot,
    RetryWithGuidance,
    RestartAgent,
    CancelTask,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskRecoveryRequest {
    pub conversation: String,
    pub plan: String,
    pub task: String,
    pub action: RecoveryAction,
    #[serde(default)]
    pub consent_files: Option<Vec<String>>,
    pub freshness_token: String,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TaskRecoveryResponse {
    pub task: String,
    pub status: String,
    pub round: u32,
    pub integrated_files: Vec<String>,
    pub preserved_branch: Option<String>,
    pub new_freshness_token: String,
    pub notice: String,
}

/// Scope hash of a plan, capturing its task ids, specs, statuses and rounds.
pub fn plan_scope_hash(tasks: &[Task]) -> String {
    let mut data = String::new();
    for task in tasks {
        data.push_str(&task.id);
        data.push('\0');
        data.push_str(&task.plan_id);
        data.push('\0');
        data.push_str(&task.status);
        data.push('\0');
        data.push_str(&task.input_generation.to_string());
        data.push_str(&task.round.to_string());
        data.push('\0');
        data.push_str(&task.spec.to_string());
        data.push('\n');
    }
    if data.is_empty() {
        crate::hash("empty-plan".as_bytes())
    } else {
        crate::hash(data.as_bytes())
    }
}

/// Scope hash of a diff, capturing the commit range and file changes.
pub fn diff_scope_hash(
    conversation: &str,
    plan: &str,
    id: &str,
    base: &str,
    head: Option<&str>,
) -> String {
    crate::hash(format!("{conversation}\0{plan}\0{id}\0{base}\0{:?}", head).as_bytes())
}

/// Scope hash of a task stage.
pub fn task_scope_hash(task: &Task) -> String {
    crate::hash(
        format!(
            "{}:{}:{}:{}:{}",
            task.id, task.status, task.round, task.input_generation, task.spec
        )
        .as_bytes(),
    )
}

const ORCHESTRATOR: &str = "orchestrator";
/// Plan feedback has an independent native CLI session.
pub const COMMENT_ORCHESTRATOR: &str = "comment-orchestrator";
/// Drafts the first plan; its own native CLI session and execution profile.
pub const PLANNER: &str = "planner";
/// Author of the orchestrator's request that the planner redraft the plan.
pub const REPLAN: &str = "replan-request";
/// The sessions that answer the user's messages, in the order they are asked.
pub const ORCHESTRATION_ROLES: [&str; 3] = [ORCHESTRATOR, COMMENT_ORCHESTRATOR, PLANNER];
const RUNTIME: &str = "runtime";
/// Author of a code review's findings, and how its message starts.
const CODE_REVIEW: &str = "code-review";
const CODE_REVIEW_PREFIX: &str = "[code review";
/// Largest patch the diff view gets.
const DIFF_LIMIT: usize = 2 * 1024 * 1024;
/// How a comment's message starts, before its id.
pub const COMMENT_PREFIX: &str = "[user comment #";
/// Checks each plan before the user sees it.
const PLAN_VERIFIER: &str = "plan-verifier";
const PLAN_LINT: &str = "plan-lint";
/// Checks, after a plan ran, that the user's request was fully delivered.
const AUDITOR: &str = "auditor";

pub struct Core {
    pub store: Store,
    pub(crate) native: native::Registry,
    pub(crate) weak: std::sync::OnceLock<std::sync::Weak<Core>>,
    /// One process owns a state directory at a time.
    _lock: std::fs::File,
    config: RwLock<Arc<Config>>,
    config_update: Mutex<()>,
    config_modified: std::sync::atomic::AtomicU64,
    /// Private state directory; writers' worktrees live under it.
    state: PathBuf,
    /// Writers land on the user's branch one at a time.
    integrating: Mutex<()>,
    graph_updates: Mutex<()>,
    revising: Mutex<std::collections::BTreeSet<String>>,
    revision_jobs: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// Per conversation, the plan and revision its orchestrator was last
    /// shown; `revise_plan` edits exactly that graph.
    revision_views: Mutex<HashMap<String, Option<(String, i64)>>>,
    agent_reviews: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// Active task processes can be cancelled independently.
    task_cancels: Mutex<HashMap<String, (String, Arc<AtomicBool>)>>,
    task_restarts: Mutex<HashMap<String, TaskRecoveryRequest>>,
    orchestrator_restarts: Mutex<std::collections::HashSet<String>>,
    turn_locks: Mutex<mail::TurnLocks>,
    /// Conversations with an orchestrator loop running, and its cancel flag.
    running: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// Conversations with a plan running, and its cancel flag.
    plans: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// Conversations with a code review the user asked for, and its flag.
    reviews: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// The blocked report last handed to each conversation's orchestrator,
    /// so that an unchanged block is told once.
    blocked_seen: Mutex<HashMap<String, String>>,
}

impl Drop for Core {
    fn drop(&mut self) {
        let _ = self.native.shutdown();
    }
}

/// Who runs a turn, where, and with what access.
struct PlanProposal<'a> {
    config: &'a Config,
    specs: &'a [TaskContract],
}

#[derive(Clone, Debug)]
enum SessionPolicy {
    Role,
    /// A complete local contract is supplied; no prior provider context is inherited.
    Independent,
    TaskRound {
        task: String,
        round: u32,
    },
}

struct TurnSetup<'a> {
    session_policy: SessionPolicy,
    conversation: &'a str,
    plan_id: Option<String>,
    role: String,
    profile: Result<ResolvedProfile>,
    cwd: PathBuf,
    add_dirs: Vec<PathBuf>,
    access: agent::Access,
    upto_message: u64,
}

//! The daemon owns a project's `Core`, so work goes on after a frontend
//! detaches. One request per connection, one JSON line each way. Every reply
//! is small; `events` is a long poll that returns as soon as something new is
//! recorded.

use super::Core;
use crate::{Error, Result, config::Config, require};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

mod client;
mod config_write;
pub use client::Client;
mod dispatch;
mod lifecycle;
#[cfg(test)]
use config_write::atomic_config_write;
#[cfg(test)]
use config_write::{PreparedConfigWrite, cleanup_config_temporaries, config_temporary_prefix};

/// Creates `path` if needed and insists it is a real directory private to
/// the current user.
pub fn private_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    if !path.exists() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)?;
    }
    let metadata = std::fs::symlink_metadata(path)?;
    // SAFETY: geteuid has no arguments or failure modes.
    let uid = unsafe { libc::geteuid() };
    require(
        metadata.is_dir()
            && !metadata.file_type().is_symlink()
            && metadata.uid() == uid
            && metadata.permissions().mode() & 0o077 == 0,
        &format!(
            "{} must be a directory owned by the current user with mode 0700",
            path.display()
        ),
    )
}

/// Identifies a build of the executable: path, size and modification time.
pub fn build_identity(executable: &Path) -> Result<String> {
    let path = executable.canonicalize()?;
    let metadata = std::fs::metadata(&path)?;
    let modified = metadata
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    Ok(crate::hash(format!(
        "{}\0{}\0{modified}",
        path.display(),
        metadata.len()
    )))
}

const PROTOCOL_VERSION: u32 = 3;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestEnvelope {
    pub(crate) protocol: u32,
    pub(crate) request: Request,
}

// Protocol ownership handoff: Request, WireRequest, their conversions, and
// serde wire-representation implementations belong to runtime-protocol.
// Daemon retains lifecycle and dispatch; keep protocol behavior compatible.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    TerminalOpen {
        conversation: String,
        role: String,
        columns: u16,
        rows: u16,
        retry: bool,
    },
    TerminalRead {
        conversation: String,
        role: String,
        handle: String,
        after: u64,
    },
    TerminalInput {
        conversation: String,
        role: String,
        handle: String,
        sequence: u64,
        input: super::terminal::Input,
    },
    TerminalResize {
        conversation: String,
        role: String,
        handle: String,
        columns: u16,
        rows: u16,
    },
    TerminalDetach {
        conversation: String,
        role: String,
        handle: String,
    },
    RunContext {
        conversation: String,
        turn: String,
    },
    CommentAt {
        conversation: String,
        plan: Option<String>,
        target: String,
        text: String,
        round: Option<u32>,
        run: Option<String>,
    },

    SendToAgent {
        message: super::store::AgentMessage,
    },
    TaskRounds {
        conversation: String,
        plan: String,
        id: String,
    },
    TaskRound {
        conversation: String,
        plan: String,
        id: String,
        round: u32,
        references: Option<bool>,
    },
    SessionEvents {
        conversation: String,
        turn: String,
        after: u64,
        limit: Option<usize>,
        include_messages: Option<bool>,
    },
    HistoryContent {
        conversation: String,
        owner: super::store::ContentOwner,
        offset: u64,
    },
    HistoryMessages {
        conversation: String,
        scope: super::store::HistoryScope,
        after: u64,
        limit: Option<usize>,
    },
    SessionContent {
        conversation: String,
        event: u64,
        field: String,
        offset: u64,
    },

    ListRuns {
        conversation: String,
    },
    RejectLedgerProposal {
        conversation: String,
        run_id: String,
        proposal_id: String,
        diff_hash: String,
        comment: String,
    },
    GetLedgerProposal {
        conversation: String,
        run_id: String,
    },

    ListRequirements {
        query: super::ledger::LedgerQuery,
    },
    GetLedgerEntry {
        conversation: String,
        run_id: String,
        id: String,
        revision: Option<i64>,
    },
    GetLedgerDiff {
        conversation: String,
        run_id: String,
        from_revision: i64,
        to_revision: i64,
    },
    GetEvidence {
        conversation: String,
        run_id: String,
        entry_id: Option<String>,
    },
    ProposeLedgerChange {
        change: super::ledger::LedgerChange,
    },
    ApplyLedgerChange {
        proposal_id: String,
        diff_hash: String,
        change: super::ledger::LedgerChange,
    },
    CreateRun {
        conversation: String,
        goal: String,
    },
    AddLedgerSource {
        conversation: String,
        run_id: String,
        text: String,
    },
    GetLedgerSource {
        conversation: String,
        run_id: String,
        source_id: String,
    },

    /// Which binary the daemon runs and whether it is executing work.
    Identity,
    GetConfig,
    SetConfig {
        toml: String,
        expected_revision: String,
    },
    /// Stops an idle daemon so a frontend from another build can replace it.
    Shutdown,
    ForceShutdown,
    Conversations,
    Start,
    StartIn {
        project: String,
    },
    Send {
        conversation: String,
        text: String,
    },
    SendAddressed {
        conversation: String,
        text: String,
        to: Option<String>,
    },
    /// Starts exactly the plan the user saw.
    Accept {
        conversation: String,
        plan: String,
    },
    AcceptScoped {
        conversation: String,
        plan: String,
        scope_hash: Option<String>,
    },
    /// Rejects a proposed plan and asks the orchestrator to revise it.
    Reject {
        conversation: String,
        plan: String,
        reason: Option<String>,
    },
    RejectScoped {
        conversation: String,
        plan: String,
        reason: Option<String>,
        scope_hash: Option<String>,
    },
    /// Runs again what the plan left unfinished, through the same sessions.
    Resume {
        conversation: String,
        plan: String,
    },
    ResumeScoped {
        conversation: String,
        plan: String,
        scope_hash: Option<String>,
    },
    RestartOrchestrator {
        conversation: String,
    },
    Cancel {
        conversation: String,
    },
    CancelScoped {
        conversation: String,
        scope_hash: Option<String>,
    },
    /// Stops starting new plan work, or continues it.
    Pause {
        conversation: String,
        paused: bool,
    },
    /// Runs the conversation on by itself, or under the user, from now on.
    Mode {
        conversation: String,
        autonomous: bool,
    },
    /// Unblocks a task that stopped: see [`super::RecoveryAction`].
    Recover(super::TaskRecoveryRequest),
    /// The user finished commenting on the plan: the orchestrator answers
    /// the draft comments together.
    ReviewComments {
        conversation: String,
        plan: String,
    },
    /// Paginated read of all stored plans for a conversation.
    Plans {
        conversation: String,
        offset: Option<usize>,
        limit: Option<usize>,
    },
    /// Detailed history of a single plan.
    Plan {
        conversation: String,
        plan: String,
    },
    /// Comments of a conversation, optionally filtered by plan and/or status.
    Comments {
        conversation: String,
        plan: Option<String>,
        status: Option<String>,
    },
    /// The user's comment on one element of the team panel or a diff.
    Comment {
        conversation: String,
        target: String,
        text: String,
    },
    /// The user's comment scoped to a specific plan.
    CommentScoped {
        conversation: String,
        plan: Option<String>,
        target: String,
        text: String,
    },
    /// Resolves, reopens (`open`) or deletes a comment.
    CommentStatus {
        conversation: String,
        id: u64,
        status: String,
    },
    /// A writer's change as files and numbered lines.
    Diff {
        conversation: String,
        plan: String,
        id: String,
    },
    /// Reviews a writer's change now; findings go to the orchestrator.
    Review {
        conversation: String,
        plan: String,
        id: String,
    },
    ReviewScoped {
        conversation: String,
        plan: String,
        id: String,
        scope_hash: Option<String>,
    },
    /// One task of a plan with its latest report and verdict.
    Task {
        plan: String,
        id: String,
    },
    /// Inspector snapshot and checks without a Git freshness fingerprint.
    TaskChecks {
        conversation: Option<String>,
        plan: String,
        id: String,
    },
    TaskScoped {
        conversation: Option<String>,
        plan: String,
        id: String,
    },
    Events {
        conversation: String,
        after: u64,
        wait_ms: u64,
    },
    EventsPaged {
        conversation: String,
        after: u64,
        wait_ms: u64,
        limit: Option<usize>,
    },
    Sessions {
        conversation: String,
    },
    History {
        conversation: String,
        after: u64,
        limit: Option<usize>,
    },
    Mail {
        conversation: String,
    },
    TrustStatus,
    Trust {
        trusted: bool,
    },
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum WireRequest {
    TerminalOpen {
        conversation: String,
        role: String,
        columns: u16,
        rows: u16,
        #[serde(default)]
        retry: bool,
    },
    TerminalRead {
        conversation: String,
        role: String,
        handle: String,
        after: u64,
    },
    TerminalInput {
        conversation: String,
        role: String,
        handle: String,
        sequence: u64,
        input: super::terminal::Input,
    },
    TerminalResize {
        conversation: String,
        role: String,
        handle: String,
        columns: u16,
        rows: u16,
    },
    TerminalDetach {
        conversation: String,
        role: String,
        handle: String,
    },
    RunContext {
        conversation: String,
        turn: String,
    },
    CommentAt {
        conversation: String,
        plan: Option<String>,
        target: String,
        text: String,
        round: Option<u32>,
        run: Option<String>,
    },

    SendToAgent {
        message: super::store::AgentMessage,
    },
    TaskRounds {
        conversation: String,
        plan: String,
        id: String,
    },
    TaskRound {
        conversation: String,
        plan: String,
        id: String,
        round: u32,
        #[serde(default)]
        references: Option<bool>,
    },
    SessionEvents {
        conversation: String,
        turn: String,
        after: u64,
        limit: Option<usize>,
        #[serde(default)]
        include_messages: Option<bool>,
    },
    HistoryContent {
        conversation: String,
        owner: super::store::ContentOwner,
        offset: u64,
    },
    HistoryMessages {
        conversation: String,
        scope: super::store::HistoryScope,
        after: u64,
        limit: Option<usize>,
    },
    SessionContent {
        conversation: String,
        event: u64,
        field: String,
        offset: u64,
    },

    ListRuns {
        conversation: String,
    },
    RejectLedgerProposal {
        conversation: String,
        run_id: String,
        proposal_id: String,
        diff_hash: String,
        comment: String,
    },
    GetLedgerProposal {
        conversation: String,
        run_id: String,
    },

    ListRequirements {
        query: super::ledger::LedgerQuery,
    },
    GetLedgerEntry {
        conversation: String,
        run_id: String,
        id: String,
        revision: Option<i64>,
    },
    GetLedgerDiff {
        conversation: String,
        run_id: String,
        from_revision: i64,
        to_revision: i64,
    },
    GetEvidence {
        conversation: String,
        run_id: String,
        entry_id: Option<String>,
    },
    ProposeLedgerChange {
        change: super::ledger::LedgerChange,
    },
    ApplyLedgerChange {
        proposal_id: String,
        diff_hash: String,
        change: super::ledger::LedgerChange,
    },
    CreateRun {
        conversation: String,
        goal: String,
    },
    AddLedgerSource {
        conversation: String,
        run_id: String,
        text: String,
    },
    GetLedgerSource {
        conversation: String,
        run_id: String,
        source_id: String,
    },

    Identity,
    GetConfig,
    SetConfig {
        toml: String,
        expected_revision: String,
    },
    Shutdown,
    ForceShutdown,
    Conversations,
    Start {
        #[serde(default)]
        project: Option<String>,
    },
    Send {
        conversation: String,
        text: String,
        #[serde(default)]
        to: Option<String>,
    },
    Accept {
        conversation: String,
        plan: String,
        #[serde(default)]
        scope_hash: Option<String>,
    },
    Reject {
        conversation: String,
        plan: String,
        #[serde(default)]
        reason: Option<String>,
        #[serde(default)]
        scope_hash: Option<String>,
    },
    Resume {
        conversation: String,
        plan: String,
        #[serde(default)]
        scope_hash: Option<String>,
    },
    ReviewComments {
        conversation: String,
        plan: String,
    },
    Recover {
        conversation: String,
        plan: String,
        task: String,
        action: super::RecoveryAction,
        #[serde(default)]
        consent_files: Option<Vec<String>>,
        freshness_token: String,
        #[serde(default)]
        message: Option<String>,
    },
    RestartOrchestrator {
        conversation: String,
    },
    Cancel {
        conversation: String,
        #[serde(default)]
        scope_hash: Option<String>,
    },
    Pause {
        conversation: String,
        paused: bool,
    },
    Mode {
        conversation: String,
        autonomous: bool,
    },
    Plans {
        conversation: String,
        #[serde(default)]
        offset: Option<usize>,
        #[serde(default)]
        limit: Option<usize>,
    },
    Plan {
        conversation: String,
        plan: String,
    },
    Comments {
        conversation: String,
        #[serde(default)]
        plan: Option<String>,
        #[serde(default)]
        status: Option<String>,
    },
    Comment {
        conversation: String,
        #[serde(default)]
        plan: Option<String>,
        target: String,
        text: String,
    },
    CommentStatus {
        conversation: String,
        id: u64,
        status: String,
    },
    Diff {
        conversation: String,
        plan: String,
        id: String,
    },
    Review {
        conversation: String,
        plan: String,
        id: String,
        #[serde(default)]
        scope_hash: Option<String>,
    },
    TaskChecks {
        #[serde(default)]
        conversation: Option<String>,
        plan: String,
        id: String,
    },
    Task {
        #[serde(default)]
        conversation: Option<String>,
        plan: String,
        id: String,
    },
    Events {
        conversation: String,
        after: u64,
        wait_ms: u64,
        #[serde(default)]
        limit: Option<usize>,
    },
    Sessions {
        conversation: String,
    },
    History {
        conversation: String,
        #[serde(default)]
        after: u64,
        #[serde(default)]
        limit: Option<usize>,
    },
    Mail {
        conversation: String,
    },
    TrustStatus,
    Trust {
        #[serde(default = "default_true")]
        trusted: bool,
    },
}

impl From<Request> for WireRequest {
    fn from(req: Request) -> Self {
        match req {
            Request::CommentAt {
                conversation,
                plan,
                target,
                text,
                round,
                run,
            } => WireRequest::CommentAt {
                conversation,
                plan,
                target,
                text,
                round,
                run,
            },
            Request::SendToAgent { message } => WireRequest::SendToAgent { message },
            Request::TaskRounds {
                conversation,
                plan,
                id,
            } => WireRequest::TaskRounds {
                conversation,
                plan,
                id,
            },
            Request::TaskRound {
                conversation,
                plan,
                id,
                round,
                references,
            } => WireRequest::TaskRound {
                conversation,
                plan,
                id,
                round,
                references,
            },
            Request::TerminalOpen {
                conversation,
                role,
                columns,
                rows,
                retry,
            } => WireRequest::TerminalOpen {
                conversation,
                role,
                columns,
                rows,
                retry,
            },
            Request::TerminalRead {
                conversation,
                role,
                handle,
                after,
            } => WireRequest::TerminalRead {
                conversation,
                role,
                handle,
                after,
            },
            Request::TerminalInput {
                conversation,
                role,
                handle,
                sequence,
                input,
            } => WireRequest::TerminalInput {
                conversation,
                role,
                handle,
                sequence,
                input,
            },
            Request::TerminalResize {
                conversation,
                role,
                handle,
                columns,
                rows,
            } => WireRequest::TerminalResize {
                conversation,
                role,
                handle,
                columns,
                rows,
            },
            Request::TerminalDetach {
                conversation,
                role,
                handle,
            } => WireRequest::TerminalDetach {
                conversation,
                role,
                handle,
            },
            Request::RunContext { conversation, turn } => {
                WireRequest::RunContext { conversation, turn }
            }
            Request::SessionEvents {
                conversation,
                turn,
                after,
                limit,
                include_messages,
            } => WireRequest::SessionEvents {
                conversation,
                turn,
                after,
                limit,
                include_messages,
            },
            Request::HistoryContent {
                conversation,
                owner,
                offset,
            } => WireRequest::HistoryContent {
                conversation,
                owner,
                offset,
            },
            Request::HistoryMessages {
                conversation,
                scope,
                after,
                limit,
            } => WireRequest::HistoryMessages {
                conversation,
                scope,
                after,
                limit,
            },
            Request::SessionContent {
                conversation,
                event,
                field,
                offset,
            } => WireRequest::SessionContent {
                conversation,
                event,
                field,
                offset,
            },
            Request::ListRuns { conversation } => WireRequest::ListRuns { conversation },
            Request::RejectLedgerProposal {
                conversation,
                run_id,
                proposal_id,
                diff_hash,
                comment,
            } => WireRequest::RejectLedgerProposal {
                conversation,
                run_id,
                proposal_id,
                diff_hash,
                comment,
            },
            Request::GetLedgerProposal {
                conversation,
                run_id,
            } => WireRequest::GetLedgerProposal {
                conversation,
                run_id,
            },

            Request::ListRequirements { query } => WireRequest::ListRequirements { query },
            Request::GetLedgerEntry {
                conversation,
                run_id,
                id,
                revision,
            } => WireRequest::GetLedgerEntry {
                conversation,
                run_id,
                id,
                revision,
            },
            Request::GetLedgerDiff {
                conversation,
                run_id,
                from_revision,
                to_revision,
            } => WireRequest::GetLedgerDiff {
                conversation,
                run_id,
                from_revision,
                to_revision,
            },
            Request::GetEvidence {
                conversation,
                run_id,
                entry_id,
            } => WireRequest::GetEvidence {
                conversation,
                run_id,
                entry_id,
            },
            Request::ProposeLedgerChange { change } => WireRequest::ProposeLedgerChange { change },
            Request::ApplyLedgerChange {
                proposal_id,
                diff_hash,
                change,
            } => WireRequest::ApplyLedgerChange {
                proposal_id,
                diff_hash,
                change,
            },
            Request::CreateRun { conversation, goal } => {
                WireRequest::CreateRun { conversation, goal }
            }
            Request::AddLedgerSource {
                conversation,
                run_id,
                text,
            } => WireRequest::AddLedgerSource {
                conversation,
                run_id,
                text,
            },
            Request::GetLedgerSource {
                conversation,
                run_id,
                source_id,
            } => WireRequest::GetLedgerSource {
                conversation,
                run_id,
                source_id,
            },

            Request::Identity => WireRequest::Identity,
            Request::GetConfig => WireRequest::GetConfig,
            Request::SetConfig {
                toml,
                expected_revision,
            } => WireRequest::SetConfig {
                toml,
                expected_revision,
            },
            Request::Shutdown => WireRequest::Shutdown,
            Request::ForceShutdown => WireRequest::ForceShutdown,
            Request::Conversations => WireRequest::Conversations,
            Request::Start => WireRequest::Start { project: None },
            Request::StartIn { project } => WireRequest::Start {
                project: Some(project),
            },
            Request::Send { conversation, text } => WireRequest::Send {
                conversation,
                text,
                to: None,
            },
            Request::SendAddressed {
                conversation,
                text,
                to,
            } => WireRequest::Send {
                conversation,
                text,
                to,
            },
            Request::Accept { conversation, plan } => WireRequest::Accept {
                conversation,
                plan,
                scope_hash: None,
            },
            Request::AcceptScoped {
                conversation,
                plan,
                scope_hash,
            } => WireRequest::Accept {
                conversation,
                plan,
                scope_hash,
            },
            Request::Reject {
                conversation,
                plan,
                reason,
            } => WireRequest::Reject {
                conversation,
                plan,
                reason,
                scope_hash: None,
            },
            Request::RejectScoped {
                conversation,
                plan,
                reason,
                scope_hash,
            } => WireRequest::Reject {
                conversation,
                plan,
                reason,
                scope_hash,
            },
            Request::Resume { conversation, plan } => WireRequest::Resume {
                conversation,
                plan,
                scope_hash: None,
            },
            Request::ResumeScoped {
                conversation,
                plan,
                scope_hash,
            } => WireRequest::Resume {
                conversation,
                plan,
                scope_hash,
            },
            Request::ReviewComments { conversation, plan } => {
                WireRequest::ReviewComments { conversation, plan }
            }
            Request::Recover(req) => WireRequest::Recover {
                conversation: req.conversation,
                plan: req.plan,
                task: req.task,
                action: req.action,
                consent_files: req.consent_files,
                freshness_token: req.freshness_token,
                message: req.message,
            },
            Request::RestartOrchestrator { conversation } => {
                WireRequest::RestartOrchestrator { conversation }
            }
            Request::Cancel { conversation } => WireRequest::Cancel {
                conversation,
                scope_hash: None,
            },
            Request::CancelScoped {
                conversation,
                scope_hash,
            } => WireRequest::Cancel {
                conversation,
                scope_hash,
            },
            Request::Pause {
                conversation,
                paused,
            } => WireRequest::Pause {
                conversation,
                paused,
            },
            Request::Mode {
                conversation,
                autonomous,
            } => WireRequest::Mode {
                conversation,
                autonomous,
            },
            Request::Plans {
                conversation,
                offset,
                limit,
            } => WireRequest::Plans {
                conversation,
                offset,
                limit,
            },
            Request::Plan { conversation, plan } => WireRequest::Plan { conversation, plan },
            Request::Comments {
                conversation,
                plan,
                status,
            } => WireRequest::Comments {
                conversation,
                plan,
                status,
            },
            Request::Comment {
                conversation,
                target,
                text,
            } => WireRequest::Comment {
                conversation,
                plan: None,
                target,
                text,
            },
            Request::CommentScoped {
                conversation,
                plan,
                target,
                text,
            } => WireRequest::Comment {
                conversation,
                plan,
                target,
                text,
            },
            Request::CommentStatus {
                conversation,
                id,
                status,
            } => WireRequest::CommentStatus {
                conversation,
                id,
                status,
            },
            Request::Diff {
                conversation,
                plan,
                id,
            } => WireRequest::Diff {
                conversation,
                plan,
                id,
            },
            Request::Review {
                conversation,
                plan,
                id,
            } => WireRequest::Review {
                conversation,
                plan,
                id,
                scope_hash: None,
            },
            Request::ReviewScoped {
                conversation,
                plan,
                id,
                scope_hash,
            } => WireRequest::Review {
                conversation,
                plan,
                id,
                scope_hash,
            },
            Request::TaskChecks {
                conversation,
                plan,
                id,
            } => WireRequest::TaskChecks {
                conversation,
                plan,
                id,
            },
            Request::Task { plan, id } => WireRequest::Task {
                conversation: None,
                plan,
                id,
            },
            Request::TaskScoped {
                conversation,
                plan,
                id,
            } => WireRequest::Task {
                conversation,
                plan,
                id,
            },
            Request::Events {
                conversation,
                after,
                wait_ms,
            } => WireRequest::Events {
                conversation,
                after,
                wait_ms,
                limit: None,
            },
            Request::EventsPaged {
                conversation,
                after,
                wait_ms,
                limit,
            } => WireRequest::Events {
                conversation,
                after,
                wait_ms,
                limit,
            },
            Request::Sessions { conversation } => WireRequest::Sessions { conversation },
            Request::History {
                conversation,
                after,
                limit,
            } => WireRequest::History {
                conversation,
                after,
                limit,
            },
            Request::Mail { conversation } => WireRequest::Mail { conversation },
            Request::TrustStatus => WireRequest::TrustStatus,
            Request::Trust { trusted } => WireRequest::Trust { trusted },
        }
    }
}

impl From<WireRequest> for Request {
    fn from(wire: WireRequest) -> Self {
        match wire {
            WireRequest::CommentAt {
                conversation,
                plan,
                target,
                text,
                round,
                run,
            } => Request::CommentAt {
                conversation,
                plan,
                target,
                text,
                round,
                run,
            },
            WireRequest::SendToAgent { message } => Request::SendToAgent { message },
            WireRequest::TaskRounds {
                conversation,
                plan,
                id,
            } => Request::TaskRounds {
                conversation,
                plan,
                id,
            },
            WireRequest::TaskRound {
                conversation,
                plan,
                id,
                round,
                references,
            } => Request::TaskRound {
                conversation,
                plan,
                id,
                round,
                references,
            },
            WireRequest::TerminalOpen {
                conversation,
                role,
                columns,
                rows,
                retry,
            } => Request::TerminalOpen {
                conversation,
                role,
                columns,
                rows,
                retry,
            },
            WireRequest::TerminalRead {
                conversation,
                role,
                handle,
                after,
            } => Request::TerminalRead {
                conversation,
                role,
                handle,
                after,
            },
            WireRequest::TerminalInput {
                conversation,
                role,
                handle,
                sequence,
                input,
            } => Request::TerminalInput {
                conversation,
                role,
                handle,
                sequence,
                input,
            },
            WireRequest::TerminalResize {
                conversation,
                role,
                handle,
                columns,
                rows,
            } => Request::TerminalResize {
                conversation,
                role,
                handle,
                columns,
                rows,
            },
            WireRequest::TerminalDetach {
                conversation,
                role,
                handle,
            } => Request::TerminalDetach {
                conversation,
                role,
                handle,
            },
            WireRequest::RunContext { conversation, turn } => {
                Request::RunContext { conversation, turn }
            }
            WireRequest::SessionEvents {
                conversation,
                turn,
                after,
                limit,
                include_messages,
            } => Request::SessionEvents {
                conversation,
                turn,
                after,
                limit,
                include_messages,
            },
            WireRequest::HistoryContent {
                conversation,
                owner,
                offset,
            } => Request::HistoryContent {
                conversation,
                owner,
                offset,
            },
            WireRequest::HistoryMessages {
                conversation,
                scope,
                after,
                limit,
            } => Request::HistoryMessages {
                conversation,
                scope,
                after,
                limit,
            },
            WireRequest::SessionContent {
                conversation,
                event,
                field,
                offset,
            } => Request::SessionContent {
                conversation,
                event,
                field,
                offset,
            },
            WireRequest::ListRuns { conversation } => Request::ListRuns { conversation },
            WireRequest::RejectLedgerProposal {
                conversation,
                run_id,
                proposal_id,
                diff_hash,
                comment,
            } => Request::RejectLedgerProposal {
                conversation,
                run_id,
                proposal_id,
                diff_hash,
                comment,
            },
            WireRequest::GetLedgerProposal {
                conversation,
                run_id,
            } => Request::GetLedgerProposal {
                conversation,
                run_id,
            },

            WireRequest::ListRequirements { query } => Request::ListRequirements { query },
            WireRequest::GetLedgerEntry {
                conversation,
                run_id,
                id,
                revision,
            } => Request::GetLedgerEntry {
                conversation,
                run_id,
                id,
                revision,
            },
            WireRequest::GetLedgerDiff {
                conversation,
                run_id,
                from_revision,
                to_revision,
            } => Request::GetLedgerDiff {
                conversation,
                run_id,
                from_revision,
                to_revision,
            },
            WireRequest::GetEvidence {
                conversation,
                run_id,
                entry_id,
            } => Request::GetEvidence {
                conversation,
                run_id,
                entry_id,
            },
            WireRequest::ProposeLedgerChange { change } => Request::ProposeLedgerChange { change },
            WireRequest::ApplyLedgerChange {
                proposal_id,
                diff_hash,
                change,
            } => Request::ApplyLedgerChange {
                proposal_id,
                diff_hash,
                change,
            },
            WireRequest::CreateRun { conversation, goal } => {
                Request::CreateRun { conversation, goal }
            }
            WireRequest::AddLedgerSource {
                conversation,
                run_id,
                text,
            } => Request::AddLedgerSource {
                conversation,
                run_id,
                text,
            },
            WireRequest::GetLedgerSource {
                conversation,
                run_id,
                source_id,
            } => Request::GetLedgerSource {
                conversation,
                run_id,
                source_id,
            },

            WireRequest::Identity => Request::Identity,
            WireRequest::GetConfig => Request::GetConfig,
            WireRequest::SetConfig {
                toml,
                expected_revision,
            } => Request::SetConfig {
                toml,
                expected_revision,
            },
            WireRequest::Shutdown => Request::Shutdown,
            WireRequest::ForceShutdown => Request::ForceShutdown,
            WireRequest::Conversations => Request::Conversations,
            WireRequest::Start { project } => match project {
                Some(project) => Request::StartIn { project },
                None => Request::Start,
            },
            WireRequest::Send {
                conversation,
                text,
                to,
            } => {
                if to.is_some() {
                    Request::SendAddressed {
                        conversation,
                        text,
                        to,
                    }
                } else {
                    Request::Send { conversation, text }
                }
            }
            WireRequest::Accept {
                conversation,
                plan,
                scope_hash,
            } => {
                if scope_hash.is_some() {
                    Request::AcceptScoped {
                        conversation,
                        plan,
                        scope_hash,
                    }
                } else {
                    Request::Accept { conversation, plan }
                }
            }
            WireRequest::Reject {
                conversation,
                plan,
                reason,
                scope_hash,
            } => {
                if scope_hash.is_some() {
                    Request::RejectScoped {
                        conversation,
                        plan,
                        reason,
                        scope_hash,
                    }
                } else {
                    Request::Reject {
                        conversation,
                        plan,
                        reason,
                    }
                }
            }
            WireRequest::Resume {
                conversation,
                plan,
                scope_hash,
            } => {
                if scope_hash.is_some() {
                    Request::ResumeScoped {
                        conversation,
                        plan,
                        scope_hash,
                    }
                } else {
                    Request::Resume { conversation, plan }
                }
            }
            WireRequest::ReviewComments { conversation, plan } => {
                Request::ReviewComments { conversation, plan }
            }
            WireRequest::Recover {
                conversation,
                plan,
                task,
                action,
                consent_files,
                freshness_token,
                message,
            } => Request::Recover(super::TaskRecoveryRequest {
                conversation,
                plan,
                task,
                action,
                consent_files,
                freshness_token,
                message,
            }),
            WireRequest::RestartOrchestrator { conversation } => {
                Request::RestartOrchestrator { conversation }
            }
            WireRequest::Cancel {
                conversation,
                scope_hash,
            } => {
                if scope_hash.is_some() {
                    Request::CancelScoped {
                        conversation,
                        scope_hash,
                    }
                } else {
                    Request::Cancel { conversation }
                }
            }
            WireRequest::Pause {
                conversation,
                paused,
            } => Request::Pause {
                conversation,
                paused,
            },
            WireRequest::Mode {
                conversation,
                autonomous,
            } => Request::Mode {
                conversation,
                autonomous,
            },
            WireRequest::Plans {
                conversation,
                offset,
                limit,
            } => Request::Plans {
                conversation,
                offset,
                limit,
            },
            WireRequest::Plan { conversation, plan } => Request::Plan { conversation, plan },
            WireRequest::Comments {
                conversation,
                plan,
                status,
            } => Request::Comments {
                conversation,
                plan,
                status,
            },
            WireRequest::Comment {
                conversation,
                plan,
                target,
                text,
            } => {
                if plan.is_some() {
                    Request::CommentScoped {
                        conversation,
                        plan,
                        target,
                        text,
                    }
                } else {
                    Request::Comment {
                        conversation,
                        target,
                        text,
                    }
                }
            }
            WireRequest::CommentStatus {
                conversation,
                id,
                status,
            } => Request::CommentStatus {
                conversation,
                id,
                status,
            },
            WireRequest::Diff {
                conversation,
                plan,
                id,
            } => Request::Diff {
                conversation,
                plan,
                id,
            },
            WireRequest::Review {
                conversation,
                plan,
                id,
                scope_hash,
            } => {
                if scope_hash.is_some() {
                    Request::ReviewScoped {
                        conversation,
                        plan,
                        id,
                        scope_hash,
                    }
                } else {
                    Request::Review {
                        conversation,
                        plan,
                        id,
                    }
                }
            }
            WireRequest::TaskChecks {
                conversation,
                plan,
                id,
            } => Request::TaskChecks {
                conversation,
                plan,
                id,
            },
            WireRequest::Task {
                conversation,
                plan,
                id,
            } => {
                if conversation.is_some() {
                    Request::TaskScoped {
                        conversation,
                        plan,
                        id,
                    }
                } else {
                    Request::Task { plan, id }
                }
            }
            WireRequest::Events {
                conversation,
                after,
                wait_ms,
                limit,
            } => {
                if limit.is_some() {
                    Request::EventsPaged {
                        conversation,
                        after,
                        wait_ms,
                        limit,
                    }
                } else {
                    Request::Events {
                        conversation,
                        after,
                        wait_ms,
                    }
                }
            }
            WireRequest::Sessions { conversation } => Request::Sessions { conversation },
            WireRequest::History {
                conversation,
                after,
                limit,
            } => Request::History {
                conversation,
                after,
                limit,
            },
            WireRequest::Mail { conversation } => Request::Mail { conversation },
            WireRequest::TrustStatus => Request::TrustStatus,
            WireRequest::Trust { trusted } => Request::Trust { trusted },
        }
    }
}

impl Serialize for Request {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        WireRequest::from(self.clone()).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Request {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        WireRequest::deserialize(deserializer).map(Request::from)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Response {
    data: Option<Value>,
    error: Option<String>,
}

/// Longest a single `events` call waits for something new.
const LONGEST_WAIT: Duration = Duration::from_secs(20);
/// Events per reply, and their combined size, so replies stay small.
const EVENTS_PER_REPLY: usize = 500;
const REPLY_BYTES: usize = 1024 * 1024;

/// How many of the (non-empty, ordered) events fit one reply: payload
/// bytes add up until `reply_bytes` is crossed, but the first event is
/// always taken whole — even one oversized historical event must not
/// stall the cursor, so a reply's size has no strict ceiling deducible
/// from `reply_bytes` alone; the cursor, not the size, bounds a reply.
/// Everything left over waits for the next poll, which the returned
/// `next_after`/`has_more` invite.
pub fn reply_event_count(events: &[super::store::Event], reply_bytes: usize) -> usize {
    let mut bytes = 0;
    let mut taken = 0;
    for event in events {
        bytes += event.payload.to_string().len();
        if taken > 0 && bytes > reply_bytes {
            break;
        }
        taken += 1;
    }
    taken
}

/// `<state>/core.sock`, or, when that path is too long for a Unix socket, a
/// short one named after the state directory in a private directory in /tmp.
pub fn socket(state: &Path) -> PathBuf {
    let socket = state.join("core.sock");
    if socket.as_os_str().len() < 100 {
        return socket;
    }
    // SAFETY: geteuid has no arguments or failure modes.
    let user = unsafe { libc::geteuid() };
    let hash = crate::hash(state.to_string_lossy().as_bytes());
    PathBuf::from(format!("/tmp/antislop-{user}")).join(format!("{}.sock", &hash[..16]))
}

pub fn serve(project: &Path, config_path: &Path, state: &Path) -> Result<()> {
    lifecycle::serve(project, config_path, state)
}

fn handle(core: &Arc<Core>, project: &Path, config: &Path, request: Request) -> Result<Value> {
    dispatch::handle(core, project, config, request)
}

fn modified(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |time| time.as_nanos() as u64)
}

fn build() -> &'static str {
    static BUILD: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    BUILD.get_or_init(|| {
        std::env::current_exe()
            .and_then(|exe| build_identity(&exe).map_err(std::io::Error::other))
            .unwrap_or_default()
    })
}

#[cfg(test)]
mod live_config_tests {
    use super::*;

    #[test]
    fn abrupt_exit_before_and_after_config_publish_keeps_a_complete_file() {
        const CHILD: &str = "ANTISLOP_TEST_CONFIG_EXIT_PHASE";
        if let Some(phase) = std::env::var_os(CHILD) {
            let path = PathBuf::from(std::env::var_os("ANTISLOP_TEST_CONFIG_EXIT_PATH").unwrap());
            let text = std::fs::read_to_string(&path)
                .unwrap()
                .replace("max_parallel = 4", "max_parallel = 1");
            let prepared = PreparedConfigWrite::prepare(&path, &text).unwrap();
            if phase == "published" {
                prepared.publish().unwrap();
            }
            // Exit without dropping PreparedConfigWrite, as after a process
            // interruption between preparation, publication and directory sync.
            std::process::exit(0);
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let original = include_str!("../../examples/antislop.toml");
        for phase in ["prepared", "published"] {
            std::fs::write(&path, original).unwrap();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "lean::daemon::live_config_tests::abrupt_exit_before_and_after_config_publish_keeps_a_complete_file"])
                .env(CHILD, phase)
                .env("ANTISLOP_TEST_CONFIG_EXIT_PATH", &path)
                .stdout(std::process::Stdio::null())
                .status().unwrap();
            assert!(status.success());
            let expected = if phase == "prepared" {
                original.to_owned()
            } else {
                original.replace("max_parallel = 4", "max_parallel = 1")
            };
            assert_eq!(std::fs::read_to_string(&path).unwrap(), expected);
            Config::load(&path).unwrap();
            atomic_config_write(&path, original).unwrap();
            assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        }
    }

    #[test]
    fn configuration_requests_validate_before_writing_and_record_the_update() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let original = include_str!("../../examples/antislop.toml");
        std::fs::write(&path, original).unwrap();
        let core = Core::open(
            &dir.path().join(".antislop"),
            Config::from_toml(original).unwrap(),
        )
        .unwrap();
        let old = core.config_snapshot();
        let call = |request| handle(&core, dir.path(), &path, request);
        assert!(
            call(Request::SetConfig {
                toml: "bad = [".into(),
                expected_revision: crate::hash(original),
            })
            .is_err()
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        assert_eq!(*core.config_snapshot(), *old);
        let updated = original.replace("max_parallel = 4", "max_parallel = 1");
        call(Request::SetConfig {
            toml: updated.clone(),
            expected_revision: crate::hash(original),
        })
        .unwrap();
        assert_eq!(core.config_snapshot().policy.max_parallel, 1);
        assert_eq!(old.policy.max_parallel, 4);
        let stale = call(Request::SetConfig {
            toml: original.into(),
            expected_revision: crate::hash(original),
        })
        .unwrap_err();
        assert!(stale.to_string().contains("configuration changed"));
        assert_eq!(core.config_snapshot().policy.max_parallel, 1);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), updated);
        assert_eq!(call(Request::GetConfig).unwrap()["toml"], updated);
        assert_eq!(
            call(Request::Identity).unwrap()["config_modified"],
            modified(&path)
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), updated);
        let conversation = core.start(dir.path()).unwrap();
        assert!(
            core.store
                .events_since(&conversation.id, 0, 100)
                .unwrap()
                .iter()
                .any(|event| event.kind == "config_updated")
        );
        let db = rusqlite::Connection::open(core.state.join("core.db")).unwrap();
        db.execute_batch("CREATE TRIGGER reject_config_event BEFORE INSERT ON events WHEN NEW.kind='config_updated' BEGIN SELECT RAISE(FAIL,'injected log failure'); END;").unwrap();
        let failure = call(Request::SetConfig {
            toml: original.into(),
            expected_revision: crate::hash(&updated),
        })
        .unwrap_err();
        assert!(failure.to_string().contains("injected log failure"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), updated);
        assert_eq!(core.config_snapshot().policy.max_parallel, 1);
        db.execute_batch("DROP TRIGGER reject_config_event;
            CREATE TABLE deferred_failure(id TEXT REFERENCES conversations(id) DEFERRABLE INITIALLY DEFERRED);
            CREATE TRIGGER fail_config_commit AFTER INSERT ON events WHEN NEW.kind='config_updated'
            BEGIN INSERT INTO deferred_failure VALUES('missing'); END;").unwrap();
        let applied = call(Request::SetConfig {
            toml: original.into(),
            expected_revision: crate::hash(&updated),
        })
        .unwrap();
        assert_eq!(applied["status"], "applied_with_warnings");
        assert_eq!(applied["warnings"][0]["stage"], "event_commit");
        assert!(
            applied["warnings"][0]["message"]
                .as_str()
                .unwrap()
                .contains("FOREIGN KEY")
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        assert_eq!(core.config_snapshot().policy.max_parallel, 4);
    }

    #[test]
    fn orphan_cleanup_preserves_active_writes_other_targets_and_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("config.toml");
        let other = dir.path().join("other.toml");
        std::fs::write(&target, "original").unwrap();
        std::fs::write(&other, "other").unwrap();
        let mut active = PreparedConfigWrite::prepare(&target, "active").unwrap();
        let other_active = PreparedConfigWrite::prepare(&other, "other active").unwrap();
        let prefix = config_temporary_prefix(&target.canonicalize().unwrap());
        let abandoned = dir.path().join(format!("{prefix}abandoned.tmp"));
        let symlink = dir.path().join(format!("{prefix}symlink.tmp"));
        std::fs::write(&abandoned, "abandoned").unwrap();
        std::os::unix::fs::symlink(&target, &symlink).unwrap();
        assert_eq!(cleanup_config_temporaries(&target).unwrap(), 1);
        assert!(!abandoned.exists());
        assert!(active.temporary.exists());
        assert!(other_active.temporary.exists());
        assert!(
            std::fs::symlink_metadata(&symlink)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "original");
        // Simulate a crashed writer: its descriptor closes without Drop cleanup.
        let temporary = active.temporary.clone();
        let lease = std::mem::replace(&mut active.lease, std::fs::File::open(&target).unwrap());
        drop(lease);
        assert_eq!(cleanup_config_temporaries(&target).unwrap(), 1);
        assert!(!temporary.exists());
    }

    #[test]
    fn preparing_config_does_not_publish_and_drop_removes_the_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("config.toml");
        std::fs::write(&target, "old").unwrap();
        let prepared = PreparedConfigWrite::prepare(&target, "new").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "old");
        assert_eq!(std::fs::read_to_string(&prepared.temporary).unwrap(), "new");
        let temporary = prepared.temporary.clone();
        drop(prepared);
        assert!(!temporary.exists());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "old");
    }

    #[test]
    fn atomic_write_preserves_symlink_and_rejects_missing_target() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.toml");
        let link = dir.path().join("link.toml");
        std::fs::write(&target, "old").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        atomic_config_write(&link, "new").unwrap();
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
        std::fs::remove_file(&target).unwrap();
        assert!(atomic_config_write(&link, "again").is_err());
        assert!(!target.exists());
    }

    #[test]
    fn failed_atomic_rename_preserves_destination_and_cleans_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("config.toml");
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(destination.join("original"), "unchanged").unwrap();
        assert!(atomic_config_write(&destination, "replacement").is_err());
        assert_eq!(
            std::fs::read_to_string(destination.join("original")).unwrap(),
            "unchanged"
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}

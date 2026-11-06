//! Daemon-owned interactive agent sessions. Every agent is the provider's own
//! CLI running in a terminal the daemon owns; people attach to that terminal
//! and use the CLI as usual. The runtime types each turn's prompt into it and
//! hears the answer back through one channel only: the result tools of the
//! session's MCP gateway (`mcp.rs`).
use super::terminal::{self, State};
use super::{Core, agent, launch, mcp, store};
use crate::{Error, Result, require};
use rusqlite::OptionalExtension;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub(crate) mod approvals;
/// Makes a headless-style test script answer as an interactive agent CLI too
/// (see `tests/support/fixture_cli.rs`).
mod registry;
mod stops;
pub(crate) use stops::StopCause;
mod session;

#[cfg(test)]
pub(crate) mod fixture {
    include!("../../tests/support/fixture_cli.rs");
}

pub(crate) const EXECUTION_FAILED: &str = "native backend execution failed: ";
/// A session nobody uses or looks at is released; it resumes on demand.
const IDLE: Duration = Duration::from_secs(10 * 60);
/// How long the CLI must stay silent before a prompt is typed, once it has
/// connected to the gateway: it may still be drawing its interface.
const SETTLE: Duration = Duration::from_millis(400);
/// UI animation must not block a CLI whose MCP tools are already available.
const SETTLE_LIMIT: Duration = Duration::from_secs(2);
const SCREEN_LINES: usize = 12;
/// A dialog can be taller than the lines shown in a failure.
const APPROVAL_LINES: usize = 40;

pub(super) fn invalid(error: impl std::fmt::Display) -> Error {
    Error::Invalid(error.to_string())
}

#[derive(Default)]
pub struct Registry {
    sessions: Mutex<HashMap<(String, String), Arc<Session>>>,
}

pub struct Session {
    pub(crate) spec: agent::Spec,
    signature: String,
    _settings: Option<tempfile::NamedTempFile>,
    gateway: mcp::Gateway,
    pub(crate) terminal: Mutex<terminal::Session>,
    screen: Arc<Mutex<State>>,
    /// The CLI's own session identifier, once known.
    identity: Mutex<Option<String>>,
    started_ms: i64,
    stop: AtomicBool,
    executing: AtomicBool,
    primed: AtomicBool,
    /// The CLI died or never connected before any prompt was typed into it,
    /// so the turn did nothing and can be started again without loss.
    startup_failed: AtomicBool,
    touched: Mutex<Instant>,
    approver: Mutex<Option<Arc<approvals::Approver>>>,
}

/// What is typed into the CLI: the prompt and how to hand the answer back. The
/// shape of the result for the agent's role is the tool's own schema.
fn turn_text(turn: &str, prompt: &str) -> String {
    format!(
        "{prompt}\n\n---\nWhen this turn is complete, end it as your last action with the \
         result tool of the MCP server `{}` that this prompt names (`{}` when it names none), \
         with turn = \"{turn}\"; stop only after receiving recorded=true. If rejected, correct \
         the reported error and call it again. Task workers/reviewers can call current_turn with an empty object after a turn-id rejection and must copy its active id exactly. Never finish only in terminal prose. A worker blocked by unavailable input must first finish any permitted independent work; only when none remains, call report_blocker with unblocked_work_remaining=false and the resume condition instead of claiming completion, repeating answered questions or waiting for a timeout.",
        mcp::SERVER,
        mcp::TOOL
    )
}

/// The task and round a turn belonged to, for a task's workers and reviewers.
fn turn_scope(core: &Core, turn: &str) -> Result<Option<(String, u32)>> {
    let row = core
        .store
        .db()
        .query_row(
            "SELECT task_id,round FROM turn_scopes WHERE turn_id=?1",
            [turn],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<u32>>(1)?,
                ))
            },
        )
        .optional()?;
    Ok(row.and_then(|(task, round)| task.zip(round)))
}

/// After a crash: ends CLIs an earlier daemon left running.
pub(crate) fn recover(store: &store::Store, state: &Path) -> Result<()> {
    store.db().execute_batch(
        "CREATE TABLE IF NOT EXISTS native_processes(
        pid INTEGER PRIMARY KEY,started_ms INTEGER NOT NULL,provider TEXT NOT NULL)",
    )?;
    let rows = {
        let db = store.db();
        let mut query = db.prepare("SELECT pid,started_ms,provider FROM native_processes")?;
        query
            .query_map([], |row| {
                Ok((
                    row.get::<_, u32>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?
    };
    for (pid, started, provider) in rows {
        agent::reap(pid, started, &provider);
        store
            .db()
            .execute("DELETE FROM native_processes WHERE pid=?1", [pid])?;
    }
    super::process_tree::kill_leftovers(state);
    Ok(())
}

#[cfg(test)]
#[path = "native/tests.rs"]
mod tests;

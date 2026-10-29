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

/// Makes a headless-style test script answer as an interactive agent CLI too
/// (see `tests/support/fixture_cli.rs`).
#[cfg(test)]
pub(crate) mod fixture {
    include!("../../tests/support/fixture_cli.rs");
}

pub(crate) const EXECUTION_FAILED: &str =
    "native backend execution failed (automatic retry disabled): ";
/// A turn that has not ended after this long is reported as failed;
/// `ANTISLOP_TURN_TIMEOUT_SECS` changes it.
fn turn_timeout() -> Duration {
    std::env::var("ANTISLOP_TURN_TIMEOUT_SECS")
        .ok()
        .and_then(|seconds| seconds.parse().ok())
        .map_or(Duration::from_secs(30 * 60), Duration::from_secs)
}
/// A session nobody uses or looks at is released; it resumes on demand.
const IDLE: Duration = Duration::from_secs(10 * 60);
/// How long the CLI must stay silent before a prompt is typed, once it has
/// connected to the gateway: it may still be drawing its interface.
const SETTLE: Duration = Duration::from_millis(400);
/// UI animation must not block a CLI whose MCP tools are already available.
const SETTLE_LIMIT: Duration = Duration::from_secs(2);
/// How long a starting CLI may take to connect to the gateway.
const READY_LIMIT: Duration = Duration::from_secs(60);
const SCREEN_LINES: usize = 12;

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
}

impl Registry {
    pub(crate) fn reap_idle(&self) -> Result<()> {
        let retired = {
            let mut sessions = self.sessions.lock().map_err(invalid)?;
            let keys = sessions
                .iter()
                .filter(|(_, session)| {
                    Arc::strong_count(session) == 1
                        && !session.executing.load(Ordering::SeqCst)
                        && session.touched.lock().is_ok_and(|at| at.elapsed() > IDLE)
                        && session
                            .terminal
                            .lock()
                            .is_ok_and(|mut terminal| !terminal.attached())
                })
                .map(|(key, _)| key.clone())
                .collect::<Vec<_>>();
            keys.into_iter()
                .filter_map(|key| sessions.remove(&key))
                .collect::<Vec<_>>()
        };
        drop(retired);
        Ok(())
    }

    /// Starts the role's CLI again from its recorded run, resuming the
    /// provider's own session. This is how an idle or earlier session is
    /// looked at: the provider's history is the only history. `role#rN` names
    /// the session of round N, which is resumed read-only.
    pub(crate) fn restore(
        &self,
        core: &Core,
        conversation: &str,
        role_key: &str,
    ) -> Result<Arc<Session>> {
        let (role, round) = match role_key.split_once("#r") {
            Some((role, round)) => (
                role,
                Some(
                    round
                        .parse::<u32>()
                        .map_err(|_| invalid("invalid round in the session name"))?,
                ),
            ),
            None => (role_key, None),
        };
        let turn_id: String = match round {
            Some(round) => core
                .store
                .db()
                .query_row(
                    "SELECT t.id FROM turns t JOIN turn_scopes s ON s.turn_id=t.id
                     WHERE t.conversation_id=?1 AND t.role=?2 AND s.round=?3
                       AND t.session_id IS NOT NULL
                     ORDER BY t.started_ms DESC,t.rowid DESC LIMIT 1",
                    rusqlite::params![conversation, role, round],
                    |row| row.get(0),
                )
                .optional()?
                .ok_or_else(|| invalid("no session was recorded for that round"))?,
            None => {
                core.store
                    .last_turn(conversation, role)?
                    .ok_or_else(|| invalid("native session has not started"))?
                    .id
            }
        };
        let context = core
            .store
            .run_context(conversation, &turn_id)?
            .ok_or_else(|| invalid("this run recorded no working directory to resume in"))?;
        require(
            context.cwd.is_dir(),
            "the session's working directory no longer exists; start new work",
        )?;
        let config = core.config_snapshot();
        let profile = config
            .profiles
            .get(&context.profile)
            .ok_or_else(|| invalid("the session's profile no longer exists"))?;
        require(
            agent::name(profile.provider) == context.provider,
            "the session's provider changed; start a new turn before attaching",
        )?;
        let provider = config
            .providers
            .get(&profile.provider)
            .ok_or_else(|| invalid("the session's provider is not configured"))?;
        let (binary, args) = crate::config::resolve_command(provider, profile)?;
        let spec = agent::Spec {
            provider: profile.provider,
            binary,
            args,
            model: profile.model.clone(),
            effort: profile
                .provider
                .native_effort(profile.effort)
                .map(str::to_owned),
            cwd: context.cwd,
            add_dirs: context.add_dirs,
            access: match (context.access, round) {
                (store::RunAccess::Write, None) => agent::Access::Write,
                _ => agent::Access::ReadOnly,
            },
            resume: context.provider_thread_id.or(context.provider_session_id),
        };
        require(
            spec.resume.is_some(),
            "the provider has not reported this session's identifier yet",
        )?;
        let execution_signature =
            super::rounds::spec_signature(&context.profile, profile.effort, &spec)?;
        if round.is_none() && super::task_role(role).is_some() {
            let recorded: Option<String> = core
                .store
                .db()
                .query_row(
                    "SELECT json_extract(payload,'$.execution_signature') FROM events
                 WHERE turn_id=?1 AND kind='turn_started' ORDER BY seq DESC LIMIT 1",
                    [&turn_id],
                    |row| row.get(0),
                )
                .optional()?
                .flatten();
            require(
                recorded.as_deref() == Some(execution_signature.as_str()),
                "the execution profile changed; start a fresh task turn before attaching",
            )?;
        }
        let scope = turn_scope(core, &turn_id)?
            .map(|(task, round)| {
                format!(
                    "{:?}:{}:{}",
                    super::SessionPolicy::TaskRound { task, round },
                    context.profile,
                    execution_signature
                )
            })
            .unwrap_or_default();
        self.prepare_scoped(core, conversation, role_key, &spec, &scope)
    }

    pub(crate) fn shutdown(&self) -> Result<()> {
        let mut sessions = self.sessions.lock().map_err(invalid)?;
        for session in sessions.values() {
            session.stop.store(true, Ordering::SeqCst);
        }
        sessions.clear();
        Ok(())
    }

    /// The role's running session, or a new one when its specification
    /// (apart from resume) differs or its CLI has ended.
    pub(crate) fn prepare_scoped(
        &self,
        core: &Core,
        conversation: &str,
        role: &str,
        spec: &agent::Spec,
        scope: &str,
    ) -> Result<Arc<Session>> {
        // Resume changes after the first turn, but does not replace the CLI.
        let mut stable = spec.clone();
        stable.resume = None;
        let signature = if scope.is_empty() {
            format!("{role}:{stable:?}")
        } else {
            format!("{role}:{scope}:{stable:?}")
        };
        let key = (conversation.to_owned(), role.to_owned());
        let mut sessions = self.sessions.lock().map_err(invalid)?;
        // The live CLI continues its own conversation; a turn that wants a
        // different session (or a fresh one after a restart) needs another CLI.
        if let Some(session) = sessions.get(&key)
            && session.signature == signature
            && !session.stop.load(Ordering::SeqCst)
            && (spec.resume == session.identity() || !session.primed.load(Ordering::SeqCst))
            && session.check_alive().is_ok()
        {
            return Ok(session.clone());
        }
        sessions.remove(&key);
        let session = Arc::new(Session::start(core, spec.clone(), signature, role)?);
        sessions.insert(key, session.clone());
        Ok(session)
    }

    pub(crate) fn get(&self, conversation: &str, role: &str) -> Result<Option<Arc<Session>>> {
        Ok(self
            .sessions
            .lock()
            .map_err(invalid)?
            .get(&(conversation.into(), role.into()))
            .cloned())
    }

    /// Identifiers other live sessions already own.
    fn claimed(&self) -> Vec<String> {
        self.sessions.lock().map_or_else(
            |_| vec![],
            |sessions| {
                sessions
                    .values()
                    .filter_map(|session| session.identity())
                    .collect()
            },
        )
    }
}

impl Session {
    fn start(core: &Core, spec: agent::Spec, signature: String, role: &str) -> Result<Self> {
        let gateway = mcp::Gateway::start()?;
        gateway.set_role(role)?;
        let launch = launch::command(&spec, &gateway)?;
        let weak = core.weak.get().expect("core weak").clone();
        let terminal =
            terminal::Session::spawn(launch.command, &weak, agent::name(spec.provider), 120, 40)?;
        let screen = terminal.shared();
        Ok(Self {
            spec,
            signature,
            _settings: launch.settings,
            gateway,
            terminal: Mutex::new(terminal),
            screen,
            identity: Mutex::new(launch.session),
            started_ms: store::now_ms(),
            stop: AtomicBool::new(false),
            executing: AtomicBool::new(false),
            primed: AtomicBool::new(false),
            startup_failed: AtomicBool::new(false),
            touched: Mutex::new(Instant::now()),
        })
    }

    pub(crate) fn identity(&self) -> Option<String> {
        self.identity.lock().ok()?.clone()
    }

    pub(crate) fn touch(&self) {
        if let Ok(mut touched) = self.touched.lock() {
            *touched = Instant::now();
        }
    }

    /// Fails with the CLI's exit status and last screen lines once it ended.
    pub(crate) fn check_alive(&self) -> Result<()> {
        if self.stop.load(Ordering::SeqCst) {
            return Err(invalid("the agent session was stopped"));
        }
        let mut terminal = self.terminal.lock().map_err(invalid)?;
        if let Some(status) = terminal.exited() {
            return Err(invalid(format!(
                "{} exited ({status}): {}",
                agent::name(self.spec.provider),
                terminal::tail(&self.screen, SCREEN_LINES)
            )));
        }
        Ok(())
    }

    /// Require MCP tool discovery, then briefly let startup drawing settle.
    /// Output silence is a courtesy, not a readiness signal: interactive CLIs
    /// may redraw a spinner continuously, including between completed turns.
    fn wait_ready(&self, cancel: &AtomicBool) -> Result<()> {
        let limit = Instant::now() + READY_LIMIT;
        while !self.gateway.connected() {
            if cancel.load(Ordering::SeqCst) {
                return Err(invalid(mcp::CANCELLED));
            }
            self.check_alive()?;
            if Instant::now() >= limit {
                return Err(invalid(format!(
                    "{} did not connect to the antislop MCP gateway within {} s; the terminal shows:\n{}",
                    agent::name(self.spec.provider),
                    READY_LIMIT.as_secs(),
                    terminal::tail(&self.screen, SCREEN_LINES)
                )));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let settle_until = Instant::now() + SETTLE_LIMIT;
        loop {
            if cancel.load(Ordering::SeqCst) {
                return Err(invalid(mcp::CANCELLED));
            }
            self.check_alive()?;
            if terminal::quiet_for(&self.screen) >= SETTLE || Instant::now() >= settle_until {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Runs one turn: types the prompt into the CLI and waits for it to end
    /// the turn through the gateway.
    pub(crate) fn run(
        &self,
        registry: &Registry,
        turn: &str,
        prompt: &str,
        cancel: &AtomicBool,
    ) -> Result<agent::Outcome> {
        self.check_alive()?;
        require(
            self.executing
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok(),
            "the agent session already has an executing turn",
        )?;
        let result = self.execute(registry, turn, prompt, cancel);
        self.executing.store(false, Ordering::SeqCst);
        self.touch();
        // No failover after the CLI accepted a prompt: a failed session is
        // dropped so the next turn starts a clean one that resumes it.
        match result {
            Err(error) if error.to_string() != mcp::CANCELLED => {
                self.stop.store(true, Ordering::SeqCst);
                Err(invalid(format!("{EXECUTION_FAILED}{error}")))
            }
            other => other,
        }
    }

    /// Whether the last run failed before the CLI took its prompt.
    pub(crate) fn failed_before_start(&self) -> bool {
        self.startup_failed.load(Ordering::SeqCst)
    }

    pub(crate) fn set_tool_handler(&self, handler: Arc<mcp::Handler>) -> Result<()> {
        self.gateway.set_handler(handler)
    }

    fn execute(
        &self,
        registry: &Registry,
        turn: &str,
        prompt: &str,
        cancel: &AtomicBool,
    ) -> Result<agent::Outcome> {
        self.wait_ready(cancel).inspect_err(|error| {
            if error.to_string() != mcp::CANCELLED {
                self.startup_failed.store(true, Ordering::SeqCst);
            }
        })?;
        self.gateway.begin(turn)?;
        terminal::Session::submit(&self.screen, &turn_text(turn, prompt))?;
        self.primed.store(true, Ordering::SeqCst);
        let finish = self
            .gateway
            .wait(turn, cancel, turn_timeout(), || self.check_alive())
            .inspect_err(|error| {
                if error.to_string() == mcp::CANCELLED {
                    let _ = terminal::Session::interrupt(&self.screen);
                }
            })
            .map_err(|error| {
                if ["no accepted result", "result rejected"]
                    .iter()
                    .any(|cause| error.to_string().contains(cause))
                {
                    invalid(format!(
                        "{error}; the terminal shows:\n{}",
                        terminal::tail(&self.screen, SCREEN_LINES)
                    ))
                } else {
                    error
                }
            })?;
        self.learn_identity(registry);
        Ok(agent::Outcome {
            session: self.identity(),
            text: finish.text,
        })
    }

    /// Records the CLI's session identifier once it has shown one. Nothing
    /// is held while the others' identifiers are read.
    fn learn_identity(&self, registry: &Registry) {
        if self.identity().is_some() {
            return;
        }
        let found = self.gateway.session().or_else(|| {
            (self.spec.provider == crate::config::ProviderKind::Opencode)
                .then(|| self.opencode_session(&registry.claimed()))
                .flatten()
        });
        if let Some(id) = found
            && let Ok(mut identity) = self.identity.lock()
        {
            identity.get_or_insert(id);
        }
    }

    /// OpenCode does not announce its session. The one created in this
    /// directory since the process started, and not owned by another live
    /// session, is ours; with several candidates none is guessed.
    fn opencode_session(&self, claimed: &[String]) -> Option<String> {
        let output = std::process::Command::new(&self.spec.binary)
            .args(&self.spec.args)
            .args(["session", "list", "--format", "json", "-n", "40"])
            .current_dir(&self.spec.cwd)
            .output()
            .ok()?;
        let sessions: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
        let cwd = self.spec.cwd.canonicalize().ok()?;
        let mut candidates = sessions.as_array()?.iter().filter(|session| {
            session["created"]
                .as_i64()
                .is_some_and(|at| at >= self.started_ms - 1_000)
                && session["directory"].as_str().is_some_and(|directory| {
                    Path::new(directory).canonicalize().ok().as_deref() == Some(&cwd)
                })
                && session["id"]
                    .as_str()
                    .is_some_and(|id| !claimed.iter().any(|known| known == id))
        });
        let first = candidates.next()?;
        candidates
            .next()
            .is_none()
            .then(|| first["id"].as_str().map(str::to_owned))
            .flatten()
    }
}

/// What is typed into the CLI: the prompt and how to hand the answer back. The
/// shape of the result for the agent's role is the tool's own schema.
fn turn_text(turn: &str, prompt: &str) -> String {
    format!(
        "{prompt}\n\n---\nWhen this turn is complete, end it as your last action with the \
         result tool of the MCP server `{}` that this prompt names (`{}` when it names none), \
         with turn = \"{turn}\"; stop only after receiving recorded=true. If rejected, correct \
         the reported error and call it again.",
        mcp::SERVER,
        mcp::TOOL
    )
}

/// The task and round a turn belonged to, for a task's workers and verifiers.
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
pub(crate) fn recover(store: &store::Store, _state: &Path) -> Result<()> {
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
    Ok(())
}

#[cfg(test)]
#[path = "native/tests.rs"]
mod tests;

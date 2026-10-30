// The one channel back from an interactive agent: a loopback MCP server
// with role-scoped tools. The daemon types the prompt into the
// agent's terminal; the agent ends the turn by calling the tool with the
// turn's id and its final answer. Nothing is read from the terminal screen.
use super::allowed_tools;
use crate::lean::native::invalid;
use crate::{Error, Result};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
mod listener;
mod wait;
use wait::take_finish;

pub(crate) const SERVER: &str = "antislop";
pub(crate) const TOOL: &str = "finish_turn";
/// Ends a planning turn with a task graph checked within the turn.
pub(crate) const SUBMIT_PLAN: &str = "submit_plan";
/// Asks the orchestrator a question and waits briefly for the answer.
pub(crate) const ASK: &str = "ask";
/// Ends a task turn until the question asked in it is answered.
pub(crate) const WAIT: &str = "wait_for_answer";
/// Asks the user a question; ends the orchestrator's turn.
pub(crate) const ASK_USER: &str = "ask_user";
/// Ends an interview turn with the requirements summary.
pub(crate) const SUBMIT_REQUIREMENTS: &str = "submit_requirements";
/// Ends a review turn with its findings; the runtime derives the verdict.
pub(crate) const REVIEW: &str = "review";
/// The tools that end a requirements analyst's turn with its result; the
/// runtime derives source ranges and evidence identities.
pub(crate) const LEDGER_RESULTS: [&str; 3] =
    ["submit_ledger", "submit_audit", "submit_audit_review"];
/// The tools that end a task turn with its result.
pub(crate) const TASK_RESULTS: [&str; 3] = ["complete", "select", REVIEW];
pub(crate) const TOOLS: &[&str] = &[
    TOOL,
    SUBMIT_PLAN,
    ASK_USER,
    SUBMIT_REQUIREMENTS,
    "complete",
    "select",
    REVIEW,
    ASK,
    WAIT,
    "answer_agent",
    "defer_to_user",
    "retry_task",
    "revise_plan",
    "request_replan",
    "submit_ledger",
    "submit_audit",
    "submit_audit_review",
];

/// Whether a successful call of `name` ends the turn.
pub(super) fn is_terminal(name: &str) -> bool {
    matches!(
        name,
        TOOL | SUBMIT_PLAN | WAIT | ASK_USER | SUBMIT_REQUIREMENTS
    ) || TASK_RESULTS.contains(&name)
        || LEDGER_RESULTS.contains(&name)
}

/// The typed result a terminal call records, where the arguments alone
/// determine it; the handler supplies the rest.
pub(super) fn terminal_result(
    name: &str,
    args: &Value,
) -> Result<super::super::turn_result::TurnResult> {
    if name == WAIT || LEDGER_RESULTS.contains(&name) {
        Err(invalid(format!("{name} needs the runtime's handler")))
    } else {
        super::super::turn_result::tool_result(name, args)
    }
}

/// What a terminal call is recognised by when the agent repeats it.
pub(super) fn receipt_payload(name: &str, args: &Value) -> Value {
    let mut payload = args.clone();
    if let Some(fields) = payload.as_object_mut() {
        fields.remove("turn");
    }
    json!({"tool": name, "args": payload})
}
pub(crate) type Handler = dyn Fn(&str, &Value) -> Result<Value> + Send + Sync;
/// The error of a turn the runtime cancelled while it waited.
pub(crate) const CANCELLED: &str = "agent turn cancelled";
/// Largest request body accepted; an answer is text, not a repository.
pub(super) const BODY: u64 = 64 * 1024 * 1024;

/// What the agent reported when it ended a turn.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Finish {
    pub text: String,
}

#[derive(Default)]
pub(super) struct State {
    /// The turn the agent is working on; calls for any other turn are refused.
    pub(super) expected: Option<String>,
    pub(super) finished: Option<Finish>,
    /// The CLI's own session identifier, where it announces one (Codex).
    pub(super) session: Option<String>,
    /// The CLI asked for the tool list: it is running and can use the tool.
    pub(super) listed: bool,
    pub(super) handler: Option<Arc<Handler>>,
    pub(super) role: String,
    /// The turn a terminal call ended and that call, so a repeat is
    /// acknowledged instead of refused.
    pub(super) receipt: Option<(String, Value)>,
    pub(super) waiting: Option<Instant>,
    pub(super) waited: Duration,
    pub(super) rejected_finish: Option<(Instant, String)>,
}

pub(super) struct Shared {
    pub(super) token: String,
    pub(super) state: Mutex<State>,
    pub(super) changed: Condvar,
}

pub(crate) struct Gateway {
    pub(super) shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
    address: String,
}

impl Gateway {
    pub(crate) fn start() -> Result<Self> {
        let server = tiny_http::Server::http("127.0.0.1:0").map_err(invalid)?;
        let address = server
            .server_addr()
            .to_ip()
            .ok_or_else(|| invalid("MCP gateway has no TCP address"))?;
        let shared = Arc::new(Shared {
            token: crate::id(),
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
        });
        let stop = Arc::new(AtomicBool::new(false));
        listener::spawn_listener(server, shared.clone(), stop.clone());
        Ok(Self {
            shared,
            stop,
            address: format!("http://{address}/mcp"),
        })
    }

    pub(crate) fn url(&self) -> &str {
        &self.address
    }

    pub(crate) fn token(&self) -> &str {
        &self.shared.token
    }

    /// Opens `turn`; an answer to an earlier turn can no longer be accepted.
    pub(crate) fn begin(&self, turn: &str) -> Result<()> {
        let mut state = self.shared.state.lock().map_err(invalid)?;
        state.expected = Some(turn.into());
        state.finished = None;
        state.waited = Duration::ZERO;
        state.waiting = None;
        state.rejected_finish = None;
        Ok(())
    }

    /// Whether the CLI has connected and discovered the tool; every supported
    /// CLI does so while starting, before it takes a prompt.
    pub(crate) fn set_role(&self, role: &str) -> Result<()> {
        self.shared.state.lock().map_err(invalid)?.role = role.into();
        Ok(())
    }

    pub(crate) fn tool_names(&self) -> Vec<&'static str> {
        self.shared
            .state
            .lock()
            .map_or_else(|_| vec![TOOL], |s| allowed_tools(&s.role))
    }

    pub(crate) fn set_handler(&self, handler: Arc<Handler>) -> Result<()> {
        self.shared.state.lock().map_err(invalid)?.handler = Some(handler);
        Ok(())
    }

    pub(crate) fn connected(&self) -> bool {
        self.shared.state.lock().is_ok_and(|state| state.listed)
    }

    /// The session identifier the CLI announced, if any.
    pub(crate) fn session(&self) -> Option<String> {
        self.shared.state.lock().ok()?.session.clone()
    }

    /// Whether the agent is blocked in `wait_for_answer`: silence then is the
    /// orchestrator's, not the agent's.
    pub(crate) fn awaiting_answer(&self) -> bool {
        self.shared
            .state
            .lock()
            .is_ok_and(|state| state.waiting.is_some())
    }

    /// Waits for the agent to end `turn`. `alive` reports a dead agent as an
    /// error so a crashed CLI does not wait for the full timeout.
    /// Waits for the agent to end `turn`, honoring cancellation and liveness.
    pub(crate) fn wait(
        &self,
        turn: &str,
        cancel: &AtomicBool,
        timeout: Duration,
        alive: impl Fn() -> Result<()>,
    ) -> Result<Finish> {
        wait_loop(&self.shared, turn, cancel, timeout, alive)
    }
}

fn wait_loop(
    shared: &Shared,
    turn: &str,
    cancel: &AtomicBool,
    timeout: Duration,
    alive: impl Fn() -> Result<()>,
) -> Result<Finish> {
    let started = Instant::now();
    let mut state = shared.state.lock().map_err(invalid)?;
    loop {
        if let Some(finish) = wait::poll_wait(&mut state, started, turn, cancel, timeout)? {
            return Ok(finish);
        }
        drop(state);
        if let Err(error) = alive() {
            return agent_exited(shared, error);
        }
        state = shared.state.lock().map_err(invalid)?;
        if state.finished.is_none() {
            state = shared
                .changed
                .wait_timeout(state, Duration::from_millis(100))
                .map_err(invalid)?
                .0;
        }
    }
}

fn agent_exited(shared: &Shared, error: Error) -> Result<Finish> {
    let mut state = shared.state.lock().map_err(invalid)?;
    if let Some(finish) = take_finish(&mut state) {
        return Ok(finish);
    }
    let rejection = state
        .rejected_finish
        .as_ref()
        .map_or(String::new(), |(_, reason)| {
            format!("; last rejected result: {reason}")
        });
    state.expected = None;
    Err(invalid(format!("{error}{rejection}")))
}

impl Drop for Gateway {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

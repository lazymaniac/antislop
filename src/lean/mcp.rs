//! The one channel back from an interactive agent: a loopback MCP server
//! with role-scoped tools. The daemon types the prompt into the
//! agent's terminal; the agent ends the turn by calling the tool with the
//! turn's id and its final answer. Nothing is read from the terminal screen.
mod schema;
use super::native::invalid;
use crate::{Error, Result};
use serde_json::{Value, json};
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

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
const LEDGER_RESULTS: [&str; 3] = ["submit_ledger", "submit_audit", "submit_audit_review"];
/// The tools that end a task turn with its result.
const TASK_RESULTS: [&str; 3] = ["complete", "select", REVIEW];
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
    "revise_plan",
    "request_replan",
    "submit_ledger",
    "submit_audit",
    "submit_audit_review",
];

/// Whether a successful call of `name` ends the turn.
fn is_terminal(name: &str) -> bool {
    matches!(
        name,
        TOOL | SUBMIT_PLAN | WAIT | ASK_USER | SUBMIT_REQUIREMENTS
    ) || TASK_RESULTS.contains(&name)
        || LEDGER_RESULTS.contains(&name)
}

/// The typed result a terminal call records, where the arguments alone
/// determine it; the handler supplies the rest.
fn terminal_result(name: &str, args: &Value) -> Result<super::turn_result::TurnResult> {
    if name == WAIT || LEDGER_RESULTS.contains(&name) {
        Err(invalid(format!("{name} needs the runtime's handler")))
    } else {
        super::turn_result::tool_result(name, args)
    }
}

/// What a terminal call is recognised by when the agent repeats it.
fn receipt_payload(name: &str, args: &Value) -> Value {
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
const BODY: u64 = 64 * 1024 * 1024;

/// What the agent reported when it ended a turn.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Finish {
    pub text: String,
}

#[derive(Default)]
struct State {
    /// The turn the agent is working on; calls for any other turn are refused.
    expected: Option<String>,
    finished: Option<Finish>,
    /// The CLI's own session identifier, where it announces one (Codex).
    session: Option<String>,
    /// The CLI asked for the tool list: it is running and can use the tool.
    listed: bool,
    handler: Option<Arc<Handler>>,
    role: String,
    /// The turn a terminal call ended and that call, so a repeat is
    /// acknowledged instead of refused.
    receipt: Option<(String, Value)>,
    waiting: Option<Instant>,
    waited: Duration,
    rejected_finish: Option<(Instant, String)>,
}

struct Shared {
    token: String,
    state: Mutex<State>,
    changed: Condvar,
}

pub(crate) struct Gateway {
    shared: Arc<Shared>,
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
        let (worker, halt) = (shared.clone(), stop.clone());
        std::thread::spawn(move || {
            while !halt.load(Ordering::SeqCst) {
                match server.recv_timeout(Duration::from_millis(100)) {
                    Ok(Some(request)) => {
                        let shared = worker.clone();
                        std::thread::spawn(move || {
                            let _ = serve(&shared, request);
                        });
                    }
                    Ok(None) => {}
                    Err(_) => break,
                }
            }
        });
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

    /// Waits for the agent to end `turn`. `alive` reports a dead agent as an
    /// error so a crashed CLI does not wait for the full timeout.
    pub(crate) fn wait(
        &self,
        turn: &str,
        cancel: &AtomicBool,
        timeout: Duration,
        alive: impl Fn() -> Result<()>,
    ) -> Result<Finish> {
        let started = Instant::now();
        let mut state = self.shared.state.lock().map_err(invalid)?;
        loop {
            if let Some(finish) = state.finished.take() {
                state.expected = None;
                return Ok(finish);
            }
            if cancel.load(Ordering::SeqCst) {
                state.expected = None;
                return Err(Error::Invalid(CANCELLED.into()));
            }
            let waiting = state.waited + state.waiting.map_or(Duration::ZERO, |at| at.elapsed());
            if started.elapsed().saturating_sub(waiting) >= timeout {
                state.expected = None;
                let rejection = state
                    .rejected_finish
                    .as_ref()
                    .map_or(String::new(), |(_, error)| {
                        format!("; last rejected submission: {error}")
                    });
                return Err(Error::Invalid(format!(
                    "no accepted result for turn {turn} within {} s{rejection}",
                    timeout.as_secs()
                )));
            }
            drop(state);
            if let Err(error) = alive() {
                let mut state = self.shared.state.lock().map_err(invalid)?;
                if let Some(finish) = state.finished.take() {
                    state.expected = None;
                    return Ok(finish);
                }
                let rejection = state
                    .rejected_finish
                    .as_ref()
                    .map_or(String::new(), |(_, reason)| {
                        format!("; last rejected result: {reason}")
                    });
                state.expected = None;
                return Err(invalid(format!("{error}{rejection}")));
            }
            state = self.shared.state.lock().map_err(invalid)?;
            if state.finished.is_none() {
                state = self
                    .shared
                    .changed
                    .wait_timeout(state, Duration::from_millis(100))
                    .map_err(invalid)?
                    .0;
            }
        }
    }
}

impl Drop for Gateway {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// The stdio side of the gateway, for CLIs that can only be given a stdio MCP
/// server (Agy): the CLI starts `antislop mcp-bridge`, which forwards each
/// JSON-RPC line to the gateway named by the environment it inherited.
pub fn bridge() -> Result<()> {
    let url = std::env::var("ANTISLOP_MCP_URL").map_err(|_| {
        invalid("ANTISLOP_MCP_URL is not set; mcp-bridge is started by an agent CLI")
    })?;
    let token = std::env::var("ANTISLOP_MCP_TOKEN")
        .map_err(|_| invalid("ANTISLOP_MCP_TOKEN is not set"))?;
    let conversation = std::env::var("ANTIGRAVITY_CONVERSATION_ID").ok();
    forward(
        std::io::stdin().lock(),
        std::io::stdout().lock(),
        &url,
        &token,
        conversation.as_deref(),
    )
}

/// Relays JSON-RPC lines from `input` to the gateway and its replies to
/// `output`; a notification gets no reply. The CLI's conversation, where it is
/// known, rides along on tool calls so the runtime can resume it.
fn forward(
    input: impl std::io::BufRead,
    mut output: impl std::io::Write,
    url: &str,
    token: &str,
    conversation: Option<&str>,
) -> Result<()> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    for line in input.lines() {
        let line = line.map_err(Error::Io)?;
        if line.trim().is_empty() {
            continue;
        }
        let mut message: Value = serde_json::from_str(&line)?;
        if let Some(conversation) = conversation
            && message["method"] == "tools/call"
        {
            message["params"]["_meta"]["sessionId"] = Value::String(conversation.into());
        }
        let mut response = agent
            .post(url)
            .header("authorization", format!("Bearer {token}"))
            .send_json(&message)
            .map_err(|error| invalid(format!("gateway unreachable: {error}")))?;
        let body = response.body_mut().read_to_string().map_err(invalid)?;
        if let Ok(reply) = serde_json::from_str::<Value>(&body) {
            writeln!(output, "{reply}").map_err(Error::Io)?;
            output.flush().map_err(Error::Io)?;
        }
    }
    Ok(())
}

fn respond(request: tiny_http::Request, status: u16, body: Option<Value>) -> Result<()> {
    let mut response =
        tiny_http::Response::from_string(body.map(|body| body.to_string()).unwrap_or_default())
            .with_status_code(status);
    response.add_header(
        tiny_http::Header::from_bytes("content-type", "application/json")
            .map_err(|_| invalid("invalid content-type header"))?,
    );
    request.respond(response).map_err(Error::Io)
}

fn serve(shared: &Shared, mut request: tiny_http::Request) -> Result<()> {
    let authorized = request.headers().iter().any(|header| {
        header.field.equiv("authorization")
            && header.value.as_str() == format!("Bearer {}", shared.token)
    });
    if !authorized {
        return respond(request, 401, None);
    }
    if request.method() != &tiny_http::Method::Post {
        // No server-initiated stream: clients fall back to plain POST.
        return respond(request, 405, None);
    }
    let mut text = String::new();
    request
        .as_reader()
        .take(BODY)
        .read_to_string(&mut text)
        .map_err(Error::Io)?;
    let Ok(message) = serde_json::from_str::<Value>(&text) else {
        return respond(
            request,
            400,
            Some(error(Value::Null, -32700, "invalid JSON")),
        );
    };
    let Some(id) = message.get("id").cloned() else {
        // Notifications (initialized, cancelled) need no reply.
        return respond(request, 202, None);
    };
    let method = message["method"].as_str().unwrap_or_default();
    let reply = match method {
        "initialize" => result(
            id,
            json!({
                "protocolVersion": message["params"]["protocolVersion"]
                    .as_str()
                    .unwrap_or("2025-03-26"),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": SERVER, "version": env!("CARGO_PKG_VERSION")}
            }),
        ),
        "ping" => result(id, json!({})),
        "tools/list" => {
            if let Ok(mut state) = shared.state.lock() {
                state.listed = true;
            }
            result(
                id,
                json!({"tools": tool_list(&shared.state.lock().map_err(invalid)?.role)}),
            )
        }
        "tools/call" => result(id, call(shared, &message["params"])),
        other => error(id, -32601, &format!("method not supported: {other}")),
    };
    respond(request, 200, Some(reply))
}

/// The roles that review a plan, audit delivery or review a code change:
/// findings without per-check coverage.
fn plain_reviewer(role: &str) -> bool {
    matches!(role, "plan-verifier" | "auditor") || role.starts_with("code-review:")
}

fn allowed_tools(role: &str) -> Vec<&'static str> {
    if let Some((_, verifier)) = super::task_role(role) {
        if verifier {
            vec![REVIEW, ASK, WAIT]
        } else {
            vec!["complete", "select", ASK, WAIT]
        }
    } else if matches!(role, "orchestrator" | "comment-orchestrator") {
        vec![
            TOOL,
            ASK_USER,
            SUBMIT_REQUIREMENTS,
            SUBMIT_PLAN,
            "answer_agent",
            "defer_to_user",
            "revise_plan",
            "request_replan",
        ]
    } else if role == "planner" {
        vec![SUBMIT_PLAN]
    } else if plain_reviewer(role) {
        vec![REVIEW]
    } else if role == "requirements-extraction-analyst" {
        vec!["submit_ledger"]
    } else if role == "requirements-audit-analyst" {
        vec!["submit_audit"]
    } else if role == "requirements-audit-verifier" {
        vec!["submit_audit_review"]
    } else {
        vec![TOOL]
    }
}

const RETRY: &str = "Stop only after recorded=true; if rejected, fix what the error names and call again within the remaining turn time.";

fn tool_list(role: &str) -> Vec<Value> {
    let orchestrator = matches!(role, "orchestrator" | "comment-orchestrator");
    allowed_tools(role).iter().map(|name| {
        let (description, fields, required): (String, Value, Vec<&str>) = match *name {
            "finish_turn" if orchestrator => (format!("End the turn with your answer to the user. {RETRY}"), json!({"text":{"type":"string","description":"Your answer, in Markdown"},"addressed":schema::addressed()}), vec!["turn","text"]),
            "finish_turn" => (format!("End the turn with your answer. {RETRY}"), json!({"text":{"type":"string","description":"Your answer, in Markdown"}}), vec!["turn","text"]),
            "ask_user" => (format!("Ask the user one question with 2–6 choices; this ends your turn and the answer arrives as the user's next message. {RETRY}"), schema::question(), vec!["turn","context","question","options"]),
            "submit_requirements" => (format!("Interview only: end the turn with the requirements summary for the user to approve: the goal and its acceptance criteria. {RETRY}"), json!({"markdown":{"type":"string","description":"The summary in Markdown"}}), vec!["turn","markdown"]),
            "complete" => (format!("Finish your task; this ends your turn. {RETRY}"), json!({"summary":{"type":"string","description":"A few sentences on what was delivered; tasks that depend on this one receive only this"},"report":{"type":"string","description":"The full result your verifier checks: what you did, where, and how you checked it"}}), vec!["turn","summary","report"]),
            "select" => (format!("Options tasks only: record the approach you chose among 2–4 alternatives; this ends your turn. {RETRY}"), schema::selection(), vec!["turn","alternatives","selected","rationale"]),
            "review" if plain_reviewer(role) => (format!("Record your review; this ends your turn. The runtime approves when no finding is critical, high or medium. {RETRY}"), schema::plain_review(), vec!["turn","report","findings"]),
            "review" => (format!("Record your review of the task; this ends your turn. The runtime approves the task when every check is ok or n/a and no finding is critical, high or medium. {RETRY}"), schema::review(), vec!["turn","report","checks","findings"]),
            "ask" => ("Ask the orchestrator a question about your task without ending your turn. Waits up to 25 seconds and returns the answer if one came; asking the same question again returns its state.".into(), json!({"question":{"type":"string"}}), vec!["turn","question"]),
            "wait_for_answer" => ("End your turn until the question you asked is answered; your work so far is kept and the task resumes with the answer.".into(), json!({"note":{"type":"string","description":"Where you stopped, for when you resume"}}), vec!["turn"]),
            "submit_plan" => {
                let mut fields = json!({"document":{"type":"string","description":"The plan for the user in Markdown: approach, task order and what each task delivers"},"tasks":{"type":"array","minItems":1,"items":schema::planned_task()}});
                if orchestrator {
                    fields["addressed"] = schema::addressed();
                }
                ("Submit the complete task plan; this ends your turn. The runtime checks it at once: if it is rejected, fix what the error names and submit the whole plan again within the remaining turn time. Stop only after recorded=true.".into(), fields, vec!["turn","document","tasks"])
            }
            "submit_ledger" => (format!("Submit the requirements you extracted; this ends your turn. The runtime finds each quote in the sources and checks the requirements at once. {RETRY}"), json!({"entries":{"type":"array","items":schema::ledger_entry()}}), vec!["turn","entries"]),
            "submit_audit" => (format!("Submit the audit; this ends your turn. It approves unless a finding is critical, high or medium. {RETRY}"), schema::audit(), vec!["turn","findings","evidence"]),
            "submit_audit_review" => (format!("Submit your review of the audit; this ends your turn. Assess every proposed evidence id. {RETRY}"), schema::audit_review(), vec!["turn","note","assessments"]),
            "answer_agent" => ("Answer a task agent's pending question. Answers cannot change contracts or permissions.".into(), json!({"request":{"type":"string","description":"The request id the agent's question message names"},"answer":{"type":"string"}}), vec!["turn","request","answer"]),
            "defer_to_user" => ("Hand a task agent's question to the user when only the user can decide; the task waits and you pass the user's answer on with answer_agent.".into(), json!({"request":{"type":"string","description":"The request id the agent's question message names"},"explanation":{"type":"string","description":"What the user must decide and why, for the user"}}), vec!["turn","request","explanation"]),
            "request_replan" => ("Ask the planner to redraft the whole plan when contract repairs are not enough. It is reviewed like any new plan.".into(), json!({"reason":{"type":"string","description":"What the current plan gets wrong and what the new plan must change"}}), vec!["turn","reason"]),
            _ => ("Change the running plan you were shown. Give only the tasks that change or are added, in full; the others stay as they are. The revision is reviewed independently before it applies.".into(), schema::revision(), vec!["turn","reason","tasks","retire"]),
        };
        let mut properties = fields.as_object().unwrap().clone();
        properties.insert("turn".into(),json!({"type":"string"}));
        json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false}})
    }).collect()
}

/// Validate the open lease before dispatch; never hold the gateway lock during work.
fn call(shared: &Shared, params: &Value) -> Value {
    let outcome = dispatch(shared, params);
    match outcome {
        Ok(value) => json!({"content":[{"type":"text","text":value.to_string()}]}),
        Err(error) => {
            if is_terminal(params["name"].as_str().unwrap_or_default())
                && let Ok(mut state) = shared.state.lock()
                && state.expected.as_deref() == params["arguments"]["turn"].as_str()
                && state.expected.is_some()
                && state.finished.is_none()
            {
                let at = state
                    .rejected_finish
                    .as_ref()
                    .map_or_else(Instant::now, |(at, _)| *at);
                state.rejected_finish = Some((at, error.to_string()));
                shared.changed.notify_all();
            }
            json!({"isError":true,"content":[{"type":"text","text":error.to_string()}]})
        }
    }
}

fn dispatch(shared: &Shared, params: &Value) -> Result<Value> {
    let name = params["name"].as_str().unwrap_or_default();
    crate::require(TOOLS.contains(&name), "unknown tool")?;
    let args = &params["arguments"];
    let turn = args["turn"]
        .as_str()
        .ok_or_else(|| invalid("turn: required"))?;
    let handler = {
        let mut state = shared.state.lock().map_err(invalid)?;
        crate::require(
            allowed_tools(&state.role).contains(&name),
            "tool is unavailable for this role",
        )?;
        for key in ["threadId", "sessionId"] {
            if let Some(id) = params["_meta"][key].as_str() {
                state.session.get_or_insert_with(|| id.into());
            }
        }
        if is_terminal(name)
            && let Some((recorded, result)) = &state.receipt
            && recorded == turn
        {
            // The first result ended the turn; a later, different call must
            // not read as a failure of it.
            crate::require(
                result == &receipt_payload(name, args),
                &format!(
                    "this turn already ended with your earlier {} call; that result stands and this call changed nothing. Stop now.",
                    result["tool"].as_str().unwrap_or(TOOL)
                ),
            )?;
            return Ok(json!({"recorded":true}));
        }
        crate::require(
            state.expected.as_deref() == Some(turn),
            "turn is not open; use the current prompt's turn",
        )?;
        crate::require(state.finished.is_none(), "turn is already finished")?;
        if name == ASK {
            crate::require(
                state.waiting.is_none(),
                "only one answer wait may be active",
            )?;
            state.waiting = Some(Instant::now());
        }
        state.handler.clone()
    };
    let response = match handler {
        Some(handler) => handler(name, args),
        None if is_terminal(name) => terminal_result(name, args).map(|_| json!({"recorded":true})),
        None => Err(invalid("agent tool handler is not installed")),
    };
    let mut state = shared.state.lock().map_err(invalid)?;
    if name == ASK
        && let Some(at) = state.waiting.take()
    {
        state.waited += at.elapsed();
    }
    let mut response = response?;
    crate::require(
        state.expected.as_deref() == Some(turn),
        "turn closed while the tool was executing",
    )?;
    if is_terminal(name) {
        // A handler that derived more than the arguments say (the open
        // question a wait is for) hands the result back with its receipt.
        let result = match response.as_object_mut().and_then(|r| r.remove("result")) {
            Some(result) => serde_json::from_value(result)?,
            None => terminal_result(name, args)?,
        };
        state.receipt = Some((turn.into(), receipt_payload(name, args)));
        let text = result.encode()?;
        state.finished = Some(Finish { text });
        shared.changed.notify_all();
    }
    Ok(response)
}

fn result(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn post(gateway: &Gateway, token: &str, body: Value) -> (u16, Value) {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .into();
        let mut response = match agent
            .post(gateway.url())
            .header("authorization", format!("Bearer {token}"))
            .send_json(body)
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };
        let status = response.status().as_u16();
        let text = response.body_mut().read_to_string().unwrap();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    fn finish(turn: &str, text: &str) -> Value {
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
            "name":TOOL,"arguments":{"turn":turn,"text":text}}})
    }

    #[test]
    fn the_agent_ends_exactly_the_open_turn() {
        let gateway = Gateway::start().unwrap();
        let token = gateway.token().to_owned();
        gateway.begin("t1").unwrap();
        let (_, wrong) = post(&gateway, &token, finish("t0", "stale"));
        assert_eq!(wrong["result"]["isError"], true);
        let (_, right) = post(&gateway, &token, finish("t1", "done"));
        assert_eq!(right["result"]["isError"], Value::Null);
        let (_, again) = post(&gateway, &token, finish("t1", "twice"));
        assert_eq!(again["result"]["isError"], true);
        let cancel = AtomicBool::new(false);
        let answer = gateway
            .wait("t1", &cancel, Duration::from_secs(2), || Ok(()))
            .unwrap();
        assert_eq!(answer.text, "done");
    }

    #[test]
    fn requests_without_the_session_token_are_refused() {
        let gateway = Gateway::start().unwrap();
        gateway.begin("t1").unwrap();
        let (status, _) = post(&gateway, "wrong", finish("t1", "x"));
        assert_eq!(status, 401);
        let cancel = AtomicBool::new(false);
        assert!(
            gateway
                .wait("t1", &cancel, Duration::from_millis(300), || Ok(()))
                .is_err()
        );
    }

    #[test]
    fn a_call_between_turns_is_refused() {
        let gateway = Gateway::start().unwrap();
        let token = gateway.token().to_owned();
        let (_, reply) = post(&gateway, &token, finish("t1", "late"));
        assert_eq!(reply["result"]["isError"], true);
    }

    #[test]
    fn handshake_lists_the_single_tool_and_reports_the_codex_thread() {
        let gateway = Gateway::start().unwrap();
        let token = gateway.token().to_owned();
        let (_, init) = post(
            &gateway,
            &token,
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}),
        );
        assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
        assert!(!gateway.connected());
        let (_, tools) = post(
            &gateway,
            &token,
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        );
        assert_eq!(tools["result"]["tools"][0]["name"], TOOL);
        assert!(gateway.connected(), "listing the tool shows the CLI is up");
        gateway.begin("t1").unwrap();
        post(
            &gateway,
            &token,
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{
                "name":TOOL,"arguments":{"turn":"t1","text":"ok"},
                "_meta":{"threadId":"thread-7"}}}),
        );
        assert_eq!(gateway.session().as_deref(), Some("thread-7"));
    }

    #[test]
    fn the_stdio_bridge_relays_a_whole_turn_for_agy() {
        let gateway = Gateway::start().unwrap();
        gateway.begin("t1").unwrap();
        let input = [
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{
                "name":TOOL,"arguments":{"turn":"t1","text":"from agy"}}}),
        ]
        .iter()
        .map(|message| format!("{message}\n"))
        .collect::<String>();
        let mut output = Vec::new();
        forward(
            input.as_bytes(),
            &mut output,
            gateway.url(),
            gateway.token(),
            Some("conversation-9"),
        )
        .unwrap();
        let replies = String::from_utf8(output).unwrap();
        let replies: Vec<Value> = replies
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(replies.len(), 3, "the notification gets no reply");
        assert_eq!(replies[1]["result"]["tools"][0]["name"], TOOL);
        assert!(
            replies[2]["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("recorded")
        );
        assert_eq!(gateway.session().as_deref(), Some("conversation-9"));
        let cancel = AtomicBool::new(false);
        let finish = gateway
            .wait("t1", &cancel, Duration::from_secs(2), || Ok(()))
            .unwrap();
        assert_eq!(finish.text, "from agy");
    }

    #[test]
    fn cancellation_and_a_dead_agent_end_the_wait() {
        let gateway = Gateway::start().unwrap();
        gateway.begin("t1").unwrap();
        let cancel = AtomicBool::new(true);
        let error = gateway
            .wait("t1", &cancel, Duration::from_secs(5), || Ok(()))
            .unwrap_err();
        assert_eq!(error.to_string(), CANCELLED);
        gateway.begin("t2").unwrap();
        let cancel = AtomicBool::new(false);
        let error = gateway
            .wait("t2", &cancel, Duration::from_secs(5), || {
                Err(invalid("agent exited"))
            })
            .unwrap_err();
        assert!(error.to_string().contains("agent exited"));
    }
}

#[cfg(test)]
mod role_tests {
    use super::*;

    fn shared(role: &str) -> Shared {
        Shared {
            token: "test".into(),
            state: Mutex::new(State {
                role: role.into(),
                expected: Some("current".into()),
                ..State::default()
            }),
            changed: Condvar::new(),
        }
    }

    #[test]
    fn tools_and_result_schemas_are_role_scoped() {
        for (role, expected) in [
            (
                "worker:task",
                vec!["complete", "select", "ask", "wait_for_answer"],
            ),
            ("verifier:task", vec!["review", "ask", "wait_for_answer"]),
        ] {
            let tools = tool_list(role);
            let names: Vec<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
            assert_eq!(names, expected);
            assert!(!allowed_tools(role).contains(&"revise_plan"));
        }
        let review = &tool_list("verifier:task")[0]["inputSchema"];
        assert_eq!(
            review["required"],
            json!(["turn", "report", "checks", "findings"])
        );
        let finding = &review["properties"]["findings"]["items"];
        for derived in ["id", "approved"] {
            assert!(finding["properties"][derived].is_null(), "{derived}");
            assert!(review["properties"][derived].is_null(), "{derived}");
        }
        assert_eq!(
            allowed_tools("orchestrator"),
            vec![
                "finish_turn",
                "ask_user",
                "submit_requirements",
                "submit_plan",
                "answer_agent",
                "defer_to_user",
                "revise_plan",
                "request_replan",
            ]
        );
        assert_eq!(allowed_tools("planner"), vec!["submit_plan"]);
        for reviewer in ["auditor", "plan-verifier", "code-review:t1"] {
            assert_eq!(allowed_tools(reviewer), vec!["review"], "{reviewer}");
            let review = &tool_list(reviewer)[0]["inputSchema"];
            assert_eq!(review["required"], json!(["turn", "report", "findings"]));
            assert!(review["properties"]["checks"].is_null());
        }
        for (role, tool) in [
            ("requirements-extraction-analyst", "submit_ledger"),
            ("requirements-audit-analyst", "submit_audit"),
            ("requirements-audit-verifier", "submit_audit_review"),
        ] {
            assert_eq!(allowed_tools(role), vec![tool], "{role}");
            assert!(is_terminal(tool));
        }
        let entry = &tool_list("requirements-extraction-analyst")[0]["inputSchema"]["properties"]["entries"]
            ["items"];
        assert_eq!(
            entry["required"],
            json!(["id", "quote", "interpretation", "criteria"])
        );
        for gone in [
            "start",
            "end",
            "position",
            "disposition",
            "implementation_owners",
        ] {
            assert!(entry["properties"][gone].is_null(), "{gone}");
        }
        let evidence = &tool_list("requirements-audit-analyst")[0]["inputSchema"]["properties"]["evidence"]
            ["items"];
        for gone in [
            "entry_id",
            "entry_version",
            "generation",
            "integrated_fingerprint",
        ] {
            assert!(evidence["properties"][gone].is_null(), "{gone}");
        }
        assert_eq!(allowed_tools("anyone-else"), vec!["finish_turn"]);
        let finish = &tool_list("orchestrator")[0]["inputSchema"];
        assert_eq!(finish["required"], json!(["turn", "text"]));
        assert!(finish["properties"]["addressed"].is_object());
        assert!(tool_list("auditor")[0]["inputSchema"]["properties"]["addressed"].is_null());
        let task = &schema::revision()["tasks"]["items"];
        assert_eq!(task["required"], json!(["id", "title", "goal", "kind"]));
        for gone in ["inputs", "outputs", "verifier", "requirement_ids"] {
            assert!(task["properties"][gone].is_null(), "{gone}");
        }
        let planner = tool_list("planner");
        assert_eq!(
            planner[0]["inputSchema"]["required"],
            json!(["turn", "document", "tasks"])
        );
    }

    #[test]
    fn a_submitted_plan_ends_the_turn_and_a_repeat_is_acknowledged() {
        let shared = shared("planner");
        shared.state.lock().unwrap().expected = Some("t1".into());
        let call = json!({"name":"submit_plan","arguments":{"turn":"t1","document":"# Plan","tasks":[{"id":"t1","title":"Do","goal":"Do it","kind":"implementation"}]}});
        assert_eq!(dispatch(&shared, &call).unwrap(), json!({"recorded":true}));
        let finish = shared.state.lock().unwrap().finished.clone().unwrap();
        let (text, plan) = super::super::turn_result::TurnResult::plan_answer(&finish.text);
        assert_eq!(text, "# Plan");
        assert!(plan.unwrap().unwrap()[0].writes);
        shared.state.lock().unwrap().finished = None;
        assert_eq!(dispatch(&shared, &call).unwrap(), json!({"recorded":true}));
        let mut other = call.clone();
        other["arguments"]["document"] = json!("# Other");
        assert!(dispatch(&shared, &other).is_err());
    }

    #[test]
    fn stale_leases_and_foreign_role_commands_are_rejected_before_dispatch() {
        let shared = shared("worker:task");
        let stale =
            json!({"name":"complete","arguments":{"turn":"old","summary":"done","report":"done"}});
        assert!(
            dispatch(&shared, &stale)
                .unwrap_err()
                .to_string()
                .contains("turn is not open")
        );
        let foreign = json!({"name":"answer_agent","arguments":{"turn":"current","request":"q","answer":"ok"}});
        assert!(
            dispatch(&shared, &foreign)
                .unwrap_err()
                .to_string()
                .contains("unavailable for this role")
        );
        assert!(shared.state.lock().unwrap().finished.is_none());
    }

    #[test]
    fn finish_receipt_is_idempotent_after_the_result_is_consumed() {
        let shared = shared("orchestrator");
        let submission = json!({"name":"finish_turn","arguments":{"turn":"current","text":"done"}});
        assert_eq!(dispatch(&shared, &submission).unwrap()["recorded"], true);
        {
            let mut state = shared.state.lock().unwrap();
            state.finished.take();
            state.expected = None;
        }
        assert_eq!(dispatch(&shared, &submission).unwrap()["recorded"], true);
        let mut conflict = submission;
        conflict["arguments"]["text"] = json!("different");
        let error = dispatch(&shared, &conflict).unwrap_err().to_string();
        assert!(
            error.contains("already ended with your earlier finish_turn call"),
            "{error}"
        );
    }
}

#[cfg(test)]
mod rejection_tests {
    use super::*;

    fn submission(turn: &str) -> Value {
        json!({"name":TOOL,"arguments":{"turn":turn,"text":"done"}})
    }

    #[test]
    fn rejected_finish_can_be_corrected_without_accepting_a_stale_turn() {
        let gateway = Gateway::start().unwrap();
        gateway.begin("current").unwrap();
        gateway
            .set_handler(Arc::new(|_, _| {
                Err(invalid("request: unknown question ID"))
            }))
            .unwrap();
        assert_eq!(call(&gateway.shared, &submission("old"))["isError"], true);
        assert!(
            gateway
                .shared
                .state
                .lock()
                .unwrap()
                .rejected_finish
                .is_none()
        );
        assert_eq!(
            call(&gateway.shared, &submission("current"))["isError"],
            true
        );
        assert!(gateway.shared.state.lock().unwrap().finished.is_none());
        let rejected_at = gateway
            .shared
            .state
            .lock()
            .unwrap()
            .rejected_finish
            .as_ref()
            .unwrap()
            .0;
        call(&gateway.shared, &submission("current"));
        assert_eq!(
            gateway
                .shared
                .state
                .lock()
                .unwrap()
                .rejected_finish
                .as_ref()
                .unwrap()
                .0,
            rejected_at
        );
        gateway
            .set_handler(Arc::new(|_, _| Ok(json!({"recorded":true}))))
            .unwrap();
        call(&gateway.shared, &submission("current"));
        assert_eq!(
            gateway
                .wait(
                    "current",
                    &AtomicBool::new(false),
                    Duration::ZERO,
                    || Ok(())
                )
                .unwrap()
                .text,
            "done"
        );
    }

    #[test]
    fn a_correction_after_sixty_seconds_can_finish_within_the_turn_timeout() {
        let gateway = Gateway::start().unwrap();
        gateway.begin("current").unwrap();
        gateway.shared.state.lock().unwrap().rejected_finish = Some((
            Instant::now() - Duration::from_secs(61),
            "old rejection".into(),
        ));
        let finish = gateway
            .wait(
                "current",
                &AtomicBool::new(false),
                Duration::from_secs(1800),
                || {
                    assert_eq!(
                        call(&gateway.shared, &submission("current"))["isError"],
                        Value::Null
                    );
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(finish.text, "done");
    }

    #[test]
    fn rejection_keeps_the_turn_timeout_and_resets_for_a_new_turn() {
        let gateway = Gateway::start().unwrap();
        gateway.begin("current").unwrap();
        gateway.shared.state.lock().unwrap().rejected_finish = Some((
            Instant::now() - Duration::from_secs(61),
            "request: unknown question ID".into(),
        ));
        let error = gateway
            .wait(
                "current",
                &AtomicBool::new(false),
                Duration::ZERO,
                || Ok(()),
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("request: unknown question ID"));
        assert!(error.contains("no accepted result"), "{error}");
        assert!(gateway.shared.state.lock().unwrap().expected.is_none());
        gateway.begin("next").unwrap();
        assert!(
            gateway
                .shared
                .state
                .lock()
                .unwrap()
                .rejected_finish
                .is_none()
        );
    }

    #[test]
    fn cli_exit_retains_rejection_and_closes_the_turn() {
        let gateway = Gateway::start().unwrap();
        gateway.begin("current").unwrap();
        gateway.shared.state.lock().unwrap().rejected_finish =
            Some((Instant::now(), "bad checkpoint".into()));
        let error = gateway
            .wait(
                "current",
                &AtomicBool::new(false),
                Duration::from_secs(1800),
                || Err(invalid("CLI exited (1)")),
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("CLI exited (1)"));
        assert!(error.contains("last rejected result: bad checkpoint"));
        assert!(gateway.shared.state.lock().unwrap().expected.is_none());
    }

    #[test]
    fn timeout_retains_the_rejection_reason() {
        let gateway = Gateway::start().unwrap();
        gateway.begin("current").unwrap();
        gateway.shared.state.lock().unwrap().rejected_finish =
            Some((Instant::now(), "bad checkpoint".into()));
        let error = gateway
            .wait(
                "current",
                &AtomicBool::new(false),
                Duration::ZERO,
                || Ok(()),
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("last rejected submission: bad checkpoint"));
    }
}

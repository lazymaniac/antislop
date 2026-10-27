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
pub(crate) const TOOLS: &[&str] = &[
    TOOL,
    "ask_orchestrator",
    "await_orchestrator",
    "reply_to_agent",
    "submit_plan_revision",
    "request_replan",
    "task_risks",
];
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
            if let Some((at, error)) = &state.rejected_finish
                && at.elapsed() >= Duration::from_secs(60)
            {
                let error = format!(
                    "{TOOL} rejected for turn {turn}; no accepted correction within 60 s: {error}"
                );
                state.expected = None;
                return Err(invalid(error));
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
                    "no accepted {TOOL} for turn {turn} within {} s{rejection}",
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
                        format!("; last rejected {TOOL}: {reason}")
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

fn allowed_tools(role: &str) -> Vec<&'static str> {
    if super::task_role(role).is_some() {
        vec![TOOL, "ask_orchestrator", "await_orchestrator"]
    } else if matches!(role, "orchestrator" | "comment-orchestrator") {
        vec![
            TOOL,
            "reply_to_agent",
            "submit_plan_revision",
            "request_replan",
            "task_risks",
        ]
    } else if role == "planner" {
        vec![TOOL, "task_risks"]
    } else {
        vec![TOOL]
    }
}

fn tool_list(role: &str) -> Vec<Value> {
    allowed_tools(role).iter().map(|name| {
        let (description, fields, required) = match *name {
            "finish_turn" => ("Record the typed result of the current turn. Stop only after recorded=true; if rejected, correct the error and resubmit within 60 seconds.", json!({"result":{"type":"object","description":"Tasks: completed {summary,report,outputs,selection?}, review {report,verdict}, or needs_input {request,checkpoint}. Other roles: report {text}. Each object requires type."}}), vec!["turn","result"]),
            "ask_orchestrator" => ("Ask a question about your local task without ending your turn.", json!({"request_key":{"type":"string"},"question":{"type":"string"}}), vec!["turn","request_key","question"]),
            "await_orchestrator" => ("Wait up to 25 seconds for your answer; pending does not mean failure.", json!({"request":{"type":"string","description":"Question ID returned by ask_orchestrator, not question text"}}), vec!["turn","request"]),
            "task_risks" => ("Read the domain catalog before selecting relevant verifier.risk_ids for a local contract.", json!({"kind":{"type":"string"}}),vec!["turn","kind"]),
            "reply_to_agent" => ("Orchestrator only: answer a pending question. Answers cannot change contracts or permissions.", json!({"request":{"type":"string","description":"Question ID returned by ask_orchestrator, not question text"},"answer":{"type":"string"},"state":{"enum":["answered","awaiting_user"]}}), vec!["turn","request","answer"]),
            "request_replan" => ("Orchestrator only: ask the planner to redraft the whole plan when contract repairs are not enough. It is reviewed like any new plan.", json!({"reason":{"type":"string","description":"What the current plan gets wrong and what the new plan must change"}}), vec!["turn","reason"]),
            _ => ("Orchestrator only: submit a complete plan revision for independent review.", json!({"request_key":{"type":"string"},"revision":schema::revision()}), vec!["turn","request_key","revision"]),
        };
        let mut properties = fields.as_object().unwrap().clone();
        if *name == TOOL { properties.insert("result".into(),schema::result(role)); }
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
            if params["name"] == TOOL
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
        if name == TOOL
            && let Some((recorded, result)) = &state.receipt
            && recorded == turn
        {
            crate::require(
                result == &args["result"],
                "turn already recorded with another result",
            )?;
            return Ok(json!({"recorded":true}));
        }
        crate::require(
            state.expected.as_deref() == Some(turn),
            "turn is not open; use the current prompt's turn",
        )?;
        crate::require(state.finished.is_none(), "turn is already finished")?;
        if name == "await_orchestrator" {
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
        None if name == TOOL => {
            serde_json::from_value::<super::turn_result::TurnResult>(args["result"].clone())
                .map(|_| json!({"recorded":true}))
                .map_err(Into::into)
        }
        None => Err(invalid("agent tool handler is not installed")),
    };
    let mut state = shared.state.lock().map_err(invalid)?;
    if name == "await_orchestrator"
        && let Some(at) = state.waiting.take()
    {
        state.waited += at.elapsed();
    }
    let response = response?;
    crate::require(
        state.expected.as_deref() == Some(turn),
        "turn closed while the tool was executing",
    )?;
    if name == TOOL {
        let result: super::turn_result::TurnResult =
            serde_json::from_value(args["result"].clone())?;
        state.receipt = Some((turn.into(), args["result"].clone()));
        let text = match result {
            super::turn_result::TurnResult::Report { text } => text,
            other => serde_json::to_string(&other)?,
        };
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
            "name":TOOL,"arguments":{"turn":turn,"result":{"type":"report","text":text}}}})
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
                "name":TOOL,"arguments":{"turn":"t1","result":{"type":"report","text":"ok"}},
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
                "name":TOOL,"arguments":{"turn":"t1","result":{"type":"report","text":"from agy"}}}}),
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
        for role in ["worker:task", "verifier:task"] {
            let tools = tool_list(role);
            let names: Vec<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
            assert_eq!(
                names,
                vec!["finish_turn", "ask_orchestrator", "await_orchestrator"]
            );
            assert!(tools[0]["inputSchema"]["properties"]["result"]["oneOf"].is_array());
            assert!(!allowed_tools(role).contains(&"submit_plan_revision"));
        }
        assert_eq!(
            allowed_tools("orchestrator"),
            vec![
                "finish_turn",
                "reply_to_agent",
                "submit_plan_revision",
                "request_replan",
                "task_risks"
            ]
        );
        assert_eq!(allowed_tools("planner"), vec!["finish_turn", "task_risks"]);
        assert_eq!(allowed_tools("auditor"), vec!["finish_turn"]);
        assert!(
            schema::revision()["properties"]["tasks"]["items"]["properties"]["inputs"].is_object()
        );
    }

    #[test]
    fn stale_leases_and_foreign_role_commands_are_rejected_before_dispatch() {
        let shared = shared("worker:task");
        let stale = json!({"name":"finish_turn","arguments":{"turn":"old","result":{"type":"report","text":"done"}}});
        assert!(
            dispatch(&shared, &stale)
                .unwrap_err()
                .to_string()
                .contains("turn is not open")
        );
        let foreign = json!({"name":"reply_to_agent","arguments":{"turn":"current","request":"q","answer":"ok"}});
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
        let submission = json!({"name":"finish_turn","arguments":{"turn":"current","result":{"type":"report","text":"done"}}});
        assert_eq!(dispatch(&shared, &submission).unwrap()["recorded"], true);
        {
            let mut state = shared.state.lock().unwrap();
            state.finished.take();
            state.expected = None;
        }
        assert_eq!(dispatch(&shared, &submission).unwrap()["recorded"], true);
        let mut conflict = submission;
        conflict["arguments"]["result"]["text"] = json!("different");
        assert!(dispatch(&shared, &conflict).is_err());
    }
}

#[cfg(test)]
mod rejection_tests {
    use super::*;

    fn submission(turn: &str) -> Value {
        json!({"name":TOOL,"arguments":{"turn":turn,"result":{"type":"report","text":"done"}}})
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
    fn rejection_has_bounded_recovery_time_and_resets_for_a_new_turn() {
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
                Duration::from_secs(1800),
                || Ok(()),
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("request: unknown question ID"));
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
        assert!(error.contains("last rejected finish_turn: bad checkpoint"));
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

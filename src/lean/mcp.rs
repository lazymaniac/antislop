//! The one channel back from an interactive agent: a loopback MCP server
//! with a single tool, `finish_turn`. The daemon types the prompt into the
//! agent's terminal; the agent ends the turn by calling the tool with the
//! turn's id and its final answer. Nothing is read from the terminal screen.
use super::native::invalid;
use crate::{Error, Result};
use serde_json::{Value, json};
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

pub(crate) const SERVER: &str = "antislop";
pub(crate) const TOOL: &str = "finish_turn";
/// The error of a turn the runtime cancelled while it waited.
pub(crate) const CANCELLED: &str = "agent turn cancelled";
/// Largest request body accepted; an answer is text, not a repository.
const BODY: u64 = 4 * 1024 * 1024;

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
        Ok(())
    }

    /// Whether the CLI has connected and discovered the tool; every supported
    /// CLI does so while starting, before it takes a prompt.
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
        let deadline = Instant::now() + timeout;
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
            if Instant::now() >= deadline {
                state.expected = None;
                return Err(Error::Invalid(format!(
                    "the agent did not call {TOOL} for turn {turn} within {} s",
                    timeout.as_secs()
                )));
            }
            drop(state);
            alive()?;
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
    use std::io::{BufRead, Write};
    let url = std::env::var("ANTISLOP_MCP_URL")
        .map_err(|_| invalid("ANTISLOP_MCP_URL is not set; mcp-bridge is started by an agent CLI"))?;
    let token = std::env::var("ANTISLOP_MCP_TOKEN")
        .map_err(|_| invalid("ANTISLOP_MCP_TOKEN is not set"))?;
    let conversation = std::env::var("ANTIGRAVITY_CONVERSATION_ID").ok();
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    let mut out = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let line = line.map_err(Error::Io)?;
        if line.trim().is_empty() {
            continue;
        }
        let mut message: Value = serde_json::from_str(&line)?;
        if let Some(conversation) = &conversation
            && message["method"] == "tools/call"
        {
            message["params"]["_meta"]["sessionId"] = Value::String(conversation.clone());
        }
        let mut response = agent
            .post(&url)
            .header("authorization", format!("Bearer {token}"))
            .send_json(&message)
            .map_err(|error| invalid(format!("gateway unreachable: {error}")))?;
        let body = response.body_mut().read_to_string().map_err(invalid)?;
        // Notifications are acknowledged with an empty body: nothing to relay.
        if let Ok(reply) = serde_json::from_str::<Value>(&body) {
            writeln!(out, "{reply}").map_err(Error::Io)?;
            out.flush().map_err(Error::Io)?;
        }
    }
    Ok(())
}

fn respond(request: tiny_http::Request, status: u16, body: Option<Value>) -> Result<()> {
    let mut response = tiny_http::Response::from_string(
        body.map(|body| body.to_string()).unwrap_or_default(),
    )
    .with_status_code(status);
    response.add_header(
        tiny_http::Header::from_bytes("content-type", "application/json").map_err(|_| {
            invalid("invalid content-type header")
        })?,
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
            result(id, json!({"tools": [tool()]}))
        }
        "tools/call" => result(id, call(shared, &message["params"])),
        other => error(id, -32601, &format!("method not supported: {other}")),
    };
    respond(request, 200, Some(reply))
}

fn tool() -> Value {
    json!({
        "name": TOOL,
        "description": "End your current turn. Call it exactly once, as your last action, \
                        with the turn id you were given and your complete final answer.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "turn": {"type": "string", "description": "The turn id from the prompt."},
                "text": {"type": "string", "description": "Your complete final answer."}
            },
            "required": ["turn", "text"]
        }
    })
}

/// Runs the tool; a refusal is a tool error the model can read and correct.
fn call(shared: &Shared, params: &Value) -> Value {
    let refuse = |message: &str| json!({"isError": true, "content": [{"type":"text","text":message}]});
    for key in ["threadId", "sessionId"] {
        if let Some(id) = params["_meta"][key].as_str()
            && let Ok(mut state) = shared.state.lock()
        {
            state.session.get_or_insert_with(|| id.into());
        }
    }
    if params["name"] != TOOL {
        return refuse("unknown tool");
    }
    let (Some(turn), Some(text)) = (
        params["arguments"]["turn"].as_str(),
        params["arguments"]["text"].as_str(),
    ) else {
        return refuse("turn and text are required");
    };
    let Ok(mut state) = shared.state.lock() else {
        return refuse("gateway failed");
    };
    if state.expected.as_deref() != Some(turn) {
        return refuse("that turn is not open; use the turn id from your latest prompt");
    }
    if state.finished.is_some() {
        return refuse("this turn is already finished");
    }
    state.finished = Some(Finish { text: text.into() });
    shared.changed.notify_all();
    json!({"content": [{"type": "text", "text": "turn recorded"}]})
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

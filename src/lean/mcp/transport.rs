use super::dispatch::{call, error, result};
use super::gateway::{BODY, SERVER, Shared};
use super::tools::tool_list;
use crate::lean::native::invalid;
use crate::{Error, Result};
use serde_json::{Value, json};
use std::io::Read;

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
pub(super) fn forward(
    input: impl std::io::BufRead,
    mut output: impl std::io::Write,
    url: &str,
    token: &str,
    conversation: Option<&str>,
) -> Result<()> {
    let agent = gateway_client();
    for line in input.lines() {
        let line = line.map_err(Error::Io)?;
        if !line.trim().is_empty() {
            forward_line(&agent, &line, &mut output, url, token, conversation)?;
        }
    }
    Ok(())
}

fn gateway_client() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into()
}

fn forward_line(
    agent: &ureq::Agent,
    line: &str,
    output: &mut impl std::io::Write,
    url: &str,
    token: &str,
    conversation: Option<&str>,
) -> Result<()> {
    let message = with_session(line, conversation)?;
    let mut response = agent
        .post(url)
        .header("authorization", format!("Bearer {token}"))
        .send_json(&message)
        .map_err(|error| invalid(format!("gateway unreachable: {error}")))?;
    write_reply(
        response.body_mut().read_to_string().map_err(invalid)?,
        output,
    )
}

fn with_session(line: &str, conversation: Option<&str>) -> Result<Value> {
    let mut message: Value = serde_json::from_str(line)?;
    if let Some(conversation) = conversation
        && message["method"] == "tools/call"
    {
        message["params"]["_meta"]["sessionId"] = Value::String(conversation.into());
    }
    Ok(message)
}

fn write_reply(body: String, output: &mut impl std::io::Write) -> Result<()> {
    if let Ok(reply) = serde_json::from_str::<Value>(&body) {
        writeln!(output, "{reply}").map_err(Error::Io)?;
        output.flush().map_err(Error::Io)?;
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

pub(super) fn serve(shared: &Shared, mut request: tiny_http::Request) -> Result<()> {
    if !authorized(&request, &shared.token) {
        return respond(request, 401, None);
    }
    if request.method() != &tiny_http::Method::Post {
        return respond(request, 405, None);
    }
    let Some(message) = read_message(&mut request)? else {
        return respond(
            request,
            400,
            Some(error(Value::Null, -32700, "invalid JSON")),
        );
    };
    let Some(id) = message.get("id").cloned() else {
        return respond(request, 202, None);
    };
    let reply = handle_message(shared, &message, id)?;
    respond(request, 200, Some(reply))
}

fn authorized(request: &tiny_http::Request, token: &str) -> bool {
    request.headers().iter().any(|header| {
        header.field.equiv("authorization") && header.value.as_str() == format!("Bearer {token}")
    })
}

fn read_message(request: &mut tiny_http::Request) -> Result<Option<Value>> {
    let mut text = String::new();
    request
        .as_reader()
        .take(BODY)
        .read_to_string(&mut text)
        .map_err(Error::Io)?;
    Ok(serde_json::from_str(&text).ok())
}

fn handle_message(shared: &Shared, message: &Value, id: Value) -> Result<Value> {
    match message["method"].as_str().unwrap_or_default() {
        "initialize" => Ok(result(id, initialize_result(message))),
        "ping" => Ok(result(id, json!({}))),
        "tools/list" => list_tools(shared, id),
        "tools/call" => Ok(result(id, call(shared, &message["params"]))),
        other => Ok(error(id, -32601, &format!("method not supported: {other}"))),
    }
}

fn initialize_result(message: &Value) -> Value {
    json!({
        "protocolVersion": message["params"]["protocolVersion"].as_str().unwrap_or("2025-03-26"),
        "capabilities": {"tools": {}},
        "serverInfo": {"name": SERVER, "version": env!("CARGO_PKG_VERSION")}
    })
}

fn list_tools(shared: &Shared, id: Value) -> Result<Value> {
    if let Ok(mut state) = shared.state.lock() {
        state.listed = true;
    }
    let role = shared.state.lock().map_err(invalid)?.role.clone();
    Ok(result(id, json!({"tools": tool_list(&role)})))
}

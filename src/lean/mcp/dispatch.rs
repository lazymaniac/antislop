use super::gateway::{
    ASK, Finish, Shared, TOOL, TOOLS, is_terminal, receipt_payload, terminal_result,
};
use super::tools::scoped_tools;
use crate::Result;
use crate::lean::native::invalid;
use serde_json::{Value, json};
use std::time::Instant;

/// Validate the open lease before dispatch; never hold the gateway lock during work.
pub(super) fn call(shared: &Shared, params: &Value) -> Value {
    let (expected, observer, role, allowed) = shared
        .state
        .lock()
        .map(|state| {
            (
                state.expected.clone(),
                state.protocol_observer.clone(),
                state.role.clone(),
                scoped_tools(&state.role, state.contract.as_ref()),
            )
        })
        .unwrap_or_default();
    let outcome = dispatch(shared, params);
    match outcome {
        Ok(value) => json!({"content":[{"type":"text","text":value.to_string()}]}),
        Err(error) => {
            let mut reason = error.to_string();
            if is_terminal(params["name"].as_str().unwrap_or_default())
                && let Ok(mut state) = shared.state.lock()
                && state.expected == expected
                && state.expected.is_some()
                && state.finished.is_none()
            {
                let name = params["name"].as_str().unwrap_or_default();
                if state.expected.as_deref() != params["arguments"]["turn"].as_str()
                    || !scoped_tools(&state.role, state.contract.as_ref()).contains(&name)
                {
                    state.protocol_rejected_at.get_or_insert_with(Instant::now);
                    reason.push_str(&format!(
                        "; role {} requires turn = {:?} and one of {:?}; correct the call within 60 s of the first protocol rejection",
                        state.role, state.expected.as_deref().unwrap(),
                        scoped_tools(&state.role, state.contract.as_ref())
                    ));
                    if scoped_tools(&state.role, state.contract.as_ref()).contains(&"current_turn")
                    {
                        reason.push_str("; call current_turn({}) and copy its active turn id exactly; never guess or reuse a previous id");
                    }
                }
                let at = state
                    .rejected_finish
                    .as_ref()
                    .map_or_else(Instant::now, |(at, _)| *at);
                state.rejected_finish = Some((at, reason.clone()));
                shared.changed.notify_all();
            }
            let name = params["name"].as_str().unwrap_or_default();
            let submitted = params["arguments"]["turn"].as_str();
            let active_turn = shared
                .state
                .lock()
                .ok()
                .and_then(|state| state.expected.clone());
            let protocol_error = active_turn != expected
                || !allowed.contains(&name)
                || (name != "current_turn" && submitted != expected.as_deref());
            if protocol_error && let Some(observer) = observer {
                let event = json!({"stage":"gateway","tool":name.chars().take(80).collect::<String>(),"error":reason,
                    "expected_turn":expected,"active_turn":active_turn,"submitted_turn":submitted.map(|turn|turn.chars().take(128).collect::<String>()),"role":role});
                if let Err(error) = observer(&event) {
                    reason.push_str(&format!("; recording protocol rejection failed: {error}"));
                }
            }
            json!({"isError":true,"content":[{"type":"text","text":reason}]})
        }
    }
}

pub(super) fn dispatch(shared: &Shared, params: &Value) -> Result<Value> {
    let name = params["name"].as_str().unwrap_or_default();
    crate::require(TOOLS.contains(&name), "unknown tool")?;
    let args = &params["arguments"];
    if name == "current_turn" {
        return current_turn(shared, args);
    }
    let turn = args["turn"]
        .as_str()
        .ok_or_else(|| invalid("turn: required"))?;
    let prepared = prepare_dispatch(shared, params, name, turn, args)?;
    let Prepared::Invoke(handler) = prepared else {
        return Ok(json!({"recorded":true}));
    };
    let response = invoke_handler(handler, name, args);
    finish_dispatch(shared, name, turn, args, response)
}

enum Prepared {
    Replay,
    Invoke(Option<std::sync::Arc<super::gateway::Handler>>),
}

fn prepare_dispatch(
    shared: &Shared,
    params: &Value,
    name: &str,
    turn: &str,
    args: &Value,
) -> Result<Prepared> {
    let mut state = shared.state.lock().map_err(invalid)?;
    crate::require(
        scoped_tools(&state.role, state.contract.as_ref()).contains(&name),
        "tool is unavailable for this role",
    )?;
    remember_session(&mut state, params);
    if replay_terminal(&state, name, turn, args)? {
        return Ok(Prepared::Replay);
    }
    require_open_turn(&mut state, name, turn)?;
    // A correctly scoped result is progress. Semantic validation can still
    // reject its contents without using the protocol correction deadline.
    if is_terminal(name) {
        state.protocol_rejected_at = None;
    }
    Ok(Prepared::Invoke(state.handler.clone()))
}

fn require_open_turn(state: &mut super::gateway::State, name: &str, turn: &str) -> Result<()> {
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
    Ok(())
}

fn remember_session(state: &mut super::gateway::State, params: &Value) {
    for key in ["threadId", "sessionId"] {
        if let Some(id) = params["_meta"][key].as_str() {
            state.session.get_or_insert_with(|| id.into());
        }
    }
}

fn replay_terminal(
    state: &super::gateway::State,
    name: &str,
    turn: &str,
    args: &Value,
) -> Result<bool> {
    if !is_terminal(name)
        || state
            .receipt
            .as_ref()
            .is_none_or(|(recorded, _)| recorded != turn)
    {
        return Ok(false);
    }
    let (_, result) = state.receipt.as_ref().expect("receipt checked");
    crate::require(
        result == &receipt_payload(name, args),
        &format!(
            "this turn already ended with your earlier {} call; that result stands and this call changed nothing. Stop now.",
            result["tool"].as_str().unwrap_or(TOOL)
        ),
    )?;
    Ok(true)
}

fn invoke_handler(
    handler: Option<std::sync::Arc<super::gateway::Handler>>,
    name: &str,
    args: &Value,
) -> Result<Value> {
    match handler {
        Some(handler) => handler(name, args),
        None if is_terminal(name) => terminal_result(name, args).map(|_| json!({"recorded":true})),
        None => Err(invalid("agent tool handler is not installed")),
    }
}

fn finish_dispatch(
    shared: &Shared,
    name: &str,
    turn: &str,
    args: &Value,
    response: Result<Value>,
) -> Result<Value> {
    let mut state = shared.state.lock().map_err(invalid)?;
    crate::require(
        state.expected.as_deref() == Some(turn),
        "turn closed while the tool was executing",
    )?;
    record_wait_time(&mut state, name);
    let mut response = response?;
    if is_terminal(name)
        && !(name == "wait_for_answer"
            && response["state"] == "answered"
            && response["recorded"] != true)
    {
        finish_terminal(shared, &mut state, name, turn, args, &mut response)?;
    }
    Ok(response)
}

fn record_wait_time(state: &mut super::gateway::State, name: &str) {
    if name == ASK
        && let Some(at) = state.waiting.take()
    {
        state.waited += at.elapsed();
    }
}

fn finish_terminal(
    shared: &Shared,
    state: &mut super::gateway::State,
    name: &str,
    turn: &str,
    args: &Value,
    response: &mut Value,
) -> Result<()> {
    let result = match response
        .as_object_mut()
        .and_then(|object| object.remove("result"))
    {
        Some(result) => serde_json::from_value(result)?,
        None => terminal_result(name, args)?,
    };
    state.receipt = Some((turn.into(), receipt_payload(name, args)));
    state.finished = Some(Finish {
        text: result.encode()?,
    });
    shared.changed.notify_all();
    Ok(())
}

pub(super) fn result(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

pub(super) fn error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn current_turn(shared: &Shared, args: &Value) -> Result<Value> {
    crate::require(
        args.as_object().is_some_and(|args| args.is_empty()),
        "current_turn: expected an empty object",
    )?;
    let state = shared.state.lock().map_err(invalid)?;
    crate::require(
        scoped_tools(&state.role, state.contract.as_ref()).contains(&"current_turn"),
        "tool is unavailable for this role",
    )?;
    Ok(
        json!({"turn":state.expected,"active":state.expected.is_some() && state.finished.is_none(),"role":state.role,
        "tools":scoped_tools(&state.role, state.contract.as_ref()),"last_finished_turn":state.receipt.as_ref().map(|(turn,_)|turn)}),
    )
}

#[cfg(test)]
mod tests;

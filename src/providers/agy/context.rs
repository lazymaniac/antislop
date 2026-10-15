use super::*;
use crate::context::{Page, Read, bridge::Bridge, transport::Transport};
use std::collections::{BTreeMap, BTreeSet};

pub fn cwd(bridge: &Bridge) -> Result<PathBuf> {
    let cwd = bridge.root().join("native");
    crate::ipc::private_directory(&cwd)?;
    let agents = cwd.join(".agents");
    crate::ipc::private_directory(&agents)?;
    let definitions = agents.join("agents");
    crate::ipc::private_directory(&definitions)?;
    let source=std::str::from_utf8(AGENT_SOURCE).expect("static profile")
        .replace("inheritMcp: false","inheritMcp: true")
        .replace("Never use any other tool or delegate.","You may use only the antislop-context immutable MCP reader and finish. Never use other tools or delegate.");
    controlled_file(&definitions.join("antislop-stdio.md"), source.as_bytes())?;
    let config = json!({"mcpServers":{"antislop-context":{"command":crate::process::companion()?,"args":["__context-connect",bridge.socket()]}}});
    controlled_file(
        &agents.join("mcp_config.json"),
        &serde_json::to_vec(&config)?,
    )?;
    Ok(cwd.canonicalize()?)
}
#[derive(Default)]
pub struct Tools {
    active: BTreeMap<u64, Value>,
    closed: BTreeSet<u64>,
}
impl Tools {
    pub fn frame(
        &mut self,
        frame: &Value,
        context: &mut Transport,
        native: Option<&str>,
        turn_id: &str,
        events: &mut dyn FnMut(&ProviderEvent) -> Result<()>,
    ) -> Result<bool> {
        let step = &frame["step_update"];
        if frame["event"] != "step_update"
            || step["step_type"] != "tool"
            || step["tool_name"] != "call_mcp_tool"
        {
            return Ok(false);
        }
        let info = &step["tool_info"];
        let parameters = &info["parameters"];
        require(
            native.is_some()
                && native == step["conversation_id"].as_str()
                && step.get("subagent_info").is_none_or(Value::is_null)
                && info["name"] == "call_mcp_tool"
                && parameters["ServerName"] == "antislop-context"
                && parameters["ToolName"] == "read_context"
                && parameters
                    .as_object()
                    .is_some_and(|p| p.len() == 3 && p.contains_key("Arguments")),
            "MCP event escapes the pinned context reader",
        )?;
        let index = step["step_index"]
            .as_u64()
            .ok_or_else(|| policy("context tool step has no stable index"))?;
        require(
            !self.closed.contains(&index),
            "duplicate closed context tool step",
        )?;
        match step["state"].as_str() {
            Some("ACTIVE") => {
                require(
                    self.active.get(&index).is_none_or(|p| p == parameters),
                    "active context tool arguments changed",
                )?;
                self.active.insert(index, parameters.clone());
            }
            Some("DONE" | "ERROR") => {
                require(
                    self.active.remove(&index).as_ref() == Some(parameters),
                    "context tool completion has no matching start",
                )?;
                self.closed.insert(index);
                if step["state"] == "ERROR" {
                    require(
                        info.get("error").is_some_and(|v| !v.is_null())
                            && info["output"]
                                .as_str()
                                .is_some_and(|s| s.starts_with("Context read refused: ")),
                        "unrecognized context tool failure",
                    )?;
                    return Ok(true);
                }
                require(
                    info.get("error").is_none_or(Value::is_null),
                    "successful context response contains an error",
                )?;
                let read: Read =
                    crate::transport::decode(&serde_json::to_vec(&parameters["Arguments"])?)?;
                let reader = context.reader()?;
                let page: Page = if let Some(text) = info["output"].as_str() {
                    crate::transport::decode(text.as_bytes())?
                } else {
                    require(
                        info["output"].is_null(),
                        "unsupported native context output shape",
                    )?;
                    // CLI 1.3.5 omits large output bodies from step_update.
                    // Match the completed call to an ACTUAL committed reader
                    // response; absence of an echo never manufactures a read.
                    let receipt = context
                        .receipts
                        .iter()
                        .find(|receipt| {
                            !context.observed.contains(&receipt.sequence)
                                && reader
                                    .read(&read, receipt.sequence)
                                    .is_ok_and(|p| p.receipt == **receipt)
                        })
                        .ok_or_else(|| {
                            policy("native context completion has no committed response")
                        })?;
                    reader.read(&read, receipt.sequence)?
                };
                let expected = reader.read(&read, page.receipt.sequence)?;
                require(
                    expected.receipt == page.receipt
                        && expected.content == page.content
                        && expected.next_offset == page.next_offset
                        && context
                            .receipts
                            .get(page.receipt.sequence.saturating_sub(1) as usize)
                            == Some(&page.receipt),
                    "native context response differs from its committed request",
                )?;
                events(&ProviderEvent::ContextObserved {
                    turn_id: turn_id.into(),
                    sequence: page.receipt.sequence,
                })?;
                context.observe(page.receipt.sequence)?;
            }
            _ => return Err(policy("unknown context tool event state")),
        }
        Ok(true)
    }
}

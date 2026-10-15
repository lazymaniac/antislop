//! Authenticate the sole permitted MCP tool against the daemon's committed bytes.
use super::*;
use crate::context::{Page, Read, bridge::Bridge, transport::Transport};
use std::collections::{BTreeMap, BTreeSet};

pub fn configure(command: &mut std::process::Command, bridge: &Bridge) -> Result<()> {
    let binary = crate::process::companion()?;
    let binary = binary
        .to_str()
        .ok_or_else(|| Error::Invalid("non-UTF-8 context launcher".into()))?;
    let socket = bridge
        .socket()
        .to_str()
        .ok_or_else(|| Error::Invalid("non-UTF-8 context socket".into()))?;
    let server = format!(
        "mcp_servers={{antislop-context={{command={},args=[\"__context-connect\",{}],enabled_tools=[\"read_context\"],required=true,tools={{read_context={{output_token_limit=30000}}}}}}}}",
        serde_json::to_string(binary)?,
        serde_json::to_string(socket)?
    );
    command.arg("-c").arg(server);
    Ok(())
}

#[derive(Default)]
pub struct Tools {
    active: BTreeMap<String, Value>,
    closed: BTreeSet<String>,
}
impl Tools {
    pub fn frame(
        &mut self,
        frame: &Value,
        context: &mut Transport,
        turn_id: &str,
        events: &mut dyn FnMut(&ProviderEvent) -> Result<()>,
    ) -> Result<()> {
        let item = &frame["item"];
        if item["type"] != "mcp_tool_call" {
            return Ok(());
        }
        require(
            item["server"] == "antislop-context" && item["tool"] == "read_context",
            "MCP event escapes the pinned context reader",
        )?;
        let id = item["id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::Invalid("context call has no stable identity".into()))?;
        require(!self.closed.contains(id), "duplicate context completion")?;
        match frame["type"].as_str() {
            Some("item.started") => {
                require(
                    item["status"] == "in_progress" && !self.active.contains_key(id),
                    "invalid context call start",
                )?;
                // Check the request shape before allowing a successful observation.
                let _: Read = crate::transport::decode(&serde_json::to_vec(&item["arguments"])?)?;
                self.active.insert(id.into(), item["arguments"].clone());
            }
            Some("item.completed") => {
                require(
                    self.active.remove(id).as_ref() == Some(&item["arguments"]),
                    "context response has no matching request",
                )?;
                self.closed.insert(id.into());
                require(
                    item["status"] == "completed" && item["error"].is_null(),
                    "native context call failed",
                )?;
                let read: Read =
                    crate::transport::decode(&serde_json::to_vec(&item["arguments"])?)?;
                let result = &item["result"];
                require(
                    result["isError"].as_bool().is_none_or(|v| !v),
                    "native context response is an error",
                )?;
                let content = result["content"]
                    .as_array()
                    .filter(|v| v.len() == 1)
                    .ok_or_else(|| Error::Invalid("missing context response body".into()))?;
                require(
                    content[0]["type"] == "text",
                    "unsupported context response content",
                )?;
                let text = content[0]["text"]
                    .as_str()
                    .ok_or_else(|| Error::Invalid("missing context response text".into()))?;
                let page: Page = crate::transport::decode(text.as_bytes())?;
                let expected = context.reader()?.read(&read, page.receipt.sequence)?;
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
            _ => return Err(Error::Invalid("unsupported context call event".into())),
        }
        Ok(())
    }
}

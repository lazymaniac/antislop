//! Tool-free Claude Code print-mode adapter. CLI authentication stays CLI-owned.
use super::*;
use crate::config::IsolationProfile;
use serde_json::Value;

#[derive(Default)]
pub struct ClaudeProvider {
    pub binary: Option<String>,
    pub model: Option<String>,
    pub managed_home: Option<std::path::PathBuf>,
    pub effort: Option<String>,
    pub isolation: IsolationProfile,
}
const FLAGS: &[&str] = &[
    "--safe-mode",
    "--strict-mcp-config",
    "--tools",
    "--permission-prompts",
    "--session-id",
    "--resume",
    "--json-schema",
    "--disable-slash-commands",
];
impl ClaudeProvider {
    fn binary(&self) -> &str {
        self.binary.as_deref().unwrap_or("claude")
    }
    fn inspect(&self, cancelled: Option<&AtomicBool>) -> Result<String> {
        if self.isolation != IsolationProfile::ToolFree {
            return Err(failure(
                "claude",
                FailureKind::Unsupported,
                "only the enforced tool_free CLI profile is implemented; OS sandbox profiles are unavailable",
            ));
        }
        if self
            .effort
            .as_deref()
            .is_some_and(|effort| !matches!(effort, "low" | "medium" | "high" | "xhigh" | "max"))
        {
            return Err(failure(
                "claude",
                FailureKind::Unsupported,
                "unsupported native effort value",
            ));
        }
        shared_cli::inspect("claude", self.binary(), "2.1.296", FLAGS, cancelled)
    }
}

pub fn normalize(value: &Value) -> Result<shared_cli::StreamEvent> {
    use shared_cli::StreamEvent as Event;
    let kind = value["type"]
        .as_str()
        .ok_or_else(|| failure("claude", FailureKind::Protocol, "event missing type"))?;
    if value
        .get("parent_tool_use_id")
        .is_some_and(|id| !id.is_null())
    {
        return Ok(Event::ForbiddenTool);
    }
    Ok(match kind {
        "system" if value["subtype"] == "init" => {
            if value
                .get("tools")
                .and_then(Value::as_array)
                .is_some_and(|tools| {
                    tools
                        .iter()
                        .any(|tool| tool.as_str() != Some("StructuredOutput"))
                })
            {
                return Ok(Event::ForbiddenTool);
            }
            Event::Session(
                value["session_id"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| {
                        failure("claude", FailureKind::Protocol, "init missing session ID")
                    })?
                    .into(),
            )
        }
        "assistant" => {
            if value["message"]["content"].as_array().is_some_and(|items| {
                items
                    .iter()
                    .any(|item| item["type"] == "tool_use" && item["name"] != "StructuredOutput")
            }) {
                Event::ForbiddenTool
            } else {
                Event::Unknown
            }
        }
        "result" => {
            if value["is_error"] == true
                || value["subtype"]
                    .as_str()
                    .is_some_and(|subtype| subtype != "success")
            {
                Event::Failed(shared_cli::error_kind(&value["error"]))
            } else {
                let output =
                    if let Some(output) = value.get("structured_output").filter(|v| !v.is_null()) {
                        serde_json::to_string(output)?
                    } else {
                        value["result"]
                            .as_str()
                            .ok_or_else(|| {
                                failure(
                                    "claude",
                                    FailureKind::Protocol,
                                    "successful result missing structured output",
                                )
                            })?
                            .into()
                    };
                let usage = if value.get("usage").is_some() {
                    UsageReport::Reported {
                        input_tokens: value["usage"]["input_tokens"].as_u64(),
                        output_tokens: value["usage"]["output_tokens"].as_u64(),
                        cached_tokens: value["usage"]["cache_read_input_tokens"].as_u64(),
                        estimated_usd: value["total_cost_usd"].as_f64(),
                        cumulative: true,
                    }
                } else {
                    UsageReport::Unavailable
                };
                Event::Result {
                    output: Some(output),
                    usage,
                }
            }
        }
        "error" => Event::Failed(shared_cli::error_kind(&value["error"])),
        "tool_use" | "tool_result" => Event::ForbiddenTool,
        _ => Event::Unknown,
    })
}
impl Provider for ClaudeProvider {
    fn probe(&self) -> Result<Capabilities> {
        self.probe_cancelled(&AtomicBool::new(false))
    }
    fn probe_cancelled(&self, cancelled: &AtomicBool) -> Result<Capabilities> {
        let version = self.inspect(Some(cancelled))?;
        Ok(Capabilities {provider:"claude".into(),version,structured_output:true,streaming:true,resume:true,steering:false,cancellation:true,approvals:false,native_subagents_disabled:true,
            sandbox:"CLI tools disabled; safe-mode, no MCP/skills/hooks/plugins; OS process is not sandboxed; managed policy still applies".into(),usage:"CLI-reported token counts; cumulative estimated USD, not billing".into()})
    }
    fn probe_report(&self) -> Result<ProbeReport> {
        Ok(ProbeReport {
            capabilities: self.probe()?,
            history: Support::Supported,
            exact_id_resume: Support::Supported,
            mid_turn_steering: Support::Unsupported,
            approvals: Support::Unsupported,
            os_sandbox: Support::Unsupported,
            tool_free: Support::Supported,
            authentication: Support::Unverified,
        })
    }
    fn turn_receipt(
        &self,
        request: &TurnRequest<'_>,
        cancelled: Arc<AtomicBool>,
        events: &mut dyn FnMut(&ProviderEvent) -> Result<()>,
    ) -> Result<TurnReceipt> {
        self.inspect(Some(&cancelled))?;
        let session_id = shared_cli::exact_session("claude", request.resume)?;
        let mut command = shared_cli::command("claude", self.binary())?;
        command
            .args([
                "--safe-mode",
                "--strict-mcp-config",
                "--mcp-config",
                "{\"mcpServers\":{}}",
                "--tools",
                "",
                "--no-chrome",
                "--disable-slash-commands",
                "--permission-mode",
                "dontAsk",
                "--permission-prompts",
                "none",
                "--print",
                "--output-format",
                "stream-json",
                "--verbose",
                "--json-schema",
            ])
            .arg(serde_json::to_string(request.schema)?)
            .current_dir(&request.run.spec.workspace);
        command
            .arg(if request.resume.is_some() {
                "--resume"
            } else {
                "--session-id"
            })
            .arg(&session_id);
        if let Some(model) = self.model.as_ref().or(request.run.spec.model.as_ref()) {
            command.arg("--model").arg(model);
        }
        if let Some(effort) = &self.effort {
            command.arg("--effort").arg(effort);
        }
        if let Some(home) = &self.managed_home {
            crate::ipc::private_directory(home)?;
            command.env("CLAUDE_CONFIG_DIR", home);
        }
        shared_cli::run(
            "claude",
            &mut command,
            request,
            &session_id,
            cancelled,
            events,
            normalize,
        )
    }
    fn execute(
        &self,
        run: &Run,
        cancelled: Arc<AtomicBool>,
        session: &mut dyn FnMut(&str) -> Result<()>,
    ) -> Result<(String, String)> {
        prompts::execute(self, run, cancelled, session)
    }
    fn verify(
        &self,
        run: &Run,
        artifact: &Artifact,
        evidence: &Evidence,
        cancelled: Arc<AtomicBool>,
        session: &mut dyn FnMut(&str) -> Result<()>,
    ) -> Result<Review> {
        prompts::verify(self, run, artifact, evidence, cancelled, session)
    }
}
impl prompts::JsonTurn for ClaudeProvider {
    fn json_turn(
        &self,
        run: &Run,
        prompt: &str,
        schema: &Value,
        resume: Option<&str>,
        cancelled: Arc<AtomicBool>,
        session: &mut dyn FnMut(&str) -> Result<()>,
    ) -> Result<(String, String)> {
        let turn_id = crate::id();
        let receipt = self.turn_receipt(
            &TurnRequest {
                turn_id: &turn_id,
                run,
                prompt,
                schema,
                resume,
            },
            cancelled,
            &mut |event| {
                if let ProviderEvent::DeliveryPending { session_id, .. } = event {
                    session(session_id)?;
                }
                Ok(())
            },
        )?;
        Ok((receipt.session_id, receipt.output))
    }
}

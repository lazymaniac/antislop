//! Pinned Antigravity CLI transport. `init.tools` is the global registry, emitted
//! before executor resolution in 1.3.4. The owned agent selects only `finish`;
//! that component encodes structured output and has no filesystem/tool action.
//! Global startup customizations are audited before every probe and invocation.
use super::*;
mod context;
use crate::{config::IsolationProfile, process::SupervisedChild, require};
use serde_json::{Value, json};
use std::{
    io::{BufReader, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{atomic::Ordering, mpsc},
    time::Duration,
};

const VERSIONS: &[&str] = &["1.3.4", "1.3.5"];
const AGENT: &str = "antislop-stdio";
/// Observed CLI user-content ceiling. Reject before delivery instead of letting
/// native ingestion silently replace the tail with a truncation marker.
pub const MAX_PROMPT_BYTES: usize = 192_000;
const AGENT_SOURCE: &[u8] = b"---\nname: antislop-stdio\ndescription: Structured output transport without external tools.\ntools: [finish]\nexcludeDefaultComponents: true\ninheritCustomizations: false\ninheritMcp: false\nmainAgent: true\nsubagent: false\ncommandExecutionPolicy: \"off\"\nmcpServers: []\nskills: []\nplugins: []\n---\nReturn the requested structured result using finish. Never use any other tool or delegate.\n";
const FLAGS: &[&str] = &[
    "--input-format",
    "--output-format",
    "--json-schema",
    "--agent",
    "--disable-slash-commands",
    "--mode",
    "--sandbox",
    "--print-timeout",
    "--conversation",
    "--model",
    "--effort",
];

#[derive(Default)]
pub struct AgyProvider {
    pub binary: Option<String>,
    pub managed_home: Option<PathBuf>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub isolation: IsolationProfile,
}

fn policy(message: &str) -> crate::Error {
    failure("agy", FailureKind::Policy, message)
}
fn owned_metadata(path: &Path, directory: bool) -> Result<std::fs::Metadata> {
    let metadata = std::fs::symlink_metadata(path)?;
    require(
        !metadata.file_type().is_symlink()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.permissions().mode() & 0o022 == 0
            && if directory {
                metadata.is_dir()
            } else {
                metadata.is_file()
            },
        "unsafe Antigravity startup path ownership/type/permissions",
    )?;
    Ok(metadata)
}
fn optional_json(path: &Path) -> Result<Option<Value>> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
        Ok(_) => {
            let metadata = owned_metadata(path, false)?;
            require(
                metadata.len() <= 65_536,
                "Antigravity startup file is too large",
            )?;
            let bytes = std::fs::read(path)?;
            if bytes.is_empty() {
                return Ok(Some(json!({})));
            }
            Ok(Some(crate::transport::decode(&bytes)?))
        }
    }
}
fn absent_or_empty(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
        Ok(metadata) if metadata.is_dir() => {
            owned_metadata(path, true)?;
            require(
                std::fs::read_dir(path)?.next().is_none(),
                "global Antigravity startup customizations are unsupported",
            )
        }
        Ok(_) => require(
            optional_json(path)? == Some(json!({})),
            "global Antigravity hooks/MCP startup customizations are unsupported",
        ),
    }
}

impl AgyProvider {
    fn binary(&self) -> &str {
        self.binary.as_deref().unwrap_or("agy")
    }

    /// Read-only audit. In particular, never chmod or overwrite a user's home,
    /// settings, hooks, plugins or credentials to make a profile pass.
    fn home(&self) -> Result<PathBuf> {
        if !matches!(
            self.isolation,
            IsolationProfile::ToolFree | IsolationProfile::PinnedContext
        ) {
            return Err(failure(
                "agy",
                FailureKind::Unsupported,
                "only tool_free and scoped pinned_context profiles are implemented; OS sandbox guarantees are unavailable",
            ));
        }
        let home = self
            .managed_home
            .clone()
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .ok_or_else(|| policy("Antigravity CLI home is missing"))?;
        require(home.is_absolute(), "Antigravity home must be absolute")?;
        owned_metadata(&home, true)?;
        let home = home.canonicalize()?;
        for root in [
            home.join(".gemini"),
            home.join(".gemini/config"),
            home.join(".gemini/antigravity-cli"),
        ] {
            if root.exists() {
                owned_metadata(&root, true)?;
            }
        }
        let config = home.join(".gemini/config");
        let cli = home.join(".gemini/antigravity-cli");
        for path in [
            config.join("hooks.json"),
            config.join("mcp_config.json"),
            cli.join("hooks.json"),
            cli.join("mcp_config.json"),
        ] {
            absent_or_empty(&path)?;
        }
        for root in [&config, &cli] {
            for directory in ["plugins", "agents", "skills", "rules", "workflows"] {
                absent_or_empty(&root.join(directory))?;
            }
        }
        if let Some(value) = optional_json(&config.join("config.json"))? {
            let object = value
                .as_object()
                .ok_or_else(|| policy("Antigravity global config must be an object"))?;
            require(
                object.keys().all(|key| key == "userSettings"),
                "unsupported Antigravity global config",
            )?;
            if let Some(settings) = object.get("userSettings") {
                require(
                    settings.as_object().is_some_and(|settings| {
                        settings.iter().all(|(key, value)| {
                            key == "remoteControlHostname"
                                && value
                                    .as_str()
                                    .is_some_and(|s| s.len() <= 256 && !s.contains(['\n', '\r']))
                        })
                    }),
                    "unsupported Antigravity global customization",
                )?;
            }
        }
        if let Some(value) = optional_json(&cli.join("settings.json"))? {
            let object = value
                .as_object()
                .ok_or_else(|| policy("Antigravity settings must be an object"))?;
            for (key, value) in object {
                let safe = match key.as_str() {
                    "model" => value
                        .as_str()
                        .is_some_and(|s| s.len() <= 256 && !s.contains(['\n', '\r'])),
                    "modelProvider" => value == "gemini",
                    "showFeedbackSurvey"
                    | "enableTelemetry"
                    | "enableTerminalSandbox"
                    | "allowNonWorkspaceAccess" => value.is_boolean(),
                    // Native default UI status-line shape has no executable
                    // customization. Continue refusing any configured command.
                    "statusLine" => value.as_object().is_some_and(|settings| {
                        settings.len() == 3
                            && settings.get("type") == Some(&json!(""))
                            && settings.get("command") == Some(&json!(""))
                            && settings.get("enabled").is_some_and(Value::is_boolean)
                    }),
                    "toolPermission" => value.as_str().is_some_and(|s| {
                        matches!(
                            s,
                            "strict" | "request-review" | "proceed-in-sandbox" | "always-proceed"
                        )
                    }),
                    "verbosity" => value
                        .as_str()
                        .is_some_and(|s| matches!(s, "low" | "medium" | "high")),
                    "trustedWorkspaces" => value
                        .as_array()
                        .is_some_and(|a| a.iter().all(Value::is_string)),
                    "permissions" => {
                        value
                            == &json!({"allow":[],"deny":["read_file(*)","write_file(*)","command(*)","unsandboxed(*)","read_url(*)","execute_url(*)","mcp(*)"]})
                    }
                    _ => false,
                };
                require(
                    safe,
                    "unsupported Antigravity startup setting; refusing to modify user configuration",
                )?;
            }
        }
        Ok(home)
    }

    fn inspect(&self, home: &Path, cwd: &Path, cancelled: Option<&AtomicBool>) -> Result<String> {
        let version = shared_cli::capture(
            shared_cli::command("agy", self.binary())?
                .env("HOME", home)
                .current_dir(cwd)
                .arg("--version"),
            cancelled,
        )?;
        if !VERSIONS.contains(&version.trim()) {
            return Err(failure(
                "agy",
                FailureKind::Unsupported,
                "CLI version is outside the validated Antigravity 1.3.4/1.3.5 contract",
            ));
        }
        if self.isolation == IsolationProfile::PinnedContext && version.trim() != "1.3.5" {
            return Err(failure(
                "agy",
                FailureKind::Unsupported,
                "pinned_context requires the validated Antigravity 1.3.5 MCP contract; no profile downgrade",
            ));
        }
        let help = shared_cli::capture_help(
            shared_cli::command("agy", self.binary())?
                .env("HOME", home)
                .current_dir(cwd),
            cancelled,
        )?;
        require(
            FLAGS.iter().all(|flag| help.contains(flag)),
            "Antigravity CLI is missing required transport/restriction flags",
        )?;
        Ok(version.trim().into())
    }

    fn cwd(&self, workspace: &Path) -> Result<PathBuf> {
        let base =
            std::env::temp_dir().join(format!("architect-agy-{}", unsafe { libc::geteuid() }));
        crate::ipc::private_directory(&base)?;
        let workspace = workspace.canonicalize()?;
        let cwd = base.join(crate::hash(workspace.to_string_lossy().as_bytes()));
        crate::ipc::private_directory(&cwd)?;
        for ancestor in cwd.ancestors().skip(1) {
            require(
                ![".agents", ".gemini", ".env", "AGENTS.md", "GEMINI.md"]
                    .iter()
                    .any(|name| ancestor.join(name).exists()),
                "Antigravity controlled cwd ancestor contains startup customizations",
            )?;
        }
        let agents = cwd.join(".agents");
        crate::ipc::private_directory(&agents)?;
        let definitions = agents.join("agents");
        crate::ipc::private_directory(&definitions)?;
        controlled_file(&definitions.join("antislop-stdio.md"), AGENT_SOURCE)?;
        require(
            std::fs::read_dir(&cwd)?
                .all(|entry| entry.is_ok_and(|entry| entry.file_name() == ".agents")),
            "unexpected Antigravity controlled cwd content",
        )?;
        require(
            std::fs::read_dir(&agents)?
                .all(|entry| entry.is_ok_and(|entry| entry.file_name() == "agents")),
            "unexpected Antigravity local startup customization",
        )?;
        require(
            std::fs::read_dir(&definitions)?
                .all(|entry| entry.is_ok_and(|entry| entry.file_name() == "antislop-stdio.md")),
            "unexpected Antigravity agent definition",
        )?;
        Ok(cwd.canonicalize()?)
    }
}

fn controlled_file(path: &Path, bytes: &[u8]) -> Result<()> {
    if path.exists() {
        owned_metadata(path, false)?;
        require(
            std::fs::read(path)? == bytes,
            "Antigravity control profile changed; refusing to overwrite",
        )?;
    } else {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    Ok(())
}

#[derive(Debug)]
pub enum NativeEvent {
    Init {
        id: String,
        cwd: String,
        agent: String,
    },
    Step {
        id: String,
        finish: bool,
    },
    Result {
        id: String,
        output: Value,
        schema: Value,
        usage: UsageReport,
    },
    Failed {
        id: Option<String>,
        kind: FailureKind,
    },
}
fn identity(value: &Value) -> Result<String> {
    value
        .as_str()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .map(|id| id.to_string())
        .ok_or_else(|| {
            failure(
                "agy",
                FailureKind::InvalidSession,
                "native conversation identity is not a UUID",
            )
        })
}
// Only call for a native error_message step, never an agent's artifact text.
fn step_error(step: &Value) -> &Value {
    step.get("error_message")
        .or_else(|| step.get("errorMessage"))
        .or_else(|| step.get("error"))
        .unwrap_or(step)
}

// Never copy native text, arguments, arbitrary tool names or subagent bodies
// into diagnostics. Known protocol identifiers suffice to diagnose a rejection.
fn external_step_diagnostic(step: &Value) -> String {
    let tool = |value: &Value| match value.as_str() {
        Some("finish") => "finish",
        Some("call_mcp_tool") => "call_mcp_tool",
        Some("manage_task") => "manage_task",
        Some("invoke_subagent") => "invoke_subagent",
        Some("run_command") => "run_command",
        Some("write_to_file") => "write_to_file",
        Some("read_file") => "read_file",
        None => "absent",
        _ => "redacted",
    };
    let kind = match step["step_type"].as_str() {
        Some("tool") => "tool",
        Some("agent_response") => "agent_response",
        Some("user_input") => "user_input",
        Some("checkpoint") => "checkpoint",
        Some("finish") => "finish",
        Some("error_message") => "error_message",
        _ => "other",
    };
    format!(
        "external tool/subagent event violates the selected profile (step={kind}, tool={}, tool_info_name={}, tool_info={}, subagent_info={})",
        tool(&step["tool_name"]),
        tool(&step["tool_info"]["name"]),
        step.get("tool_info").is_some_and(|v| !v.is_null()),
        step.get("subagent_info").is_some_and(|v| !v.is_null()),
    )
}

fn unknown_step_diagnostic(step: &Value) -> String {
    // Native system-message names are useful for protocol diagnosis, but an
    // unrecognized category is never made admissible by its diagnostic label.
    let name = match step["step_type"].as_str() {
        Some("system_message") => "system_message",
        Some(_) => "redacted",
        None => "non-string",
    };
    let fingerprint = crate::hash(serde_json::to_vec(&step["step_type"]).unwrap_or_default());
    format!(
        "unknown native step type (step={name}, type_sha256={fingerprint}, text_delta={}, system_message={}, error_message={})",
        step.get("text_delta").is_some_and(|v| !v.is_null()),
        step.get("system_message").is_some_and(|v| !v.is_null()),
        step.get("error_message").is_some_and(|v| !v.is_null()),
    )
}

/// Ignore planner text/reasoning; only the native finish result is an artifact.
pub fn normalize(value: &Value) -> Result<NativeEvent> {
    match value["event"].as_str() {
        Some("init") => {
            require(
                value["init"]["tools"]
                    .as_array()
                    .is_some_and(|tools| tools.iter().all(Value::is_string)),
                "invalid Antigravity registry catalog",
            )?;
            Ok(NativeEvent::Init {
                id: identity(&value["conversation_id"])?,
                cwd: value["init"]["cwd"]
                    .as_str()
                    .ok_or_else(|| policy("init missing cwd"))?
                    .into(),
                agent: value["init"]["agent"]
                    .as_str()
                    .ok_or_else(|| policy("init missing agent"))?
                    .into(),
            })
        }
        Some("step_update") => {
            let step = &value["step_update"];
            if step.get("subagent_info").is_some_and(|v| !v.is_null())
                || step.get("tool_info").is_some_and(|v| !v.is_null())
                || step.get("tool_name").is_some_and(|v| !v.is_null())
                || step["step_type"] == "tool"
            {
                return Err(policy(&external_step_diagnostic(step)));
            }
            require(
                matches!(step["state"].as_str(), Some("ACTIVE" | "DONE")),
                "invalid Antigravity step state",
            )?;
            let id = identity(&step["conversation_id"])?;
            match step["step_type"].as_str() {
                Some("user_input" | "agent_response" | "checkpoint") => {
                    Ok(NativeEvent::Step { id, finish: false })
                }
                Some("finish") => Ok(NativeEvent::Step {
                    id,
                    finish: step["state"] == "DONE",
                }),
                Some("error_message") => Ok(NativeEvent::Failed {
                    id: Some(id),
                    kind: shared_cli::error_kind(step_error(step)),
                }),
                _ => Err(failure(
                    "agy",
                    FailureKind::Protocol,
                    &unknown_step_diagnostic(step),
                )),
            }
        }
        Some("result") => {
            let result = &value["result"];
            if result["status"] != "SUCCESS" {
                return Ok(NativeEvent::Failed {
                    id: result["conversation_id"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .map(|_| identity(&result["conversation_id"]))
                        .transpose()?,
                    kind: shared_cli::error_kind(&result["error"]),
                });
            }
            let output = result
                .get("structured_output")
                .filter(|v| v.is_object())
                .ok_or_else(|| {
                    failure(
                        "agy",
                        FailureKind::Protocol,
                        "SUCCESS lacks structured finish output",
                    )
                })?
                .clone();
            let schema = result
                .get("json_schema")
                .filter(|v| v.is_object())
                .ok_or_else(|| {
                    failure(
                        "agy",
                        FailureKind::Protocol,
                        "SUCCESS lacks enforced schema",
                    )
                })?
                .clone();
            let usage = result
                .get("usage")
                .map_or(UsageReport::Unavailable, |usage| UsageReport::Reported {
                    input_tokens: usage["input_tokens"].as_u64(),
                    output_tokens: usage["output_tokens"].as_u64(),
                    cached_tokens: usage["cache_read_tokens"].as_u64(),
                    estimated_usd: None,
                    cumulative: true,
                });
            Ok(NativeEvent::Result {
                id: identity(&result["conversation_id"])?,
                output,
                schema,
                usage,
            })
        }
        _ => Err(failure(
            "agy",
            FailureKind::Protocol,
            "unknown native event",
        )),
    }
}

impl Provider for AgyProvider {
    fn probe(&self) -> Result<Capabilities> {
        self.probe_cancelled(&AtomicBool::new(false))
    }
    fn probe_cancelled(&self, cancelled: &AtomicBool) -> Result<Capabilities> {
        let home = self.home()?;
        let cwd = self.cwd(&home)?;
        Ok(Capabilities { provider: "agy".into(), version: self.inspect(&home,&cwd,Some(cancelled))?,
            structured_output: true, streaming: true, resume: false, steering: false, cancellation: true,
            approvals: false, native_subagents_disabled: true,
            sandbox: if self.isolation==IsolationProfile::PinnedContext {"CLI finish plus owned immutable-context reader; audited startup; no OS containment claim"}else{"CLI finish-only agent; audited startup; process-group cleanup; no OS containment claim"}.into(),
            usage: "native cumulative session counters; terminal result only".into() })
    }
    fn probe_report(&self) -> Result<ProbeReport> {
        Ok(ProbeReport {
            capabilities: self.probe()?,
            history: Support::Unverified,
            exact_id_resume: Support::Unsupported,
            mid_turn_steering: Support::Unsupported,
            approvals: Support::Unsupported,
            os_sandbox: Support::Unverified,
            tool_free: if self.isolation == IsolationProfile::ToolFree {
                Support::Supported
            } else {
                Support::Unsupported
            },
            authentication: Support::Unverified,
        })
    }
    fn turn_receipt(
        &self,
        request: &TurnRequest<'_>,
        cancelled: Arc<AtomicBool>,
        events: &mut dyn FnMut(&ProviderEvent) -> Result<()>,
    ) -> Result<TurnReceipt> {
        require(
            !request.turn_id.trim().is_empty(),
            "missing durable turn ID",
        )?;
        if request.prompt.len() > MAX_PROMPT_BYTES
            && self.isolation != IsolationProfile::PinnedContext
        {
            return Err(failure(
                "agy",
                FailureKind::Protocol,
                "input_too_large: complete prompt exceeds the observed 192000-byte native user-message ceiling; no native turn was delivered; compact or partition the context without dropping evidence",
            ));
        }
        shared_cli::require_prompt_size("agy", request.prompt)?;
        let resume = request
            .resume
            .map(|s| identity(&Value::String(s.into())))
            .transpose()?;
        if resume.is_some() {
            return Err(failure(
                "agy",
                FailureKind::Unsupported,
                "native exact-ID resume emitted an unvalidated protocol step in 1.3.4; resume is disabled without fallback",
            ));
        }
        let home = self.home()?;
        let probe_cwd = self.cwd(Path::new(&request.run.spec.workspace))?;
        self.inspect(&home, &probe_cwd, Some(&cancelled))?;
        let mut context = if self.isolation == IsolationProfile::PinnedContext {
            Some(crate::context::transport::Transport::new(request)?)
        } else {
            None
        };
        let bridge = context
            .as_ref()
            .map(|c| c.reader().and_then(crate::context::bridge::Bridge::start))
            .transpose()?;
        let cwd = if let Some(bridge) = &bridge {
            context::cwd(bridge)?
        } else {
            probe_cwd
        };
        let native_prompt = context
            .as_ref()
            .map(|c| c.native_prompt())
            .transpose()?
            .unwrap_or_else(|| request.prompt.into());
        require(
            native_prompt.len() <= MAX_PROMPT_BYTES,
            "context transport native input exceeds validated ceiling",
        )?;
        if let Some(effort) = &self.effort {
            require(
                matches!(effort.as_str(), "low" | "medium" | "high" | "xhigh" | "max"),
                "unsupported Antigravity effort",
            )?;
        }
        let schema_path =
            std::env::temp_dir().join(format!("architect-agy-schema-{}.json", crate::id()));
        controlled_file(&schema_path, &serde_json::to_vec(request.schema)?)?;
        struct SchemaFile(PathBuf);
        impl Drop for SchemaFile {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let schema_file = SchemaFile(schema_path);
        let mut command = shared_cli::command("agy", self.binary())?;
        command
            .args([
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--json-schema",
            ])
            .arg(&schema_file.0)
            .args([
                "--agent",
                AGENT,
                "--disable-slash-commands",
                "--mode",
                "accept-edits",
                "--sandbox",
                "--print-timeout",
                "0",
            ])
            .env("HOME", home)
            .current_dir(&cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(model) = self.model.as_ref().or(request.run.spec.model.as_ref()) {
            command.arg("--model").arg(model);
        }
        if let Some(effort) = &self.effort {
            command.arg("--effort").arg(effort);
        }
        if let Some(resume) = &resume {
            command.arg("--conversation").arg(resume);
        }
        if cancelled.load(Ordering::SeqCst) {
            return Err(failure(
                "agy",
                FailureKind::Cancelled,
                "turn cancelled before start",
            ));
        }
        if let Some(context) = &context {
            events(&ProviderEvent::ContextPrepared {
                turn_id: request.turn_id.into(),
                transport: Box::new(context.clone()),
            })?;
        }
        if cancelled.load(Ordering::SeqCst) {
            return Err(failure(
                "agy",
                FailureKind::Cancelled,
                "turn cancelled after context preparation; native delivery was not started",
            ));
        }
        events(&match &resume {
            Some(id) => ProviderEvent::DeliveryPending {
                turn_id: request.turn_id.into(),
                session_id: id.clone(),
            },
            None => ProviderEvent::DeliveryPendingUnknown {
                turn_id: request.turn_id.into(),
            },
        })?;
        let mut child = SupervisedChild::spawn(&mut command)?;
        let stderr = child.child.stderr.take().expect("stderr pipe");
        let diagnostics = std::thread::spawn(move || shared_cli::classify_stderr(stderr));
        let stdout = child.child.stdout.take().expect("stdout pipe");
        let (sender, receiver) = mpsc::sync_channel(16);
        let reader = std::thread::spawn(move || {
            let mut input = BufReader::new(stdout);
            loop {
                let frame = crate::transport::read::<Value>(&mut input);
                let end = matches!(frame, Ok(None) | Err(_));
                if sender.send(frame).is_err() || end {
                    break;
                }
            }
        });
        let mut stdin = child.child.stdin.take().expect("stdin pipe");
        let mut input =
            serde_json::to_vec(&json!({"event":"user","message":{"content":native_prompt}}))?;
        input.push(b'\n');
        let (sent_sender, sent_receiver) = mpsc::sync_channel(1);
        let writer = std::thread::spawn(move || {
            let result = stdin.write_all(&input).and_then(|_| stdin.flush());
            drop(stdin);
            let _ = sent_sender.send(result);
        });
        let mut native = None::<String>;
        let mut sent = false;
        let mut write_finished = false;
        let mut write_failed = false;
        let mut eof = false;
        let mut status = None;
        let mut finish = false;
        let mut terminal = false;
        let mut output = None;
        let mut usage = UsageReport::Unavailable;
        let mut failed = None;
        let mut native_diagnosis = None;
        let mut context_tools = context::Tools::default();
        #[cfg(test)]
        let mut context_trace = std::env::var_os("ANTISLOP_CONTEXT_TOOL_TRACE")
            .map(|name| {
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .custom_flags(libc::O_NOFOLLOW)
                    .open(name)
            })
            .transpose()?;
        let stream_result = (|| -> Result<()> {
            while !eof || status.is_none() || !write_finished {
                if cancelled.load(Ordering::SeqCst) {
                    let callback = events(&ProviderEvent::CancellationRequested {
                        turn_id: request.turn_id.into(),
                    });
                    child.cancel()?;
                    callback?;
                    events(&ProviderEvent::Stopped {
                        turn_id: request.turn_id.into(),
                    })?;
                    return Err(ProviderFailure {
                        provider: "agy".into(),
                        kind: FailureKind::Cancelled,
                        message: "process group stopped; prior effects require reconciliation"
                            .into(),
                        session_id: native.clone(),
                        stopped: true,
                        partial_effects: sent || write_failed || !write_finished,
                    }
                    .into());
                }
                if sent
                    && native.is_some()
                    && let (Some(bridge), Some(context)) = (&bridge, &mut context)
                {
                    while let Some(commit) = bridge.pending() {
                        let result = (|| {
                            require(
                                sent && native.is_some(),
                                "context response before native delivery",
                            )?;
                            crate::context::transport::authenticate_receipt(
                                &context.reader()?,
                                &commit.receipt,
                            )?;
                            events(&ProviderEvent::ContextRead {
                                turn_id: request.turn_id.into(),
                                receipt: commit.receipt.clone(),
                            })?;
                            context.accept(commit.receipt.clone())
                        })();
                        commit.acknowledge(result.is_ok());
                        result?;
                    }
                }
                if !write_finished {
                    match sent_receiver.try_recv() {
                        Ok(Ok(())) => {
                            write_finished = true;
                            sent = true;
                            events(&ProviderEvent::DeliverySent {
                                turn_id: request.turn_id.into(),
                            })?;
                        }
                        Ok(Err(_)) | Err(mpsc::TryRecvError::Disconnected) => {
                            write_finished = true;
                            write_failed = true;
                        }
                        Err(mpsc::TryRecvError::Empty) => (),
                    }
                }
                status = status.or(child.child.try_wait()?);
                if eof {
                    std::thread::sleep(Duration::from_millis(25));
                    continue;
                }
                let frame = match receiver.recv_timeout(Duration::from_millis(25)) {
                    Ok(Ok(Some(v))) => v,
                    Ok(Ok(None)) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                        eof = true;
                        continue;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Ok(Err(_)) => {
                        return Err(failure(
                            "agy",
                            FailureKind::Protocol,
                            "malformed, duplicate-key or oversized CLI frame",
                        ));
                    }
                };
                require(!terminal, "native event after terminal result")?;
                #[cfg(test)]
                if frame["event"] == "step_update"
                    && frame["step_update"]["tool_name"] == "call_mcp_tool"
                    && let Some(trace) = &mut context_trace
                {
                    serde_json::to_writer(&mut *trace, &frame)?;
                    trace.write_all(b"\n")?;
                    trace.flush()?;
                }
                if let Some(context) = &mut context
                    && context_tools.frame(
                        &frame,
                        context,
                        native.as_deref(),
                        request.turn_id,
                        events,
                    )?
                {
                    continue;
                }
                match normalize(&frame)? {
                    NativeEvent::Init {
                        id,
                        cwd: reported,
                        agent,
                    } => {
                        require(
                            native.is_none(),
                            "duplicate native conversation initialization",
                        )?;
                        require(
                            resume.as_ref().is_none_or(|expected| expected == &id),
                            "CLI returned another conversation than requested",
                        )?;
                        require(
                            agent == AGENT && Path::new(&reported) == cwd,
                            "CLI did not select the owned finish-only profile/cwd",
                        )?;
                        events(&ProviderEvent::SessionObserved {
                            session_id: id.clone(),
                        })?;
                        native = Some(id);
                    }
                    NativeEvent::Step { id, finish: done } => {
                        require(
                            native.as_ref() == Some(&id),
                            "step refers to another or unobserved conversation",
                        )?;
                        finish |= done;
                    }
                    NativeEvent::Result {
                        id,
                        output: result,
                        schema,
                        usage: reported,
                    } => {
                        require(
                            native.as_ref() == Some(&id) && finish,
                            "result lacks observed conversation and completed finish",
                        )?;
                        require(
                            schema == *request.schema,
                            "native result schema differs from requested schema",
                        )?;
                        if let Some(context) = &context
                            && let Err(error) = context.complete()
                        {
                            events(&ProviderEvent::OutputRejected {
                                turn_id: request.turn_id.into(),
                                output: result.to_string(),
                            })?;
                            return Err(error);
                        }
                        terminal = true;
                        output = Some(result.to_string());
                        usage = reported;
                    }
                    NativeEvent::Failed { id, kind } => {
                        require(
                            id.as_ref().is_none_or(|id| native.as_ref() == Some(id)),
                            "failure refers to another conversation",
                        )?;
                        if frame["event"] == "step_update" {
                            let diagnosis =
                                shared_cli::diagnostic_category(step_error(&frame["step_update"]))
                                    .map(|category| category.classification().1)
                                    .unwrap_or("unknown");
                            // A reported failed native step already makes this turn
                            // inadmissible. Stop its internal retries immediately.
                            // The outer error path reaps the owned process group.
                            return Err(ProviderFailure {
                                provider: "agy".into(),
                                kind,
                                message: format!("Native CLI reported a failed step; diagnosis={diagnosis}; internal retries stopped; raw diagnostics not retained"),
                                session_id: native.clone(),
                                stopped: false,
                                partial_effects: sent,
                            }.into());
                        }
                        terminal = frame["event"] == "result";
                        failed = Some(kind);
                        native_diagnosis = shared_cli::diagnostic_category(&frame["result"]);
                    }
                }
            }
            Ok(())
        })();
        if let Err(error) = stream_result {
            child.cancel()?;
            let mut error = match error {
                crate::Error::Provider(error) => error,
                _ => ProviderFailure {
                    provider: "agy".into(),
                    kind: FailureKind::Protocol,
                    message: "native stream ordering, structure or acknowledgement failed".into(),
                    session_id: None,
                    stopped: true,
                    partial_effects: false,
                },
            };
            let observed_native = native.is_some();
            error.session_id = error.session_id.or(native);
            error.stopped = true;
            error.partial_effects |= sent || write_failed || !write_finished || observed_native;
            return Err(error.into());
        }
        let _ = writer.join();
        let _ = reader.join();
        let diagnosis = native_diagnosis.or(diagnostics.join().ok().flatten());
        let status = status.expect("observed guardian exit");
        if !status.success() || !terminal || failed.is_some() || write_failed || output.is_none() {
            return Err(shared_cli::exit_failure(
                "agy",
                status,
                diagnosis,
                failed,
                native,
                sent || write_failed,
            ));
        }
        let session_id = native.expect("successful result requires init");
        events(&ProviderEvent::Usage {
            report: usage.clone(),
        })?;
        events(&ProviderEvent::Completed {
            turn_id: request.turn_id.into(),
        })?;
        Ok(TurnReceipt {
            turn_id: request.turn_id.into(),
            provider: "agy".into(),
            session_id,
            delivered: sent,
            completed: true,
            stopped: true,
            output: output.expect("validated finish result"),
            usage,
        })
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
impl prompts::JsonTurn for AgyProvider {
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
                if let ProviderEvent::SessionObserved { session_id } = event {
                    session(session_id)?;
                }
                Ok(())
            },
        )?;
        Ok((receipt.session_id, receipt.output))
    }
}

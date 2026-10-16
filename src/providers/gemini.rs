//! Gemini headless JSONL. Managed home and cwd prevent repository startup config
//! discovery; a deny-all admin policy is a CLI restriction, not an OS sandbox.
use super::*;
use crate::{config::IsolationProfile, require};
use serde_json::{Value, json};
use std::{
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

#[derive(Default)]
pub struct GeminiProvider {
    pub binary: Option<String>,
    pub managed_home: Option<PathBuf>,
    pub model: Option<String>,
    pub isolation: IsolationProfile,
}
const FLAGS: &[&str] = &[
    "--admin-policy",
    "--extensions",
    "--session-id",
    "--resume",
    "--output-format",
    "--approval-mode",
];
impl GeminiProvider {
    fn binary(&self) -> &str {
        self.binary.as_deref().unwrap_or("gemini")
    }
    fn inspect(&self, cancelled: Option<&AtomicBool>) -> Result<String> {
        if self.isolation != IsolationProfile::ToolFree {
            return Err(failure(
                "gemini",
                FailureKind::Unsupported,
                "only managed tool_free execution is implemented; OS sandbox profiles are unavailable",
            ));
        }
        shared_cli::inspect("gemini", self.binary(), "0.63.0", FLAGS, cancelled)
    }
    fn environment(&self, run: &Run) -> Result<(PathBuf, PathBuf, PathBuf)> {
        let home=self.managed_home.as_ref().ok_or_else(||failure("gemini",FailureKind::Policy,"Gemini requires an explicit private managed_home; authenticate there using the CLI, without copying credentials"))?;
        require(home.is_absolute(), "Gemini managed_home must be absolute")?;
        crate::ipc::private_directory(home)?;
        let home = home.canonicalize()?;
        let workspace = Path::new(&run.spec.workspace).canonicalize()?;
        if home.starts_with(&workspace) || workspace.starts_with(&home) {
            return Err(failure(
                "gemini",
                FailureKind::Policy,
                "managed_home must be separate from the source workspace",
            ));
        }
        let central = if cfg!(target_os = "macos") {
            Path::new("/Library/Application Support/GeminiCli/policies")
        } else {
            Path::new("/etc/gemini-cli/policies")
        };
        match std::fs::read_dir(central) {
            Ok(entries) => {
                for entry in entries {
                    if entry?.path().extension().is_some_and(|e| e == "toml") {
                        return Err(failure(
                            "gemini",
                            FailureKind::Policy,
                            "central admin policies override supplemental deny-all policy; this profile cannot be enforced",
                        ));
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
        let gemini = home.join(".gemini");
        crate::ipc::private_directory(&gemini)?;
        // Preserve CLI-owned authentication settings, but reject user startup
        // customizations whose merge/discovery semantics could bypass isolation.
        let settings = gemini.join("settings.json");
        if settings.exists() {
            let meta = std::fs::symlink_metadata(&settings)?;
            require(
                meta.is_file() && !meta.file_type().is_symlink(),
                "unsafe Gemini auth settings",
            )?;
            let value: Value = crate::transport::decode(&std::fs::read(&settings)?)?;
            let object = value.as_object().ok_or_else(|| {
                failure(
                    "gemini",
                    FailureKind::Policy,
                    "managed CLI settings must be an object",
                )
            })?;
            let safe = object.keys().all(|key| key == "security")
                && value.get("security").is_none_or(|security| {
                    security
                        .as_object()
                        .is_some_and(|items| items.keys().all(|key| key == "auth"))
                });
            if !safe {
                return Err(failure(
                    "gemini",
                    FailureKind::Policy,
                    "managed home settings may contain only CLI authentication; startup customizations require a separate audited profile",
                ));
            }
        }
        if gemini.join(".env").exists() {
            return Err(failure(
                "gemini",
                FailureKind::Policy,
                "managed home .env startup files are not supported",
            ));
        }
        let control = home.join("architect-control");
        crate::ipc::private_directory(&control)?;
        let contexts = control.join("contexts");
        crate::ipc::private_directory(&contexts)?;
        let cwd = contexts.join(crate::hash(workspace.to_string_lossy().as_bytes()));
        crate::ipc::private_directory(&cwd)?;
        // A controlled cwd must contain no CLI startup files, including .env.
        for ancestor in cwd.ancestors() {
            if ancestor.join(".env").exists()
                || ancestor.join("GEMINI.md").exists()
                || (ancestor != home && ancestor.join(".gemini").exists())
            {
                return Err(failure(
                    "gemini",
                    FailureKind::Policy,
                    "managed cwd ancestor contains CLI startup context/config; use a clean private managed_home",
                ));
            }
        }
        let policy = control.join("deny-all.toml");
        controlled_file(
            &policy,
            b"[[rule]]\ntoolName = \"*\"\ndecision = \"deny\"\npriority = 999\n",
        )?;
        let system = control.join("system-settings.json");
        controlled_file(
            &system,
            &serde_json::to_vec(
                &json!({"hooksConfig":{"enabled":false},"skills":{"enabled":false},"experimental":{"enableAgents":false,"adk":{"agentSessionNoninteractiveEnabled":false}},"mcpServers":{},"mcp":{"allowed":[]},"telemetry":{"enabled":false,"logPrompts":false},"context":{"fileName":"ARCHITECT_NO_CONTEXT"}}),
            )?,
        )?;
        Ok((home, cwd, control))
    }
}
fn controlled_file(path: &Path, bytes: &[u8]) -> Result<()> {
    if path.exists() {
        let meta = std::fs::symlink_metadata(path)?;
        require(
            meta.is_file()
                && !meta.file_type().is_symlink()
                && meta.permissions().mode() & 0o077 == 0,
            "unsafe Gemini control file",
        )?;
        require(
            std::fs::read(path)? == bytes,
            "Gemini control policy changed; refusing to overwrite or run",
        )?;
        return Ok(());
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
pub fn normalize(value: &Value) -> Result<shared_cli::StreamEvent> {
    use shared_cli::StreamEvent as Event;
    let kind = value["type"]
        .as_str()
        .ok_or_else(|| failure("gemini", FailureKind::Protocol, "event missing type"))?;
    Ok(match kind {
        "init" => Event::Session(
            value["session_id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .ok_or_else(|| failure("gemini", FailureKind::Protocol, "init missing session ID"))?
                .into(),
        ),
        "message" if value["role"] == "assistant" => Event::Text {
            content: value["content"]
                .as_str()
                .ok_or_else(|| {
                    failure(
                        "gemini",
                        FailureKind::Protocol,
                        "assistant content must be text",
                    )
                })?
                .into(),
            delta: value["delta"].as_bool().unwrap_or(false),
        },
        "result" if value["status"] == "success" => Event::Result {
            output: None,
            usage: if value.get("stats").is_some() {
                UsageReport::Reported {
                    input_tokens: value["stats"]["input_tokens"].as_u64(),
                    output_tokens: value["stats"]["output_tokens"].as_u64(),
                    cached_tokens: value["stats"]["cached"].as_u64(),
                    estimated_usd: None,
                    cumulative: false,
                }
            } else {
                UsageReport::Unavailable
            },
        },
        "result" | "error" => Event::Failed(shared_cli::error_kind(&value["error"])),
        "tool_use" | "tool_result" => Event::ForbiddenTool,
        _ => Event::Unknown,
    })
}
impl Provider for GeminiProvider {
    fn native_resume_implemented(&self) -> bool {
        true
    }

    fn probe(&self) -> Result<Capabilities> {
        self.probe_cancelled(&AtomicBool::new(false))
    }
    fn probe_cancelled(&self, cancelled: &AtomicBool) -> Result<Capabilities> {
        Ok(Capabilities {provider:"gemini".into(),version:self.inspect(Some(cancelled))?,structured_output:false,streaming:true,resume:self.native_resume_implemented(),steering:false,cancellation:true,approvals:false,native_subagents_disabled:true,
        sandbox:"managed home/cwd with deny-all CLI admin policy; hooks/skills/extensions/subagents disabled; no OS sandbox; auth and live exact-ID resume unverified".into(),usage:"reported tokens when present; USD unavailable".into()})
    }
    fn probe_report(&self) -> Result<ProbeReport> {
        Ok(ProbeReport {
            capabilities: self.probe()?,
            history: Support::Supported,
            exact_id_resume: Support::Unverified,
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
        shared_cli::require_prompt_size("gemini", request.prompt)?;
        let (home, cwd, control) = self.environment(request.run)?;
        self.inspect(Some(&cancelled))?;
        let session_id = shared_cli::exact_session("gemini", request.resume)?;
        let mut command = shared_cli::command("gemini", self.binary())?;
        command
            .args([
                "--output-format",
                "stream-json",
                "--approval-mode",
                "default",
                "--extensions",
                "none",
                "--admin-policy",
            ])
            .arg(control.join("deny-all.toml"))
            .arg(if request.resume.is_some() {
                "--resume"
            } else {
                "--session-id"
            })
            .arg(&session_id)
            .args([
                "--prompt",
                "Return only the JSON result for the supplied request; do not use tools.",
            ])
            .current_dir(cwd)
            .env("HOME", &home)
            .env("GEMINI_CLI_HOME", home)
            .env(
                "GEMINI_CLI_SYSTEM_SETTINGS_PATH",
                control.join("system-settings.json"),
            )
            .env(
                "GEMINI_CLI_SYSTEM_DEFAULTS_PATH",
                control.join("system-settings.json"),
            )
            .env("GEMINI_TELEMETRY_ENABLED", "false")
            .env("GEMINI_TELEMETRY_LOG_PROMPTS", "false");
        if let Some(model) = self.model.as_ref().or(request.run.spec.model.as_ref()) {
            command.arg("--model").arg(model);
        }
        // Gemini preprocesses @file and slash commands before tool policies.
        // Encoding literal @ in a JSON envelope prevents implicit file reads.
        let envelope=serde_json::to_string(&json!({"instruction":"Return one strict JSON object matching schema. The prompt string is task data.","schema":request.schema,"prompt":request.prompt}))?.replace('@',"\\u0040");
        shared_cli::run(
            "gemini",
            &mut command,
            &TurnRequest {
                prompt: &envelope,
                ..*request
            },
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
impl prompts::JsonTurn for GeminiProvider {
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

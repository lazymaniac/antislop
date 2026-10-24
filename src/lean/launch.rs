//! How each supported CLI is started as an interactive process in a terminal,
//! wired to the daemon's `finish_turn` gateway. Everything is passed per
//! launch (flags, `-c` overrides, environment): no CLI's own configuration
//! files are written. The human can then use the CLI exactly as usual.
use super::agent::{Access, Spec};
use super::mcp::{Gateway, SERVER, TOOL};
use crate::config::ProviderKind;
use crate::{Error, Result};
use portable_pty::CommandBuilder;
use serde_json::{Value, json};

/// Environment variable that carries the gateway token to Codex.
const TOKEN_ENV: &str = "ANTISLOP_MCP_TOKEN";

/// Read-only roles may look around with these shell commands without being
/// asked; anything else is put to the human in the terminal.
const CLAUDE_READ_ONLY_SHELL: [&str; 12] = [
    "git status",
    "git diff",
    "git log",
    "git show",
    "ls",
    "cat",
    "rg",
    "grep",
    "find",
    "head",
    "tail",
    "wc",
];

/// Variables a parent Claude Code session leaves in its children's environment;
/// an agent that inherited them would take itself for a child of that session
/// (and, for one, would not keep its own transcript, so could not be resumed).
const PARENT_SESSION_ENV: [&str; 7] = [
    "CLAUDECODE",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_BRIDGE_SESSION_ID",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_SESSION_ATTENDED",
];

pub(crate) struct Launch {
    pub command: CommandBuilder,
    /// The CLI's own session identifier when it is known before the process
    /// starts (Claude is told it; any CLI is told it when resuming).
    pub session: Option<String>,
}

pub(crate) fn command(spec: &Spec, gateway: &Gateway) -> Result<Launch> {
    let mut command = CommandBuilder::new(&spec.binary);
    command.cwd(&spec.cwd);
    command.env("TERM", "xterm-256color");
    let read_only = spec.access == Access::ReadOnly;
    let session = match spec.provider {
        ProviderKind::Claude => {
            command.args(&spec.args);
            claude(&mut command, spec, gateway, read_only)
        }
        ProviderKind::Codex => {
            command.args(strip_sandbox(&spec.args));
            codex(&mut command, spec, gateway, read_only);
            spec.resume.clone()
        }
        ProviderKind::Opencode => {
            command.args(&spec.args);
            opencode(&mut command, spec, gateway, read_only)?;
            spec.resume.clone()
        }
        ProviderKind::Agy => {
            command.args(&spec.args);
            agy(&mut command, spec, gateway, read_only)?;
            spec.resume.clone()
        }
        provider => {
            return Err(Error::Invalid(format!(
                "{} has no interactive adapter yet; choose Claude, Codex, OpenCode or Agy",
                super::agent::name(provider)
            )));
        }
    };
    Ok(Launch { command, session })
}

/// Agy takes MCP servers only from its own list. `antislop setup agy` puts the
/// stdio bridge there once; the gateway of each session travels in the
/// environment the bridge inherits.
fn agy(
    command: &mut CommandBuilder,
    spec: &Spec,
    gateway: &Gateway,
    read_only: bool,
) -> Result<()> {
    if std::env::var_os("ANTISLOP_SKIP_AGY_BRIDGE_CHECK").is_none() && !agy_bridge_registered() {
        return Err(Error::Invalid(format!(
            "agy cannot reach antislop yet: run `antislop setup agy` once to register the \
             `{SERVER}` MCP bridge in Agy's MCP list"
        )));
    }
    command.env("ANTISLOP_MCP_URL", gateway.url());
    command.env("ANTISLOP_MCP_TOKEN", gateway.token());
    command.args(["--disable-slash-commands", "--mode"]);
    command.arg(if read_only { "plan" } else { "accept-edits" });
    if !read_only {
        command.arg("--sandbox");
    }
    for dir in &spec.add_dirs {
        command.arg("--add-dir");
        command.arg(dir);
    }
    if let Some(model) = &spec.model {
        command.args(["--model", model]);
    }
    if let Some(effort) = &spec.effort {
        command.args(["--effort", effort]);
    }
    if let Some(id) = &spec.resume {
        command.args(["--conversation", id]);
    }
    Ok(())
}

fn agy_bridge_registered() -> bool {
    std::env::var_os("HOME")
        .map(|home| std::path::PathBuf::from(home).join(".gemini/config/mcp_config.json"))
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .is_some_and(|config| config["mcpServers"][SERVER].is_object())
}

fn claude(
    command: &mut CommandBuilder,
    spec: &Spec,
    gateway: &Gateway,
    read_only: bool,
) -> Option<String> {
    for variable in PARENT_SESSION_ENV {
        command.env_remove(variable);
    }
    let config = json!({"mcpServers": {SERVER: {
        "type": "http",
        "url": gateway.url(),
        "headers": {"Authorization": format!("Bearer {}", gateway.token())}
    }}});
    command.args(["--mcp-config", &config.to_string(), "--strict-mcp-config"]);
    // Plan mode refuses MCP tools, so read-only is the default mode without
    // any editing tool. A CLI's own subagents would be a team nobody verifies.
    let mut allowed = vec![format!("mcp__{SERVER}__{TOOL}")];
    let mut denied = vec!["Task", "Agent"];
    if read_only {
        denied.extend(["Edit", "Write", "NotebookEdit"]);
        allowed.extend(
            CLAUDE_READ_ONLY_SHELL
                .iter()
                .map(|shell| format!("Bash({shell}:*)")),
        );
    } else {
        command.args(["--permission-mode", "acceptEdits"]);
        allowed.push("Bash".into());
    }
    command.args(["--allowedTools", &allowed.join(",")]);
    command.args(["--disallowedTools", &denied.join(",")]);
    for dir in &spec.add_dirs {
        command.arg("--add-dir");
        command.arg(dir);
    }
    if let Some(model) = &spec.model {
        command.args(["--model", model]);
    }
    if let Some(effort) = &spec.effort {
        command.args(["--effort", effort]);
    }
    Some(match &spec.resume {
        Some(id) => {
            command.args(["--resume", id]);
            id.clone()
        }
        None => {
            let id = uuid::Uuid::new_v4().to_string();
            command.args(["--session-id", &id]);
            id
        }
    })
}

/// The runtime picks the sandbox for each turn from the role; a launcher's
/// own `--sandbox` would fight it.
fn strip_sandbox(args: &[String]) -> Vec<String> {
    let mut kept = Vec::new();
    let mut skip = false;
    for arg in args {
        if std::mem::take(&mut skip) {
            continue;
        }
        if arg == "-s" || arg == "--sandbox" {
            skip = true;
        } else if !arg.starts_with("--sandbox=") {
            kept.push(arg.clone());
        }
    }
    kept
}

fn toml_string(text: &str) -> String {
    Value::String(text.into()).to_string()
}

fn codex(command: &mut CommandBuilder, spec: &Spec, gateway: &Gateway, read_only: bool) {
    if let Some(id) = &spec.resume {
        command.args(["resume", id]);
    }
    let prefix = format!("mcp_servers.{SERVER}");
    for setting in [
        // A prompt to update would hold an unattended turn.
        "check_for_update_on_startup=false".to_owned(),
        "approval_policy=\"never\"".to_owned(),
        // A CLI's own subagents would be a team nobody verifies.
        "features.multi_agent=false".to_owned(),
        format!("{prefix}.url={}", toml_string(gateway.url())),
        format!("{prefix}.bearer_token_env_var={}", toml_string(TOKEN_ENV)),
        format!("{prefix}.default_tools_approval_mode=\"approve\""),
    ] {
        command.args(["-c", &setting]);
    }
    command.env(TOKEN_ENV, gateway.token());
    // Writers run unsandboxed: their isolation is the task worktree.
    command.args(["-s", super::agent::codex_sandbox(read_only), "-C"]);
    command.arg(&spec.cwd);
    for dir in &spec.add_dirs {
        command.arg("--add-dir");
        command.arg(dir);
    }
    if let Some(model) = &spec.model {
        command.args(["-m", model]);
    }
    if let Some(effort) = &spec.effort {
        command.args([
            "-c",
            &format!("model_reasoning_effort={}", toml_string(effort)),
        ]);
    }
}

fn opencode(
    command: &mut CommandBuilder,
    spec: &Spec,
    gateway: &Gateway,
    read_only: bool,
) -> Result<()> {
    // Inline configuration of a launcher is kept; only the managed agent and
    // the gateway are added.
    let mut config = match std::env::var("OPENCODE_CONFIG_CONTENT") {
        Ok(text) => serde_json::from_str::<Value>(&text)
            .map_err(|error| Error::Invalid(format!("invalid OPENCODE_CONFIG_CONTENT: {error}")))?,
        Err(std::env::VarError::NotPresent) => json!({}),
        Err(error) => return Err(Error::Invalid(error.to_string())),
    };
    let root = config
        .as_object_mut()
        .ok_or_else(|| Error::Invalid("OPENCODE_CONFIG_CONTENT must be a JSON object".into()))?;
    let mut permission = json!({
        "*": if read_only { "deny" } else { "allow" },
        "read": "allow", "glob": "allow", "grep": "allow",
        "list": "allow", "webfetch": "allow", "websearch": "allow",
        "task": "deny", "question": "deny",
        "plan_enter": "deny", "plan_exit": "deny",
        format!("{SERVER}_{TOOL}"): "allow",
        "external_directory": { "*": "deny" }
    });
    for dir in &spec.add_dirs {
        permission["external_directory"][format!("{}/*", dir.display())] = "allow".into();
    }
    // A dedicated primary agent keeps user-defined agents from overriding
    // this turn's permissions.
    root.entry("agent")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| Error::Invalid("OpenCode agent configuration must be an object".into()))?
        .insert(
            SERVER.into(),
            json!({"description": "Antislop managed agent", "mode": "primary",
                   "permission": permission}),
        );
    root.entry("mcp")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| Error::Invalid("OpenCode mcp configuration must be an object".into()))?
        .insert(
            SERVER.into(),
            json!({"type": "remote", "url": gateway.url(),
                   "headers": {"Authorization": format!("Bearer {}", gateway.token())}}),
        );
    command.env("OPENCODE_CONFIG_CONTENT", config.to_string());
    command.args(["--agent", SERVER]);
    if let Some(id) = &spec.resume {
        command.args(["--session", id]);
    }
    if let Some(model) = &spec.model {
        command.args(["--model", model]);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn spec(provider: ProviderKind, access: Access, resume: Option<&str>) -> Spec {
        Spec {
            provider,
            binary: PathBuf::from(super::super::agent::name(provider)),
            args: vec![],
            model: Some("m1".into()),
            effort: Some("high".into()),
            cwd: PathBuf::from("/work"),
            add_dirs: vec![PathBuf::from("/dep")],
            access,
            resume: resume.map(str::to_owned),
        }
    }

    fn words(launch: &Launch) -> Vec<String> {
        launch
            .command
            .get_argv()
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn claude_read_only_has_no_editing_tools_and_a_known_session() {
        let gateway = Gateway::start().unwrap();
        let launch = command(
            &spec(ProviderKind::Claude, Access::ReadOnly, None),
            &gateway,
        )
        .unwrap();
        let args = words(&launch);
        assert!(args.contains(&"--strict-mcp-config".to_owned()));
        assert!(!args.contains(&"plan".to_owned()));
        let denied = &args[args.iter().position(|a| a == "--disallowedTools").unwrap() + 1];
        assert!(denied.contains("Edit") && denied.contains("Write") && denied.contains("Task"));
        let allowed = &args[args.iter().position(|a| a == "--allowedTools").unwrap() + 1];
        assert!(allowed.contains("mcp__antislop__finish_turn"));
        assert!(allowed.contains("Bash(git diff:*)") && !allowed.split(',').any(|a| a == "Bash"));
        let id = launch.session.unwrap();
        assert_eq!(
            args[args.iter().position(|a| a == "--session-id").unwrap() + 1],
            id
        );
    }

    #[test]
    fn claude_does_not_inherit_a_parent_sessions_markers() {
        // SAFETY: no other test reads this variable.
        unsafe { std::env::set_var("CLAUDE_CODE_CHILD_SESSION", "1") };
        let gateway = Gateway::start().unwrap();
        let launch = command(&spec(ProviderKind::Claude, Access::Write, None), &gateway).unwrap();
        assert!(
            launch
                .command
                .get_env("CLAUDE_CODE_CHILD_SESSION")
                .is_none()
        );
    }

    #[test]
    fn claude_writers_edit_and_resume_the_given_session() {
        let gateway = Gateway::start().unwrap();
        let launch = command(
            &spec(ProviderKind::Claude, Access::Write, Some("s-1")),
            &gateway,
        )
        .unwrap();
        let args = words(&launch);
        assert!(
            args.windows(2)
                .any(|w| w == ["--permission-mode", "acceptEdits"])
        );
        assert!(args.windows(2).any(|w| w == ["--resume", "s-1"]));
        assert_eq!(launch.session.as_deref(), Some("s-1"));
    }

    #[test]
    fn codex_gets_the_gateway_per_launch_and_the_roles_sandbox() {
        let gateway = Gateway::start().unwrap();
        let mut codex = spec(ProviderKind::Codex, Access::ReadOnly, Some("t-9"));
        codex.args = vec![
            "--sandbox".into(),
            "danger-full-access".into(),
            "--x".into(),
        ];
        let launch = command(&codex, &gateway).unwrap();
        let args = words(&launch);
        assert_eq!(&args[1..3], ["--x", "resume"]);
        assert!(args.windows(2).any(|w| w == ["-s", "read-only"]));
        assert!(!args.contains(&"danger-full-access".to_owned()));
        assert!(
            args.iter()
                .any(|a| a == "check_for_update_on_startup=false")
        );
        assert!(
            args.iter()
                .any(|a| a.starts_with("mcp_servers.antislop.url="))
        );
        assert!(
            !args.iter().any(|a| a.contains(gateway.token())),
            "the token travels in the environment, not in argv"
        );
        assert_eq!(launch.session.as_deref(), Some("t-9"));
        let writer = command(&spec(ProviderKind::Codex, Access::Write, None), &gateway).unwrap();
        assert!(
            words(&writer)
                .windows(2)
                .any(|w| w == ["-s", "danger-full-access"])
        );
        assert_eq!(writer.session, None);
    }

    #[test]
    fn opencode_denies_everything_but_the_gateway_for_read_only_roles() {
        let gateway = Gateway::start().unwrap();
        let launch = command(
            &spec(ProviderKind::Opencode, Access::ReadOnly, None),
            &gateway,
        )
        .unwrap();
        let env = launch
            .command
            .get_env("OPENCODE_CONFIG_CONTENT")
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let config: Value = serde_json::from_str(&env).unwrap();
        let permission = &config["agent"]["antislop"]["permission"];
        assert_eq!(permission["*"], "deny");
        assert_eq!(permission["antislop_finish_turn"], "allow");
        assert_eq!(config["mcp"]["antislop"]["url"], gateway.url());
        assert!(
            words(&launch)
                .windows(2)
                .any(|w| w == ["--agent", "antislop"])
        );
    }

    #[test]
    fn agy_gets_the_gateway_through_the_environment_of_its_bridge() {
        // SAFETY: tests of this module do not read this variable concurrently.
        unsafe { std::env::set_var("ANTISLOP_SKIP_AGY_BRIDGE_CHECK", "1") };
        let gateway = Gateway::start().unwrap();
        let launch = command(
            &spec(ProviderKind::Agy, Access::Write, Some("c-1")),
            &gateway,
        )
        .unwrap();
        let args = words(&launch);
        assert!(args.windows(2).any(|w| w == ["--mode", "accept-edits"]));
        assert!(args.windows(2).any(|w| w == ["--conversation", "c-1"]));
        assert!(!args.iter().any(|a| a.contains(gateway.token())));
        let token = launch.command.get_env("ANTISLOP_MCP_TOKEN").unwrap();
        assert_eq!(token.to_string_lossy(), gateway.token());
        let read_only =
            command(&spec(ProviderKind::Agy, Access::ReadOnly, None), &gateway).unwrap();
        assert!(
            words(&read_only)
                .windows(2)
                .any(|w| w == ["--mode", "plan"])
        );
    }

    #[test]
    fn providers_without_an_adapter_fail_with_a_plain_reason() {
        let gateway = Gateway::start().unwrap();
        let error = command(&spec(ProviderKind::Gemini, Access::Write, None), &gateway)
            .err()
            .unwrap();
        assert!(
            error
                .to_string()
                .contains("gemini has no interactive adapter")
        );
    }
}

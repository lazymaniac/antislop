use super::*;
use crate::config::ProviderKind;
use crate::lean::agent::{Access, Spec};
use crate::lean::mcp::{Gateway, SERVER};
use serde_json::{Value, json};

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
    assert!(allowed.split(',').any(|a| a == "Bash"));
    let settings = &args[args.iter().position(|a| a == "--settings").unwrap() + 1];
    let settings: Value = serde_json::from_str(settings).unwrap();
    assert_eq!(settings["sandbox"]["enabled"], true);
    assert_eq!(settings["sandbox"]["filesystem"]["denyWrite"][0], "/work");
    assert!(
        !words(&command(&spec(ProviderKind::Claude, Access::Write, None), &gateway).unwrap())
            .contains(&"--settings".to_owned())
    );
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
    let trusted = format!(
        "projects={{{}={{trust_level=\"trusted\"}}}}",
        Value::String(codex.cwd.to_string_lossy().into_owned())
    );
    assert!(args.contains(&trusted), "{args:?}");
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
    let read_only = command(&spec(ProviderKind::Agy, Access::ReadOnly, None), &gateway).unwrap();
    assert!(
        words(&read_only)
            .windows(2)
            .any(|w| w == ["--mode", "plan"])
    );
}

#[test]
fn gemini_uses_private_session_settings_and_resumes() {
    let gateway = Gateway::start().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    for access in [Access::ReadOnly, Access::Write] {
        let mut spec = spec(ProviderKind::Gemini, access, Some("g-1"));
        spec.cwd = cwd.path().to_owned();
        let launch = command(&spec, &gateway).unwrap();
        let path = launch.settings.as_ref().unwrap().path().to_owned();
        let config: Value = serde_json::from_reader(std::fs::File::open(&path).unwrap()).unwrap();
        assert_eq!(config["mcpServers"][SERVER]["httpUrl"], gateway.url());
        assert_eq!(config["mcpServers"][SERVER]["trust"], true);
        assert_eq!(
            config["mcpServers"][SERVER]["includeTools"],
            json!(gateway.tool_names())
        );
        assert_eq!(
            config["tools"]["core"].is_array(),
            access == Access::ReadOnly
        );
        assert!(
            !std::fs::read_to_string(&path)
                .unwrap()
                .contains(gateway.token())
        );
        let args = words(&launch);
        assert!(args.windows(2).any(|w| w == ["--resume", "g-1"]));
        assert!(args.windows(2).any(|w| w == ["--model", "m1"]));
        assert!(!args.contains(&"--effort".to_owned()));
        assert_eq!(launch.session.as_deref(), Some("g-1"));
        drop(launch);
        assert!(!path.exists());
    }
    assert!(!cwd.path().join(".gemini").exists());
}

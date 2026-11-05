use super::*;
use crate::config::ProviderKind;
use crate::lean::agent::Spec;
use crate::lean::mcp::Gateway;
use serde_json::Value;

#[test]
fn every_provider_launches_with_full_access_and_resumes_the_selected_session() {
    // SAFETY: all launch tests use this fixture-only bridge-check override.
    unsafe {
        std::env::set_var("ANTISLOP_SKIP_AGY_BRIDGE_CHECK", "1");
        std::env::set_var("ANTISLOP_SKIP_GEMINI_SETTINGS_CHECK", "1");
    };
    let gateway = Gateway::start().unwrap();
    for provider in [
        ProviderKind::Codex,
        ProviderKind::Claude,
        ProviderKind::Gemini,
        ProviderKind::Agy,
        ProviderKind::Opencode,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let specification = Spec {
            provider,
            binary: crate::lean::agent::name(provider).into(),
            args: vec![],
            model: Some("model-fixture".into()),
            effort: Some("high".into()),
            cwd: project,
            runtime: directory.path().join("runtime"),
            add_dirs: vec![directory.path().join("dependency")],
            resume: Some("saved-session".into()),
        };
        let launch = command(&specification, &gateway).unwrap();
        let args: Vec<_> = launch
            .command
            .get_argv()
            .iter()
            .map(|v| v.to_string_lossy().into_owned())
            .collect();
        assert!(
            args.iter().any(|v| v == "saved-session"),
            "{provider:?}: {args:?}"
        );
        assert!(args.iter().any(|v| v == "model-fixture"));
        match provider {
            ProviderKind::Codex => {
                assert!(
                    args.iter()
                        .any(|v| v == "--dangerously-bypass-approvals-and-sandbox")
                );
                assert!(!args.iter().any(|v| v == "-s"));
                assert!(args.iter().any(|v| v == "--no-daemon"));
                assert!(
                    args.iter()
                        .any(|v| v == "sandbox_mode=\"danger-full-access\"")
                );
                assert!(launch.command.get_env("ANTISLOP_MCP_TOKEN").is_some());
            }
            ProviderKind::Claude => {
                assert!(args.iter().any(|v| v == "--dangerously-skip-permissions"));
                assert!(!args.iter().any(|v| v == "--disallowedTools"));
                let settings = &args[args.iter().position(|v| v == "--settings").unwrap() + 1];
                assert_eq!(
                    serde_json::from_str::<Value>(settings).unwrap()["sandbox"]["enabled"],
                    false
                );
            }
            ProviderKind::Gemini => {
                assert!(args.windows(2).any(|v| v == ["--approval-mode", "yolo"]));
                assert!(launch.settings.is_none());
                assert!(launch.command.get_env("ANTISLOP_MCP_URL").is_some());
                assert!(launch.command.get_env("ANTISLOP_MCP_TOKEN").is_some());
                assert!(
                    launch
                        .command
                        .get_env("GEMINI_CLI_SYSTEM_SETTINGS_PATH")
                        .is_none()
                );
                let settings: Value =
                    serde_json::from_str(&crate::lean::launch::providers::gemini::settings_text())
                        .unwrap();
                assert_eq!(settings["mcpServers"]["antislop"]["trust"], true);
                assert_eq!(
                    settings["mcpServers"]["antislop"]["httpUrl"],
                    "${ANTISLOP_MCP_URL}"
                );
            }
            ProviderKind::Agy => {
                assert!(args.iter().any(|v| v == "--dangerously-skip-permissions"));
                assert!(!args.iter().any(|v| v == "--sandbox" || v == "plan"));
            }
            ProviderKind::Opencode => {
                let config: Value = serde_json::from_str(
                    &launch
                        .command
                        .get_env("OPENCODE_CONFIG_CONTENT")
                        .unwrap()
                        .to_string_lossy(),
                )
                .unwrap();
                assert_eq!(config["permission"]["*"], "allow");
                assert_eq!(config["permission"]["external_directory"]["*"], "allow");
                assert_eq!(config["agent"]["antislop"]["permission"]["*"], "allow");
                assert!(
                    config["mcp"]["antislop"]["url"]
                        .as_str()
                        .unwrap()
                        .starts_with("http://")
                );
            }
        }
    }
}

use super::*;
use std::sync::{Arc, atomic::AtomicBool};

fn spec(directory: &std::path::Path) -> Spec {
    Spec {
        provider: ProviderKind::Pi,
        binary: "pi".into(),
        args: vec!["--custom".into(), "value with spaces".into()],
        model: Some("fixture/model".into()),
        effort: Some("high".into()),
        cwd: directory.into(),
        runtime: directory.join("runtime"),
        add_dirs: vec![],
        resume: None,
    }
}

#[test]
fn launch_preserves_both_commands_and_rejects_only_reserved_tokens() {
    let directory = tempfile::tempdir().unwrap();
    let gateway = Gateway::start().unwrap();
    for prefix in [vec![], vec!["launch".into(), "pi".into()]] {
        let mut spec = spec(directory.path());
        if !prefix.is_empty() {
            spec.binary = "unsloth".into();
        }
        spec.args.splice(0..0, prefix);
        let launch = super::super::command(&spec, &gateway).unwrap();
        let args: Vec<_> = launch
            .command
            .get_argv()
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        assert_eq!(&args[1..1 + spec.args.len()], spec.args);
        assert_eq!(launch.command.get_cwd().unwrap(), spec.cwd.as_os_str());
        assert!(args.windows(2).any(|v| v == ["--model", "fixture/model"]));
        assert!(args.windows(2).any(|v| v == ["--thinking", "high"]));
        assert!(!args.iter().any(|v| v.contains(gateway.token())));
        assert_eq!(launch.command.get_env(TOKEN_ENV).unwrap(), gateway.token());
        assert!(!launch.pi.unwrap().poll().unwrap());
    }
    for arg in ["--session=x", "--thinking=high", "--no-mcp", "--", "-p"] {
        assert!(validate_args(&[arg.into()]).is_err(), "{arg}");
    }
    validate_args(&[
        "launch".into(),
        "pi".into(),
        "--custom=--session".into(),
        "message".into(),
    ])
    .unwrap();
}

fn adapter_process(command: &CommandBuilder, spec: &Spec, fixture: &str) -> std::process::Command {
    let mut child = std::process::Command::new("node");
    child.arg(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/support")
            .join(fixture),
    );
    child
        .args(command.get_argv().iter().skip(1))
        .current_dir(&spec.cwd);
    for key in [
        "ANTISLOP_MCP_URL",
        TOKEN_ENV,
        "ANTISLOP_PI_EXPECTED",
        "ANTISLOP_PI_STATUS_PATH",
    ] {
        child.env(key, command.get_env(key).unwrap());
    }
    child
}

fn run_adapter(
    spec: &Spec,
    gateway: &Gateway,
    mode: Option<&str>,
) -> (PiLaunchState, std::process::Output) {
    let mut command = CommandBuilder::new("node");
    let (_, state) = prepare(&mut command, spec, gateway).unwrap();
    let mut child = adapter_process(&command, spec, "pi_extension.mjs");
    if let Some(mode) = mode {
        child.env("PI_TEST_MODE", mode);
    }
    (
        state,
        child
            .output()
            .expect("Node is required to execute the shipped Pi adapter"),
    )
}

#[test]
fn shipped_adapter_discovers_role_tools_and_gateway_accepts_its_result() {
    let directory = tempfile::tempdir().unwrap();
    let gateway = Gateway::start().unwrap();
    gateway.set_role("worker:task").unwrap();
    gateway
        .set_handler(Arc::new(|name, args| {
            assert_eq!(name, "complete");
            assert_eq!(args["report"], "adapter result");
            Ok(json!({"recorded": true}))
        }))
        .unwrap();
    gateway.begin("current").unwrap();
    let (state, output) = run_adapter(&spec(directory.path()), &gateway, None);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(state.poll().unwrap());
    assert!(gateway.connected());
    let result = gateway
        .wait("current", &AtomicBool::new(false), || Ok(()))
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&result.text).unwrap()["report"],
        "adapter result"
    );
}

#[test]
fn shipped_adapter_reports_api_settings_and_session_errors_without_token() {
    let directory = tempfile::tempdir().unwrap();
    let gateway = Gateway::start().unwrap();
    for (mode, reason) in [
        ("missing-api", "registerMcpServer"),
        ("register-error", "registration failed"),
        ("clamp", "thinking mismatch"),
        ("reasoning-off", "thinking mismatch"),
        ("wrong-cwd", "cwd"),
        ("wrong-session", "session file"),
        ("wrong-model", "model mismatch"),
    ] {
        let (state, output) = run_adapter(&spec(directory.path()), &gateway, Some(mode));
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let error = state.poll().unwrap_err().to_string();
        assert!(error.contains(reason), "{mode}: {error}");
        assert!(!error.contains(gateway.token()));
        let raw = std::fs::read_to_string(&state.status).unwrap();
        assert!(!raw.contains(gateway.token()));
    }
    let (state, output) = run_adapter(&spec(directory.path()), &gateway, Some("non-reasoning"));
    assert!(output.status.success());
    assert!(state.poll().unwrap());
    let status = state.status().unwrap().unwrap();
    assert_eq!(status["effort"], "high");
    assert_eq!(status["actual"]["effort"], "off");
}

#[test]
fn stale_status_and_invalid_json_never_signal_readiness() {
    let directory = tempfile::tempdir().unwrap();
    let gateway = Gateway::start().unwrap();
    let (state, output) = run_adapter(&spec(directory.path()), &gateway, Some("clamp"));
    assert!(output.status.success());
    let mut status: Value = serde_json::from_slice(&std::fs::read(&state.status).unwrap()).unwrap();
    status["launch_id"] = json!("stale");
    std::fs::write(&state.status, status.to_string()).unwrap();
    assert!(state.poll().unwrap_err().to_string().contains("launch_id"));
    std::fs::write(&state.status, "{").unwrap();
    assert!(state.poll().unwrap_err().to_string().contains("JSON"));
}

#[test]
#[ignore = "requires ANTISLOP_TEST_PI_MODULE pointing to an installed Pi dist/index.js; never installs packages"]
fn installed_pi_mcp_executes_the_shipped_adapter() {
    let module = std::env::var("ANTISLOP_TEST_PI_MODULE")
        .expect("absolute installed Pi dist/index.js path required");
    assert!(std::path::Path::new(&module).is_absolute());
    let directory = tempfile::tempdir().unwrap();
    let gateway = Gateway::start().unwrap();
    gateway.set_role("worker:task").unwrap();
    gateway
        .set_handler(Arc::new(|name, args| {
            if name == "get_images" {
                use base64::Engine;
                let png = crate::lean::content::valid_png_fixture();
                return Ok(json!({"_mcp_content":[{"type":"image","mimeType":"image/png","data":base64::engine::general_purpose::STANDARD.encode(png)}]}));
            }
            assert_eq!(args["report"], "adapter result");
            Ok(json!({"recorded": true}))
        }))
        .unwrap();
    gateway.begin("current").unwrap();
    let mut spec = spec(directory.path());
    spec.args.clear();
    spec.model = None;
    spec.effort = None;
    let mut command = CommandBuilder::new("node");
    let (_, state) = prepare(&mut command, &spec, &gateway).unwrap();
    let mut child = adapter_process(&command, &spec, "pi_sdk.mjs");
    child.env("ANTISLOP_TEST_PI_MODULE", module);
    let output = child.output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(state.poll().unwrap());
    let result = gateway
        .wait("current", &AtomicBool::new(false), || Ok(()))
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&result.text).unwrap()["report"],
        "adapter result"
    );
}

#[test]
fn transport_diagnostics_match_only_operational_lines_for_this_launch() {
    let directory = tempfile::tempdir().unwrap();
    let gateway = Gateway::start().unwrap();
    let launch = super::super::command(&spec(directory.path()), &gateway).unwrap();
    let state = launch.pi.unwrap();
    let raw = format!(
        "MCP servers need attention:\n  {}: failed: HTTP 401 unauthorized\nRun /mcp to fix.",
        state.alias()
    );
    assert!(
        state
            .check_transport(&raw)
            .unwrap_err()
            .to_string()
            .contains("HTTP 401 unauthorized")
    );
    assert!(
        state
            .check_transport("other_server: failed: offline")
            .is_ok()
    );
    assert!(
        state
            .check_transport(&format!(
                "The report says {}: failed: offline",
                state.alias()
            ))
            .is_ok()
    );
    assert!(
        state
            .check_transport("MCP failed to load: connection refused")
            .is_err()
    );
}

#[test]
fn exact_unsloth_model_is_single_and_reasoning_is_not_thinking() {
    let dir = tempfile::tempdir().unwrap();
    let gateway = Gateway::start().unwrap();
    let mut spec = spec(dir.path());
    spec.binary = "unsloth".into();
    spec.effort = None;
    let model = "unsloth/Qwen3.8-Flash-Next-GGUF:UD-Q4_K_XL";
    spec.model = Some(model.into());
    spec.args = [
        "start",
        "pi",
        "--model",
        model,
        "--context-length",
        "200000",
        "--reasoning",
        "auto",
    ]
    .map(str::to_owned)
    .into();
    let launch = super::super::command(&spec, &gateway).unwrap();
    let argv = launch.command.get_argv();
    assert_eq!(
        &argv[1..9],
        spec.args
            .iter()
            .map(std::ffi::OsString::from)
            .collect::<Vec<_>>()
    );
    assert_eq!(argv.iter().filter(|arg| *arg == "--model").count(), 1);
    assert!(!argv.iter().any(|arg| arg == "--thinking"));
    assert_eq!(launch.pi.unwrap().expected["model"], model);
    spec.model = None;
    assert_eq!(
        super::super::command(&spec, &gateway)
            .unwrap()
            .pi
            .unwrap()
            .expected["model"],
        model
    );
    spec.model = Some("conflict".into());
    assert!(super::super::command(&spec, &gateway).is_err());
    for args in [
        vec!["--model"],
        vec!["--model="],
        vec!["--model", "--reasoning"],
        vec!["--model=x", "--model", "x"],
    ] {
        assert!(argument_model(&args.into_iter().map(str::to_owned).collect::<Vec<_>>()).is_err());
    }
}

#[test]
fn resume_refuses_missing_empty_truncated_and_unsupported_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.jsonl");
    assert!(validate_resume(&path).is_err());
    for contents in [
        "",
        "{}",
        "{",
        "{\"type\":\"session\",\"version\":4,\"id\":\"s\"}\n",
        "{\"type\":\"session\",\"version\":3,\"id\":\"s\"}\n{",
    ] {
        std::fs::write(&path, contents).unwrap();
        assert!(validate_resume(&path).is_err(), "{contents}");
    }
    std::fs::write(&path, "{\"type\":\"session\",\"version\":3,\"id\":\"s\"}\n{\"type\":\"future_entry\",\"id\":\"m\"}\n").unwrap();
    validate_resume(&path).unwrap();
}

#[test]
fn idle_acknowledgement_is_bound_to_launch_and_latest_request() {
    let dir = tempfile::tempdir().unwrap();
    let gateway = Gateway::start().unwrap();
    let (state, output) = run_adapter(&spec(dir.path()), &gateway, Some("clamp"));
    assert!(output.status.success());
    let mut initial = state.expected.clone();
    initial["version"] = json!(1);
    initial["state"] = json!("ready");
    initial["actual"] = state.expected.clone();
    initial["generation"] = json!(0);
    std::fs::write(&state.status, initial.to_string()).unwrap();
    state.track_submission("turn-1").unwrap();
    state.request_idle("turn-1", true).unwrap();
    let control: Value = serde_json::from_slice(
        &std::fs::read(state.status.with_file_name("control.json")).unwrap(),
    )
    .unwrap();
    let mut status = state.expected.clone();
    status["version"] = json!(1);
    status["state"] = json!("ready");
    status["actual"] = state.expected.clone();
    status["idle"] = json!(true);
    status["ack"] = control["nonce"].clone();
    std::fs::write(&state.status, status.to_string()).unwrap();
    assert!(state.idle_confirmed().unwrap());
    status["generation"] = json!(1);
    std::fs::write(&state.status, status.to_string()).unwrap();
    state.track_submission("turn-2").unwrap();
    assert!(state.request_idle("turn-1", true).is_err());
    state.request_idle("turn-2", false).unwrap();
    assert!(!state.idle_confirmed().unwrap());
    status["launch_id"] = json!("foreign");
    std::fs::write(&state.status, status.to_string()).unwrap();
    assert!(state.idle_confirmed().is_err());
}

#[test]
fn unsloth_artifact_maps_only_to_its_expected_pi_provider_and_repo() {
    let dir = tempfile::tempdir().unwrap();
    let gateway = Gateway::start().unwrap();
    let mut spec = spec(dir.path());
    spec.binary = "unsloth".into();
    spec.args = vec!["start".into(), "pi".into()];
    spec.model = Some("unsloth/Qwen3.8-Flash-Next-GGUF:UD-Q4_K_XL".into());
    let (state, output) = run_adapter(&spec, &gateway, Some("inspect-only"));
    assert!(output.status.success());
    assert!(state.poll().unwrap());
    assert_eq!(
        state.expected["wrapper_model"]["id"],
        "unsloth/Qwen3.8-Flash-Next-GGUF"
    );
    assert_eq!(
        state.expected["wrapper_model"]["quantization_observable"],
        false
    );
    for mode in ["wrong-model", "wrong-provider"] {
        let (state, output) = run_adapter(&spec, &gateway, Some(mode));
        assert!(output.status.success());
        assert!(
            state
                .poll()
                .unwrap_err()
                .to_string()
                .contains("model mismatch")
        );
    }
    spec.binary = "pi".into();
    assert!(unsloth_model(&spec, spec.model.as_deref()).is_none());
    spec.binary = "unsloth".into();
    spec.args[0] = "launch".into();
    assert!(unsloth_model(&spec, spec.model.as_deref()).is_none());
}

#[test]
fn exact_unsloth_start_profile_round_trips_and_resolves_into_launch() {
    use crate::config::{Config, RoleKind, WorkKind};
    let source = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/examples/antislop.toml")).replace("codex", "pi").replace(
        "binary = \"pi\"",
        "command = ['unsloth', 'start', 'pi', '--model', 'unsloth/Qwen3.8-Flash-Next-GGUF:UD-Q4_K_XL', '--context-length', '200000', '--reasoning', 'auto']",
    );
    let config = Config::from_toml(&source).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, &source).unwrap();
    assert_eq!(Config::load(&path).unwrap(), config);
    let profile = config
        .resolve(WorkKind::Orchestration, RoleKind::Orchestrator)
        .unwrap();
    let args = profile.args.clone();
    let spec = crate::lean::agent::Spec {
        provider: profile.provider,
        binary: profile.binary,
        args: profile.args,
        model: profile.model,
        effort: profile.native_effort,
        cwd: dir.path().into(),
        runtime: dir.path().join("runtime"),
        add_dirs: vec![],
        resume: None,
    };
    let gateway = crate::lean::mcp::Gateway::start().unwrap();
    let launch = crate::lean::launch::command(&spec, &gateway).unwrap();
    assert_eq!(
        &launch.command.get_argv()[1..9],
        args.iter()
            .map(std::ffi::OsString::from)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        launch
            .command
            .get_argv()
            .iter()
            .filter(|arg| *arg == "--model")
            .count(),
        1
    );
    assert!(
        launch
            .command
            .get_argv()
            .windows(2)
            .any(|args| args == ["--thinking", "high"])
    );
}

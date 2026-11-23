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
    for arg in [
        "--session=x",
        "--model",
        "--thinking=high",
        "--no-mcp",
        "--",
        "-p",
    ] {
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

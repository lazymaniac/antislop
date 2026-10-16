//! No authenticated requests: stderr fixtures expose only fixed categories.
use architect::{
    domain::{Run, RunSpec},
    providers::{
        FailureKind, Provider, ProviderEvent, ProviderFailure, TurnReceipt, TurnRequest,
        codex::CodexProvider, shared_cli,
    },
};
use std::{
    os::unix::fs::PermissionsExt,
    sync::{Arc, atomic::AtomicBool},
};

const SESSION: &str = "00000000-0000-4000-8000-000000000001";
const SECRET: &str = "SENTINEL-PRIVATE-API-KEY-DO-NOT-RETAIN";

fn failure(marker: &str, exit: i32, shared: bool, read_prompt: bool) -> ProviderFailure {
    let root = tempfile::tempdir().unwrap();
    let binary = root.path().join("fixture");
    let body = format!(
        "#!/usr/bin/env python3\nimport sys,json\nif '--version' in sys.argv: print('codex-cli 0.162.1');sys.exit(0)\n{}\nprint(json.dumps(dict(type='thread.started',thread_id={SESSION:?})),flush=True)\nsys.stderr.write({SECRET:?}+'x'*4144+{marker:?}+'x'*100000+{SECRET:?})\nsys.stderr.flush()\nsys.exit({exit})\n",
        if read_prompt {
            "sys.stdin.read()"
        } else {
            "pass"
        },
    );
    std::fs::write(&binary, body).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().to_string_lossy().into();
    let run = Run::new(architect::id(), spec).unwrap();
    let prompt = if read_prompt {
        "test".into()
    } else {
        "x".repeat(1_000_000)
    };
    let request = TurnRequest {
        turn_id: "safe-diagnostic",
        run: &run,
        prompt: &prompt,
        schema: &serde_json::json!({}),
        resume: None,
    };
    let cancelled = Arc::new(AtomicBool::new(false));
    let result = if shared {
        let mut command = shared_cli::command("claude", binary.to_str().unwrap()).unwrap();
        shared_cli::run(
            "claude",
            &mut command,
            &request,
            SESSION,
            cancelled,
            &mut |_| Ok(()),
            |_| Ok(shared_cli::StreamEvent::Unknown),
        )
    } else {
        CodexProvider {
            binary: Some(binary.to_string_lossy().into()),
            ..Default::default()
        }
        .turn_receipt(&request, cancelled, &mut |_| Ok(()))
    };
    let architect::Error::Provider(error) = result.unwrap_err() else {
        panic!("turn exit must produce a typed provider failure");
    };
    let serialized = serde_json::to_string(&error).unwrap();
    assert!(!serialized.contains(SECRET));
    assert!(!error.to_string().contains(SECRET));
    assert!(serialized.len() < 700, "raw stderr leaked into failure");
    assert_eq!(error.session_id.as_deref(), Some(SESSION));
    assert!(error.stopped);
    assert!(error.partial_effects);
    assert!(error.message.contains(&format!("exit_code={exit}")));
    error
}

#[test]
fn codex_classifies_known_stderr_without_retaining_secret_or_raw_diagnostics() {
    for (marker, kind, diagnosis) in [
        (
            "Malformed function call: private attempted arguments",
            FailureKind::Protocol,
            "malformed_function_call",
        ),
        (
            "MALFORMED_FUNCTION_CALL",
            FailureKind::Protocol,
            "malformed_function_call",
        ),
        (
            "context_length_exceeded",
            FailureKind::Protocol,
            "context_length_exceeded",
        ),
        (
            "invalid_json_schema",
            FailureKind::Protocol,
            "invalid_json_schema",
        ),
        (
            "invalid_api_key",
            FailureKind::Authentication,
            "authentication_error",
        ),
        (
            "rate_limit_exceeded",
            FailureKind::RateLimit,
            "rate_limit_or_quota",
        ),
        (
            "error sending request for url",
            FailureKind::Network,
            "network_transport_error",
        ),
    ] {
        let error = failure(marker, 7, false, true);
        assert_eq!(error.kind, kind);
        assert!(error.message.contains(&format!("diagnosis={diagnosis}")));
    }
}

#[test]
fn unknown_codex_exit_is_process_failure_including_success_without_terminal() {
    for exit in [0, 7] {
        let error = failure(
            "unknown diagnostic containing private payload",
            exit,
            false,
            true,
        );
        assert_eq!(error.kind, FailureKind::Process);
        assert!(error.message.contains("diagnosis=unknown"));
        assert!(!error.message.contains("private payload"));
    }
}

#[test]
fn broken_stdin_does_not_discard_typed_exit_diagnosis_or_native_id() {
    let error = failure("context_length_exceeded", 7, false, false);
    assert_eq!(error.kind, FailureKind::Protocol);
    assert!(error.message.contains("diagnosis=context_length_exceeded"));
}

#[test]
fn shared_cli_uses_same_private_classifier_and_keeps_unknown_exit_unknown() {
    assert_eq!(
        failure("insufficient_quota", 7, true, true).kind,
        FailureKind::RateLimit
    );
    let error = failure("unknown private diagnostic", 7, true, true);
    assert_eq!(error.kind, FailureKind::Process);
    assert!(error.message.contains("diagnosis=unknown"));
}

fn stdout_fixture(
    frames: serde_json::Value,
    stderr: &str,
    exit: i32,
) -> (architect::Result<TurnReceipt>, Vec<ProviderEvent>) {
    let root = tempfile::tempdir().unwrap();
    let binary = root.path().join("stdout-fixture");
    let script = format!(
        "#!/usr/bin/env python3\nimport sys,json\nif '--version' in sys.argv: print('codex-cli 0.162.1');sys.exit(0)\nsys.stdin.read()\nprint(json.dumps(dict(type='thread.started',thread_id={SESSION:?})),flush=True)\nfor frame in json.loads({}): print(json.dumps(frame),flush=True)\nsys.stderr.write({})\nsys.exit({exit})\n",
        serde_json::to_string(&frames.to_string()).unwrap(),
        serde_json::to_string(stderr).unwrap(),
    );
    std::fs::write(&binary, script).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().to_string_lossy().into();
    let run = Run::new(architect::id(), spec).unwrap();
    let mut events = Vec::new();
    let result = CodexProvider {
        binary: Some(binary.to_string_lossy().into()),
        ..Default::default()
    }
    .turn_receipt(
        &TurnRequest {
            turn_id: "stdout-diagnostics",
            run: &run,
            prompt: "test",
            schema: &serde_json::json!({}),
            resume: None,
        },
        Arc::new(AtomicBool::new(false)),
        &mut |event| {
            events.push(event.clone());
            Ok(())
        },
    );
    (result, events)
}

#[test]
fn stdout_only_human_context_errors_preserve_identity_and_never_emit_raw_diagnostics() {
    for event in [
        serde_json::json!({"type":"turn.failed","error":{"message":format!("{SECRET} Your input exceeds the context window of this model. {SECRET}")}}),
        serde_json::json!({"type":"error","message":format!("{SECRET} This model's maximum context length was exceeded {SECRET}")}),
        serde_json::json!({"type":"turn.failed","error":format!("{SECRET} Input is too long {SECRET}")}),
        serde_json::json!({"type":"turn.failed","error":{"type":"ContextWindowExceeded","message":format!("{SECRET} private details {SECRET}")}}),
    ] {
        let (result, events) = stdout_fixture(serde_json::json!([event]), "", 1);
        let architect::Error::Provider(error) = result.unwrap_err() else {
            panic!("missing typed failure")
        };
        assert_eq!(error.kind, FailureKind::Protocol);
        assert_eq!(error.session_id.as_deref(), Some(SESSION));
        assert!(error.stopped && error.partial_effects);
        assert!(error.message.contains("diagnosis=context_length_exceeded"));
        assert!(!serde_json::to_string(&error).unwrap().contains(SECRET));
        assert!(!serde_json::to_string(&events).unwrap().contains(SECRET));
    }
}

#[test]
fn codex_turn_start_input_size_errors_have_a_distinct_static_diagnosis() {
    // Shape observed during the explicitly authorized diagnostic invocation:
    // only thread.started on stdout, followed by CLI turn/start rejection.
    for message in [
        "Error: turn/start: turn/start failed: Input exceeds the maximum length of 1048576 characters. (code -32602), data: {\"input_error_code\":\"input_too_large\",\"max_chars\":1048576,\"actual_chars\":1290156}",
        "input_too_large",
        "Input exceeds the maximum length of 1048576 characters",
    ] {
        let error = failure(message, 1, false, true);
        assert_eq!(error.kind, FailureKind::Protocol);
        assert!(error.message.contains("diagnosis=input_too_large"));
        let serialized = serde_json::to_string(&error).unwrap();
        for private in [SECRET, "1290156", "1048576", "-32602", "turn/start"] {
            assert!(!serialized.contains(private), "raw diagnostic retained");
        }
    }
}

#[test]
fn stdout_input_size_errors_preserve_identity_without_retaining_payload() {
    for event in [
        serde_json::json!({"type":"turn.failed","error":{"code":"input_too_large","message":SECRET}}),
        serde_json::json!({"type":"error","message":format!("{SECRET} Input exceeds the maximum length of 1048576 characters. {SECRET}")}),
    ] {
        let (result, events) = stdout_fixture(serde_json::json!([event]), "", 1);
        let architect::Error::Provider(error) = result.unwrap_err() else {
            panic!("missing typed failure")
        };
        assert_eq!(error.kind, FailureKind::Protocol);
        assert_eq!(error.session_id.as_deref(), Some(SESSION));
        assert!(error.stopped && error.partial_effects);
        assert!(error.message.contains("diagnosis=input_too_large"));
        assert!(!serde_json::to_string(&error).unwrap().contains(SECRET));
        assert!(!serde_json::to_string(&events).unwrap().contains(SECRET));
    }
}

#[test]
fn unknown_stdout_terminal_is_process_failure_without_secret_or_event_leak() {
    let (result, events) = stdout_fixture(
        serde_json::json!([
            {"type":"turn.failed","error":{"message":format!("{SECRET} unclassified private failure {SECRET}")}}
        ]),
        "",
        1,
    );
    let architect::Error::Provider(error) = result.unwrap_err() else {
        panic!("missing typed failure")
    };
    assert_eq!(error.kind, FailureKind::Process);
    assert!(error.message.contains("diagnosis=unknown"));
    assert!(!serde_json::to_string(&error).unwrap().contains(SECRET));
    assert!(!serde_json::to_string(&events).unwrap().contains(SECRET));
}

#[test]
fn completed_artifact_discussing_context_limits_is_not_a_diagnostic_failure() {
    let output = serde_json::json!({"artifact":"context_length_exceeded; Your input exceeds the context window; input_too_large; Input exceeds the maximum length"}).to_string();
    let (result, events) = stdout_fixture(
        serde_json::json!([
            {"type":"item.completed","item":{"id":"artifact","type":"agent_message","text":output}},
            {"type":"turn.completed"}
        ]),
        &format!("{SECRET} authentication failed"),
        0,
    );
    let receipt = result.unwrap();
    assert!(receipt.completed && receipt.delivered);
    assert!(!serde_json::to_string(&receipt).unwrap().contains(SECRET));
    assert!(!serde_json::to_string(&events).unwrap().contains(SECRET));
}

#[test]
fn human_stderr_classifications_remain_static_and_private() {
    for (message, expected) in [
        (
            "Your input exceeds the context window of this model",
            FailureKind::Protocol,
        ),
        ("Authentication required", FailureKind::Authentication),
        ("You've hit your usage limit", FailureKind::RateLimit),
        (
            "Stream disconnected before completion",
            FailureKind::Network,
        ),
    ] {
        assert_eq!(failure(message, 1, false, true).kind, expected);
    }
}

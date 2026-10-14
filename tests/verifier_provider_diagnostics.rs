//! Independent privacy and cancellation probes. All providers are local fixtures.
use architect::{
    domain::{Run, RunSpec},
    providers::{
        FailureKind, Provider, ProviderEvent, ProviderFailure, TurnRequest, codex::CodexProvider,
    },
};
use std::{
    os::unix::fs::PermissionsExt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
const SESSION: &str = "00000000-0000-4000-8000-000000000042";
const SECRET: &str = "PRIVATE-DIAGNOSTIC-SENTINEL-DO-NOT-PERSIST";
fn fixture(body: &str) -> (tempfile::TempDir, CodexProvider, Run) {
    let root = tempfile::tempdir().unwrap();
    let binary = root.path().join("provider");
    let script = format!(
        "#!/usr/bin/env python3\nimport sys,json,time,pathlib\nif '--version' in sys.argv:\n print('codex-cli 0.162.1');sys.exit(0)\nsys.stdin.read()\nprint(json.dumps(dict(type='thread.started',thread_id={SESSION:?})),flush=True)\n{body}\n"
    );
    std::fs::write(&binary, script).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().canonicalize().unwrap().to_string_lossy().into();
    let run = Run::new(architect::id(), spec).unwrap();
    let provider = CodexProvider {
        binary: Some(binary.to_string_lossy().into()),
        ..Default::default()
    };
    (root, provider, run)
}
fn typed(result: architect::Result<architect::providers::TurnReceipt>) -> ProviderFailure {
    let architect::Error::Provider(failure) = result.unwrap_err() else {
        panic!("expected typed provider failure");
    };
    let serialized = serde_json::to_string(&failure).unwrap();
    assert!(!serialized.contains(SECRET));
    assert!(serialized.len() < 1000);
    failure
}
#[test]
fn exit_zero_with_partial_artifact_and_no_terminal_never_becomes_completed() {
    let (_root, provider, run) = fixture(&format!(
        "print(json.dumps(dict(type='item.completed',item=dict(type='agent_message',text='{{}}'))),flush=True)\nsys.stderr.write({SECRET:?}+' context_window_exceeded '+{SECRET:?});sys.stderr.flush()\nsys.exit(0)"
    ));
    let mut events = vec![];
    let failure = typed(provider.turn_receipt(
        &TurnRequest {
            turn_id: "zero-incomplete",
            run: &run,
            prompt: "test",
            schema: &serde_json::json!({}),
            resume: None,
        },
        Arc::new(AtomicBool::new(false)),
        &mut |e| {
            events.push(e.clone());
            Ok(())
        },
    ));
    assert_eq!(failure.kind, FailureKind::Protocol);
    assert!(failure.message.contains("exit_code=0"));
    assert!(
        failure
            .message
            .contains("diagnosis=context_length_exceeded")
    );
    assert_eq!(failure.session_id.as_deref(), Some(SESSION));
    assert!(failure.partial_effects);
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, ProviderEvent::Completed { .. }))
    );
}
#[test]
fn cancellation_during_stderr_flood_stops_without_retaining_diagnostics() {
    let (_root, provider, run) = fixture(&format!(
        "while True:\n sys.stderr.write({SECRET:?}+' context_length_exceeded '+'x'*10000);sys.stderr.flush();time.sleep(.001)"
    ));
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    let events = Arc::new(Mutex::new(vec![]));
    let recorded = events.clone();
    let worker = std::thread::spawn(move || {
        provider.turn_receipt(
            &TurnRequest {
                turn_id: "cancel-flood",
                run: &run,
                prompt: "test",
                schema: &serde_json::json!({}),
                resume: None,
            },
            flag,
            &mut |e| {
                recorded.lock().unwrap().push(e.clone());
                Ok(())
            },
        )
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !events
        .lock()
        .unwrap()
        .iter()
        .any(|e| matches!(e, ProviderEvent::SessionObserved { .. }))
    {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let start = Instant::now();
    cancelled.store(true, Ordering::SeqCst);
    let failure = typed(worker.join().unwrap());
    assert!(start.elapsed() < Duration::from_secs(2));
    assert_eq!(failure.kind, FailureKind::Cancelled);
    assert!(failure.stopped);
    assert!(failure.partial_effects);
    assert_eq!(failure.session_id.as_deref(), Some(SESSION));
    let events = events.lock().unwrap();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, ProviderEvent::Stopped { .. }))
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, ProviderEvent::Completed { .. }))
    );
}
#[test]
fn native_failure_with_stderr_category_keeps_observed_session_and_diagnosis() {
    let (_root, provider, run) = fixture(&format!(
        "sys.stderr.write({SECRET:?}+' context_length_exceeded '+{SECRET:?});sys.stderr.flush()\nprint(json.dumps(dict(type='turn.failed',error=dict(message={SECRET:?}))),flush=True)\nsys.exit(1)"
    ));
    let failure = typed(provider.turn_receipt(
        &TurnRequest {
            turn_id: "native-failed",
            run: &run,
            prompt: "test",
            schema: &serde_json::json!({}),
            resume: None,
        },
        Arc::new(AtomicBool::new(false)),
        &mut |_| Ok(()),
    ));
    assert_eq!(
        failure.session_id.as_deref(),
        Some(SESSION),
        "native failure must retain the durable observed identity"
    );
    assert!(failure.partial_effects);
    assert_eq!(failure.kind, FailureKind::Protocol);
    assert!(
        failure
            .message
            .contains("diagnosis=context_length_exceeded")
    );
}

#[test]
fn stdout_only_human_terminal_diagnostics_classify_without_retaining_secret_fields() {
    for (message, kind, diagnosis, top_level) in [
        (
            "Your input exceeds the context window of this model",
            FailureKind::Protocol,
            "context_length_exceeded",
            true,
        ),
        (
            "maximum context length was exceeded",
            FailureKind::Protocol,
            "context_length_exceeded",
            false,
        ),
        (
            "Authentication failed; not logged in",
            FailureKind::Authentication,
            "authentication_error",
            true,
        ),
        (
            "rate limit reached for this request",
            FailureKind::RateLimit,
            "rate_limit_or_quota",
            false,
        ),
        (
            "stream disconnected before completion",
            FailureKind::Network,
            "network_transport_error",
            true,
        ),
    ] {
        let text = format!("{SECRET} {message} {SECRET}");
        let event = if top_level {
            serde_json::json!({"type":"error","message":text})
        } else {
            serde_json::json!({"type":"turn.failed","error":{"message":text}})
        };
        let (_root, provider, run) = fixture(&format!(
            "print({:?},flush=True)\nsys.exit(1)",
            event.to_string()
        ));
        let failure = typed(provider.turn_receipt(
            &TurnRequest {
                turn_id: "stdout-human",
                run: &run,
                prompt: "test",
                schema: &serde_json::json!({}),
                resume: None,
            },
            Arc::new(AtomicBool::new(false)),
            &mut |_| Ok(()),
        ));
        assert_eq!(failure.kind, kind);
        assert!(failure.message.contains(&format!("diagnosis={diagnosis}")));
        assert_eq!(failure.session_id.as_deref(), Some(SESSION));
        assert!(failure.stopped && failure.partial_effects);
        assert!(failure.message.contains("exit_code=1"));
    }
}
#[test]
fn unknown_stdout_terminal_reason_stays_unknown_without_exposing_private_message() {
    let event = serde_json::json!({"type":"error","message":format!("{SECRET} unrecognized private reason {SECRET}")});
    let (_root, provider, run) = fixture(&format!(
        "print({:?},flush=True)\nsys.exit(1)",
        event.to_string()
    ));
    let failure = typed(provider.turn_receipt(
        &TurnRequest {
            turn_id: "stdout-unknown",
            run: &run,
            prompt: "test",
            schema: &serde_json::json!({}),
            resume: None,
        },
        Arc::new(AtomicBool::new(false)),
        &mut |_| Ok(()),
    ));
    assert_eq!(failure.kind, FailureKind::Process);
    assert!(failure.message.contains("diagnosis=unknown"));
    assert!(!failure.message.contains("private reason"));
    assert_eq!(failure.session_id.as_deref(), Some(SESSION));
}
#[test]
fn successful_artifact_that_mentions_error_markers_is_never_scanned_as_a_failure() {
    let artifact=serde_json::json!({"artifact":"Document context_length_exceeded, authentication failed, network_error, input_too_large and Input exceeds the maximum length messages"}).to_string();
    let item = serde_json::json!({"type":"item.completed","item":{"type":"agent_message","text":artifact}});
    let completed = serde_json::json!({"type":"turn.completed"});
    let (_root, provider, run) = fixture(&format!(
        "print({:?},flush=True)\nprint({:?},flush=True)\nsys.stderr.write({SECRET:?}+' context_length_exceeded input_too_large '+{SECRET:?});sys.stderr.flush()\nsys.exit(0)",
        item.to_string(),
        completed.to_string()
    ));
    let receipt = provider
        .turn_receipt(
            &TurnRequest {
                turn_id: "marker-document",
                run: &run,
                prompt: "test",
                schema: &serde_json::json!({}),
                resume: None,
            },
            Arc::new(AtomicBool::new(false)),
            &mut |_| Ok(()),
        )
        .unwrap();
    assert!(receipt.completed && receipt.delivered && receipt.stopped);
    assert_eq!(receipt.output, artifact);
    assert_eq!(receipt.session_id, SESSION);
}

#[test]
fn input_scalar_size_failure_is_distinct_and_redacted_even_across_drain_boundaries() {
    let private = format!(
        "{SECRET} Error: turn/start: turn/start failed: Input exceeds the maximum length of 1048576 characters. (code -32602), data: {{\"input_error_code\":\"input_too_large\",\"max_chars\":1048576,\"actual_chars\":1290156}} {SECRET}"
    );
    let (_root, provider, run) = fixture(&format!(
        "sys.stderr.write('x'*4090+{private:?}+'y'*100000);sys.stderr.flush()\nsys.exit(1)"
    ));
    let failure = typed(provider.turn_receipt(
        &TurnRequest {
            turn_id: "scalar-size-stderr",
            run: &run,
            prompt: "test",
            schema: &serde_json::json!({}),
            resume: None,
        },
        Arc::new(AtomicBool::new(false)),
        &mut |_| Ok(()),
    ));
    assert_eq!(failure.kind, FailureKind::Protocol);
    assert!(failure.message.contains("diagnosis=input_too_large"));
    assert!(!failure.message.contains("context_length_exceeded"));
    assert_eq!(failure.session_id.as_deref(), Some(SESSION));
    assert!(failure.stopped && failure.partial_effects);
    let serialized = serde_json::to_string(&failure).unwrap();
    for opaque in ["1048576", "1290156", "-32602", "turn/start", SECRET] {
        assert!(
            !serialized.contains(opaque),
            "raw diagnostic leaked {opaque}"
        );
    }
}
#[test]
fn stdout_terminal_size_code_and_human_phrase_produce_only_static_diagnosis() {
    for event in [
        serde_json::json!({"type":"turn.failed","error":{"code":"input_too_large","message":SECRET,"data":{"max_chars":1048576,"actual_chars":1290156}}}),
        serde_json::json!({"type":"error","message":format!("{SECRET} Input exceeds the maximum length of 1048576 characters. {SECRET}")}),
    ] {
        let (_root, provider, run) = fixture(&format!(
            "print({:?},flush=True)\nsys.exit(1)",
            event.to_string()
        ));
        let failure = typed(provider.turn_receipt(
            &TurnRequest {
                turn_id: "scalar-size-stdout",
                run: &run,
                prompt: "test",
                schema: &serde_json::json!({}),
                resume: None,
            },
            Arc::new(AtomicBool::new(false)),
            &mut |_| Ok(()),
        ));
        assert_eq!(failure.kind, FailureKind::Protocol);
        assert!(failure.message.contains("diagnosis=input_too_large"));
        assert!(!failure.message.contains("context_length_exceeded"));
        let serialized = serde_json::to_string(&failure).unwrap();
        for opaque in ["1048576", "1290156", SECRET] {
            assert!(!serialized.contains(opaque));
        }
    }
}

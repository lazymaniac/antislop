//! Independent counterexamples for adapter contracts. No authenticated turns.
use architect::{
    domain::{Run, RunSpec},
    providers::{FailureKind, Provider, TurnRequest, codex::CodexProvider},
};
use std::{
    os::unix::fs::PermissionsExt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

fn fixture(body: &str) -> (tempfile::TempDir, Run, CodexProvider) {
    let root = tempfile::tempdir().unwrap();
    let binary = root.path().join("codex-fixture");
    let script = format!(
        "#!/usr/bin/env python3\nimport sys,json,os,time,pathlib\nif '--version' in sys.argv: print('codex-cli 0.162.1');sys.exit(0)\nos.chdir(pathlib.Path(__file__).parent)\n{body}\n"
    );
    std::fs::write(&binary, script).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().to_string_lossy().into();
    let run = Run::new(architect::id(), spec).unwrap();
    let provider = CodexProvider {
        binary: Some(binary.to_string_lossy().into()),
        ..Default::default()
    };
    (root, run, provider)
}

const VALID_EVENTS: &str = "sys.stdin.read()\nprint(json.dumps(dict(type='thread.started',thread_id='00000000-0000-4000-8000-000000000001')),flush=True)\nprint(json.dumps(dict(type='item.completed',item=dict(type='agent_message',text='{}'))),flush=True)\nprint(json.dumps(dict(type='turn.completed')),flush=True)";

#[test]
fn codex_resume_rejects_thread_names_and_option_like_ids() {
    let (_root, run, provider) = fixture(
        "sys.stdin.read()\nsession=sys.argv[sys.argv.index('resume')+1]\nprint(json.dumps(dict(type='thread.started',thread_id=session)),flush=True)\nprint(json.dumps(dict(type='item.completed',item=dict(type='agent_message',text='{}'))),flush=True)\nprint(json.dumps(dict(type='turn.completed')),flush=True)",
    );
    for alias in [
        "friendly-thread-name",
        "--dangerously-bypass-approvals-and-sandbox",
    ] {
        let result = provider.turn_receipt(
            &TurnRequest {
                turn_id: "resume-exact-id",
                run: &run,
                prompt: "test",
                schema: &serde_json::json!({}),
                resume: Some(alias),
            },
            Arc::new(AtomicBool::new(false)),
            &mut |_| Ok(()),
        );
        assert!(
            matches!(result, Err(architect::Error::Provider(error)) if error.kind == FailureKind::InvalidSession),
            "accepted non-ID resume argument {alias}"
        );
    }
}

#[test]
fn codex_rejects_events_after_terminal_and_forbidden_tool_events() {
    for extra in [
        "print(json.dumps(dict(type='turn.completed')),flush=True)",
        "print(json.dumps(dict(type='item.completed',item=dict(type='agent_message',text='{\"late\":true}'))),flush=True)",
        "print(json.dumps(dict(type='item.completed',item=dict(type='command_execution',command='touch forbidden',exit_code=0))),flush=True)",
    ] {
        let (_root, run, provider) = fixture(&format!("{VALID_EVENTS}\n{extra}"));
        let result = provider.turn(
            &run,
            "test",
            &serde_json::json!({}),
            None,
            Arc::new(AtomicBool::new(false)),
            &mut |_| Ok(()),
        );
        assert!(result.is_err(), "accepted invalid event sequence: {extra}");
    }
}

fn cancellation_counterexample(body: &str, prompt: String) {
    let (root, run, provider) = fixture(body);
    let started = root.path().join("started");
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = cancelled.clone();
    let worker = std::thread::spawn(move || {
        provider.turn(
            &run,
            &prompt,
            &serde_json::json!({}),
            None,
            worker_cancelled,
            &mut |_| Ok(()),
        )
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !started.exists() {
        assert!(Instant::now() < deadline, "fixture did not start");
        std::thread::sleep(Duration::from_millis(5));
    }
    let start = Instant::now();
    cancelled.store(true, Ordering::SeqCst);
    let result = worker.join().unwrap();
    assert!(result.is_err());
    assert!(
        start.elapsed() < Duration::from_millis(800),
        "cancellation blocked for {:?}",
        start.elapsed()
    );
}

#[test]
fn codex_cancellation_handles_a_full_stdin_pipe() {
    cancellation_counterexample(
        "pathlib.Path('started').write_text('ready')\ntime.sleep(2.5)",
        "x".repeat(1_000_000),
    );
}

#[test]
fn codex_scalar_preflight_rejects_before_probe_delivery_or_schema_effects() {
    const LIMIT: usize = 1_048_576;
    let (root, run, provider) = fixture(&format!(
        "pathlib.Path('turn-started').touch()\n{VALID_EVENTS}"
    ));
    let binary = root.path().join("codex-fixture");
    let body = std::fs::read_to_string(&binary).unwrap().replace(
        "if '--version' in sys.argv:",
        "pathlib.Path(__file__).with_suffix('.probe').touch()\nif '--version' in sys.argv:",
    );
    std::fs::write(&binary, body).unwrap();
    for prompt in [
        "x".repeat(LIMIT + 1),
        "🧠".repeat(LIMIT + 1),
        format!("{}x", "e\u{301}".repeat(LIMIT / 2)),
    ] {
        let mut events = vec![];
        let result = provider.turn_receipt(
            &TurnRequest {
                turn_id: "scalar-preflight-rejected",
                run: &run,
                prompt: &prompt,
                schema: &serde_json::json!({}),
                resume: None,
            },
            Arc::new(AtomicBool::new(false)),
            &mut |event| {
                events.push(event.clone());
                Ok(())
            },
        );
        let architect::Error::Provider(failure) = result.unwrap_err() else {
            panic!("expected typed scalar-limit failure")
        };
        assert_eq!(failure.kind, FailureKind::Protocol);
        assert!(failure.message.contains("input_too_large"));
        assert!(!failure.partial_effects && failure.stopped);
        assert!(failure.session_id.is_none());
        assert!(
            events.is_empty(),
            "rejected input must have no delivery events"
        );
        assert!(!binary.with_extension("probe").exists());
        assert!(!root.path().join("turn-started").exists());
    }
    // Four-megabyte UTF-8 input is valid at exactly the scalar-count boundary.
    let prompt = "🧠".repeat(LIMIT);
    assert!(prompt.len() > LIMIT);
    let receipt = provider
        .turn_receipt(
            &TurnRequest {
                turn_id: "scalar-preflight-boundary",
                run: &run,
                prompt: &prompt,
                schema: &serde_json::json!({}),
                resume: None,
            },
            Arc::new(AtomicBool::new(false)),
            &mut |_| Ok(()),
        )
        .unwrap();
    assert!(receipt.completed);
    assert!(binary.with_extension("probe").exists());
    assert!(root.path().join("turn-started").exists());
}

#[test]
fn codex_cancellation_handles_closed_stdout_before_exit() {
    cancellation_counterexample(
        "sys.stdin.read()\nos.close(1)\npathlib.Path('started').write_text('ready')\ntime.sleep(2.5)",
        "test".into(),
    );
}

#[test]
#[ignore = "OS containment acceptance pending: the process-group profile cannot contain descendants that call setsid"]
fn supervised_checks_do_not_leave_detached_descendants() {
    let root = tempfile::tempdir().unwrap();
    let script = "import subprocess,pathlib\np=subprocess.Popen(['python3','-c',\"import time,pathlib;time.sleep(.6);pathlib.Path('detached-effect').write_text('orphan')\"], start_new_session=True,stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)\npathlib.Path('detached.pid').write_text(str(p.pid))";
    architect::workspace::check(
        root.path(),
        &["python3".into(), "-c".into(), script.into()],
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(850));
    let escaped = root.path().join("detached-effect").exists();
    assert!(
        !escaped,
        "detached check descendant survived guardian cleanup"
    );
}

#[test]
fn real_installed_cli_probes_do_not_inherit_node_preloads() {
    const CHILD: &str = "ARCHITECT_INDEPENDENT_NODE_PROBE_CHILD";
    if std::env::var_os(CHILD).is_some() {
        use architect::providers::{claude::ClaudeProvider, gemini::GeminiProvider};
        let probes = [
            CodexProvider::default().probe(),
            ClaudeProvider::default().probe(),
            GeminiProvider {
                binary: Some("/tmp/architect-provider-adapters/node_modules/.bin/gemini".into()),
                ..Default::default()
            }
            .probe(),
        ];
        for probe in probes {
            match probe {
                Ok(_) => {}
                // Version rejection happens AFTER the actual --version probe.
                // Keep checking all installed CLIs and the preload marker; this
                // is an environment-isolation test, not a version admission.
                Err(architect::Error::Provider(error))
                    if error.kind == FailureKind::Unsupported
                        && error.message == "CLI version is outside the validated contract" =>
                {
                    assert!(error.stopped && !error.partial_effects && error.session_id.is_none());
                    eprintln!(
                        "{}: installed version rejected before inference; preload guard still checked",
                        error.provider
                    );
                }
                Err(error) => panic!("installed CLI environment probe failed: {error}"),
            }
        }
        return;
    }
    // This optional installed-binary smoke must not make the portable suite
    // depend on all three providers being installed.
    if !std::path::Path::new("/tmp/architect-provider-adapters/node_modules/.bin/gemini").exists()
        || !["codex", "claude"].iter().all(|name| {
            std::env::var_os("PATH").is_some_and(|paths| {
                std::env::split_paths(&paths).any(|directory| directory.join(name).is_file())
            })
        })
    {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("preload-executed");
    let preload = root.path().join("preload.cjs");
    std::fs::write(
        &preload,
        format!(
            "require('node:fs').writeFileSync({},'executed');",
            serde_json::to_string(marker.to_str().unwrap()).unwrap()
        ),
    )
    .unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "real_installed_cli_probes_do_not_inherit_node_preloads",
        ])
        .env(CHILD, "1")
        .env("NODE_OPTIONS", format!("--require={}", preload.display()))
        .status()
        .unwrap();
    assert!(status.success());
    assert!(
        !marker.exists(),
        "an installed Node CLI executed inherited preload code during a probe"
    );
}

#[test]
fn real_codex_parser_accepts_generated_start_and_resume_arguments() {
    if !std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|directory| directory.join("codex").is_file())
    }) {
        return;
    }
    let body = format!(
        "pathlib.Path('captured-argv.json').write_text(json.dumps(sys.argv[1:]))\n{VALID_EVENTS}"
    );
    let (root, run, provider) = fixture(&body);
    for resume in [None, Some("00000000-0000-4000-8000-000000000001")] {
        provider
            .turn(
                &run,
                "test",
                &serde_json::json!({}),
                resume,
                Arc::new(AtomicBool::new(false)),
                &mut |_| Ok(()),
            )
            .unwrap();
        let argv: Vec<String> =
            serde_json::from_slice(&std::fs::read(root.path().join("captured-argv.json")).unwrap())
                .unwrap();
        // --help exercises the installed argument parser without a model turn,
        // credential lookup, or relying on the permissive protocol fixture.
        let output = std::process::Command::new("codex")
            .args(argv)
            .arg("--help")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "installed Codex rejected generated argv: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

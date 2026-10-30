use super::*;
use std::os::unix::fs::PermissionsExt;

fn test_tempdir() -> tempfile::TempDir {
    let root = std::env::current_dir().unwrap().join(".antislop/tmp");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::tempdir_in(root).unwrap()
}

/// An OpenCode that lists one new session when asked and otherwise
/// behaves as an interactive CLI (through the fixture shim).
fn opencode(dir: &Path) -> std::path::PathBuf {
    let script = dir.join("opencode");
    std::fs::write(
        &script,
        r#"#!/usr/bin/env python3
import json, os, sys, time
if "session" in sys.argv[1:] and "list" in sys.argv[1:]:
    print(json.dumps([{"id": "ses_found", "created": int(time.time() * 1000),
                       "directory": os.getcwd()}]))
    sys.exit(0)
print(json.dumps({"type": "text", "part": {"text": "opencode answer"}}))
"#,
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    fixture::interactive(&script);
    script.canonicalize().unwrap()
}

fn core(dir: &Path) -> Arc<Core> {
    let config =
        crate::config::Config::from_toml(include_str!("../../../examples/antislop.toml")).unwrap();
    Core::open(&dir.join("state"), config).unwrap()
}

#[test]
fn opencode_learns_the_session_it_created_without_locking_itself() {
    let dir = test_tempdir();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let core = core(dir.path());
    let spec = agent::Spec {
        provider: crate::config::ProviderKind::Opencode,
        binary: opencode(dir.path()),
        args: vec![],
        model: None,
        effort: None,
        cwd: project,
        add_dirs: vec![],
        access: agent::Access::ReadOnly,
        resume: None,
    };
    let session = core
        .native
        .prepare_scoped(&core, "c", "orchestrator", &spec, "")
        .unwrap();
    let (finished, result) = std::sync::mpsc::channel();
    let worker = session.clone();
    let registry = core.clone();
    std::thread::spawn(move || {
        let outcome = worker.run(&registry.native, "turn-1", "hello", &AtomicBool::new(false));
        let _ = finished.send(outcome);
    });
    let outcome = result
        .recv_timeout(Duration::from_secs(120))
        .expect("learning the identity must not deadlock")
        .unwrap();
    assert_eq!(outcome.text, "opencode answer");
    assert_eq!(outcome.session.as_deref(), Some("ses_found"));
    // Another live session in the same directory is not handed the same one.
    assert_eq!(session.identity().as_deref(), Some("ses_found"));
    assert_eq!(core.native.claimed(), ["ses_found"]);
}

#[test]
fn continuous_terminal_redraw_does_not_block_connected_first_or_followup_turns() {
    let dir = test_tempdir();
    let core = core(dir.path());
    let binary = opencode(dir.path());
    let shim_path = binary.with_extension("shim.py");
    let shim = std::fs::read_to_string(&shim_path).unwrap();
    let shim = shim.replacen(
            "    buffer = b\"\"\n",
            "    import threading, time\n    def animate():\n        while True:\n            sys.stdout.write('\\rWorking')\n            sys.stdout.flush()\n            time.sleep(0.05)\n    threading.Thread(target=animate, daemon=True).start()\n    buffer = b\"\"\n",
            1,
        );
    assert!(shim.contains("def animate():"));
    std::fs::write(shim_path, shim).unwrap();
    let spec = agent::Spec {
        provider: crate::config::ProviderKind::Opencode,
        binary,
        args: vec![],
        model: None,
        effort: None,
        cwd: dir.path().into(),
        add_dirs: vec![],
        access: agent::Access::ReadOnly,
        resume: None,
    };
    let session = core
        .native
        .prepare_scoped(&core, "c", "orchestrator", &spec, "")
        .unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let (send, receive) = std::sync::mpsc::channel();
    let worker = session.clone();
    std::thread::spawn(move || {
        let outcomes = ["first", "followup"]
            .map(|turn| worker.run(&core.native, turn, "hello", &worker_cancel));
        let _ = send.send(outcomes);
    });
    let result = receive.recv_timeout(Duration::from_secs(120));
    if result.is_err() {
        cancel.store(true, Ordering::SeqCst);
        let _ = receive.recv_timeout(Duration::from_secs(5));
    }
    for outcome in result.expect("terminal animations must not block a connected CLI") {
        assert_eq!(outcome.unwrap().text, "opencode answer");
    }
    assert!(session.gateway.connected());
}

#[test]
fn gemini_connects_and_finishes_a_turn_through_the_session_gateway() {
    let dir = test_tempdir();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let core = core(dir.path());
    let spec = agent::Spec {
        provider: crate::config::ProviderKind::Gemini,
        binary: opencode(dir.path()),
        args: vec![],
        model: None,
        effort: None,
        cwd: project,
        add_dirs: vec![],
        access: agent::Access::ReadOnly,
        resume: None,
    };
    let session = core
        .native
        .prepare_scoped(&core, "g", "verifier", &spec, "")
        .unwrap();
    let settings = session._settings.as_ref().unwrap().path();
    assert!(settings.exists());
    let outcome = session
        .run(
            &core.native,
            "gemini-turn",
            "hello",
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(outcome.text, "opencode answer");
    assert!(
        settings.exists(),
        "settings must survive for the entire PTY session"
    );
}

#[test]
fn a_session_is_replaced_when_the_wanted_one_differs_from_the_live_one() {
    let dir = test_tempdir();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let core = core(dir.path());
    let mut spec = agent::Spec {
        provider: crate::config::ProviderKind::Opencode,
        binary: opencode(dir.path()),
        args: vec![],
        model: None,
        effort: None,
        cwd: project,
        add_dirs: vec![],
        access: agent::Access::ReadOnly,
        resume: None,
    };
    let first = core
        .native
        .prepare_scoped(&core, "c", "orchestrator", &spec, "")
        .unwrap();
    // Not yet used: the same terminal serves the next request.
    let again = core
        .native
        .prepare_scoped(&core, "c", "orchestrator", &spec, "")
        .unwrap();
    assert!(Arc::ptr_eq(&first, &again));
    first.primed.store(true, Ordering::SeqCst);
    *first.identity.lock().unwrap() = Some("ses_old".into());
    // A turn that resumes the live session keeps it; one that wants a
    // fresh session (or another one) gets a new terminal.
    spec.resume = Some("ses_old".into());
    let kept = core
        .native
        .prepare_scoped(&core, "c", "orchestrator", &spec, "")
        .unwrap();
    assert!(Arc::ptr_eq(&first, &kept));
    spec.resume = None;
    let fresh = core
        .native
        .prepare_scoped(&core, "c", "orchestrator", &spec, "")
        .unwrap();
    assert!(!Arc::ptr_eq(&first, &fresh));
}

#[test]
fn cancelled_gateway_startup_keeps_the_session_retryable() {
    let dir = test_tempdir();
    let core = core(dir.path());
    let binary = dir.path().join("never-connect");
    std::fs::write(&binary, "#!/bin/sh\nexec sleep 30\n").unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
    let spec = agent::Spec {
        provider: crate::config::ProviderKind::Opencode,
        binary: binary.canonicalize().unwrap(),
        args: vec![],
        model: None,
        effort: None,
        cwd: dir.path().into(),
        add_dirs: vec![],
        access: agent::Access::ReadOnly,
        resume: None,
    };
    let session = core
        .native
        .prepare_scoped(&core, "c", "orchestrator", &spec, "")
        .unwrap();

    let error = session
        .run(
            &core.native,
            "cancelled",
            "must not be typed",
            &AtomicBool::new(true),
        )
        .unwrap_err();

    assert_eq!(error.to_string(), mcp::CANCELLED);
    assert!(!session.failed_before_start());
    assert!(!session.stop.load(Ordering::SeqCst));
    assert!(!session.primed.load(Ordering::SeqCst));
    assert!(!session.gateway.connected());
    core.native.shutdown().unwrap();
}

#[test]
fn cancelling_a_running_turn_interrupts_without_discarding_its_session() {
    let dir = test_tempdir();
    let core = core(dir.path());
    let binary = dir.path().join("slow-agent");
    let started = dir.path().join("turn-started");
    let release = dir.path().join("release-turn");
    std::fs::write(
        &binary,
        format!(
            "#!/usr/bin/env python3\nimport json, os, time\nopen({:?}, 'w').write('started')\nwhile not os.path.exists({:?}): time.sleep(0.01)\nprint(json.dumps({{'type':'text','part':{{'text':'late answer'}}}}))\n",
            started.to_string_lossy(), release.to_string_lossy()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
    fixture::interactive(&binary);
    let spec = agent::Spec {
        provider: crate::config::ProviderKind::Opencode,
        binary: binary.canonicalize().unwrap(),
        args: vec![],
        model: None,
        effort: None,
        cwd: dir.path().into(),
        add_dirs: vec![],
        access: agent::Access::ReadOnly,
        resume: None,
    };
    let session = core
        .native
        .prepare_scoped(&core, "cancel", "orchestrator", &spec, "")
        .unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let worker = session.clone();
    let registry = core.clone();
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = send.send(worker.run(&registry.native, "running-turn", "wait", &worker_cancel));
    });
    let started_by = Instant::now() + Duration::from_secs(5);
    while !started.exists() {
        assert!(
            Instant::now() < started_by,
            "fixture did not receive the prompt"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    cancel.store(true, Ordering::SeqCst);
    let error = receive
        .recv_timeout(Duration::from_secs(5))
        .expect("cancel returns promptly")
        .unwrap_err();
    assert_eq!(error.to_string(), mcp::CANCELLED);
    assert!(!session.stop.load(Ordering::SeqCst));
    assert!(session.primed.load(Ordering::SeqCst));
    core.native.shutdown().unwrap();
}

#[test]
fn failed_fixture_reports_raw_terminal_tail_and_discards_the_session() {
    let dir = test_tempdir();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let core = core(dir.path());
    let binary = dir.path().join("broken-agent");
    std::fs::write(
            &binary,
            "#!/usr/bin/env python3\nimport sys\nprint('fixture failure marker', flush=True)\nsys.exit(17)\n",
        )
        .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
    fixture::interactive(&binary);
    let spec = agent::Spec {
        provider: crate::config::ProviderKind::Opencode,
        binary: binary.canonicalize().unwrap(),
        args: vec![],
        model: None,
        effort: None,
        cwd: project,
        add_dirs: vec![],
        access: agent::Access::ReadOnly,
        resume: None,
    };
    let session = core
        .native
        .prepare_scoped(&core, "failure", "orchestrator", &spec, "")
        .unwrap();

    let error = session
        .run(
            &core.native,
            "failed-turn",
            "trigger fixture failure",
            &AtomicBool::new(false),
        )
        .unwrap_err()
        .to_string();

    assert!(error.contains(EXECUTION_FAILED), "{error}");
    assert!(error.contains("fixture failure marker"), "{error}");
    assert!(error.contains("exited ("), "{error}");
    assert!(session.stop.load(Ordering::SeqCst));
    assert!(session.check_alive().is_err());
}

#[test]
fn live_cli_that_never_connects_times_out_and_is_cleaned_up() {
    let dir = test_tempdir();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let core = core(dir.path());
    let binary = dir.path().join("never-connect");
    let pid_file = dir.path().join("never-connect.pid");
    std::fs::write(
        &binary,
        format!(
            "#!/usr/bin/env python3\nimport os, time\nopen({:?}, 'w').write(str(os.getpid()))\nprint('fixture waiting without MCP', flush=True)\nwhile True: time.sleep(1)\n",
            pid_file.to_string_lossy()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
    let spec = agent::Spec {
        provider: crate::config::ProviderKind::Opencode,
        binary: binary.canonicalize().unwrap(),
        args: vec![],
        model: None,
        effort: None,
        cwd: project,
        add_dirs: vec![],
        access: agent::Access::ReadOnly,
        resume: None,
    };
    let session = core
        .native
        .prepare_scoped(&core, "never-connect", "orchestrator", &spec, "")
        .unwrap();
    let output_deadline = Instant::now() + Duration::from_secs(5);
    while !terminal::tail(&session.screen, SCREEN_LINES).contains("fixture waiting without MCP") {
        assert!(Instant::now() < output_deadline, "fixture did not start");
        std::thread::sleep(Duration::from_millis(10));
    }
    *session.ready_limit.lock().unwrap() = Duration::from_millis(200);

    let error = session
        .run(
            &core.native,
            "never-connect-turn",
            "this prompt must not be typed",
            &AtomicBool::new(false),
        )
        .unwrap_err()
        .to_string();

    assert!(error.contains(EXECUTION_FAILED), "{error}");
    assert!(
        error.contains("did not connect to the antislop MCP gateway"),
        "{error}"
    );
    assert!(error.contains("fixture waiting without MCP"), "{error}");
    assert!(session.failed_before_start());
    assert!(!session.primed.load(Ordering::SeqCst));
    assert!(!session.gateway.connected());
    assert!(session.stop.load(Ordering::SeqCst));
    assert!(Arc::ptr_eq(
        &session,
        &core
            .native
            .get("never-connect", "orchestrator")
            .unwrap()
            .unwrap()
    ));

    let pid: i32 = std::fs::read_to_string(pid_file).unwrap().parse().unwrap();
    core.native.shutdown().unwrap();
    drop(session);
    // Session drop must kill and reap a CLI that stayed alive through the
    // readiness failure, rather than leaving a daemon child behind.
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert!(
        core.native
            .get("never-connect", "orchestrator")
            .unwrap()
            .is_none()
    );
    let process_rows: i64 = core
        .store
        .db()
        .query_row("SELECT COUNT(*) FROM native_processes", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(process_rows, 0, "the timed-out CLI must be reaped");
}

#[test]
fn exited_cli_error_contains_the_raw_terminal_tail() {
    let dir = test_tempdir();
    let core = core(dir.path());
    let binary = dir.path().join("exit-agent");
    std::fs::write(
        &binary,
        "#!/bin/sh\nprintf 'provider detail 73\\n'\nexit 17\n",
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
    let spec = agent::Spec {
        provider: crate::config::ProviderKind::Opencode,
        binary: binary.canonicalize().unwrap(),
        args: vec![],
        model: None,
        effort: None,
        cwd: dir.path().into(),
        add_dirs: vec![],
        access: agent::Access::ReadOnly,
        resume: None,
    };
    let session = core
        .native
        .prepare_scoped(&core, "c", "orchestrator", &spec, "")
        .unwrap();

    let mut observed = None;
    for _ in 0..40 {
        if let Err(error) = session.check_alive() {
            observed = Some(error.to_string());
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }

    let error = observed.expect("the process should exit promptly");
    assert!(error.contains("opencode exited ("), "{error}");
    assert!(error.contains("provider detail 73"), "{error}");
    assert!(!error.contains("automatic retry"));
    core.native.shutdown().unwrap();
}

#[test]
fn a_cli_that_prints_nothing_for_the_idle_limit_is_reported_hung_with_its_tail() {
    let dir = test_tempdir();
    let core = core(dir.path());
    let binary = dir.path().join("hung-agent");
    std::fs::write(&binary, "#!/bin/sh\nprintf 'last words 41\\n'\nsleep 30\n").unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
    let spec = agent::Spec {
        provider: crate::config::ProviderKind::Opencode,
        binary: binary.canonicalize().unwrap(),
        args: vec![],
        model: None,
        effort: None,
        cwd: dir.path().into(),
        add_dirs: vec![],
        access: agent::Access::ReadOnly,
        resume: None,
    };
    let session = core
        .native
        .prepare_scoped(&core, "c", "orchestrator", &spec, "")
        .unwrap();

    let output_deadline = Instant::now() + Duration::from_secs(5);
    while !terminal::tail(&session.screen, SCREEN_LINES).contains("last words 41") {
        assert!(
            Instant::now() < output_deadline,
            "fixture output was not captured"
        );
        std::thread::sleep(Duration::from_millis(10));
    }

    let mut observed = None;
    for _ in 0..80 {
        if let Err(error) = session.check_active(Duration::from_millis(300)) {
            observed = Some(error.to_string());
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }

    let error = observed.expect("a silent CLI should be reported hung");
    assert!(error.contains("opencode printed nothing for"), "{error}");
    assert!(error.contains("last words 41"), "{error}");
    session.check_active(Duration::from_secs(3600)).unwrap();
    core.native.shutdown().unwrap();
}

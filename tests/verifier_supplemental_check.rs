//! Independent supplemental-runner authorization, evidence and cancellation tests.
use architect::{
    domain::*,
    hash, id, recovery,
    runtime::{Command, Runtime},
    storage::Store,
};
use std::{
    process::Command as Process,
    sync::Arc,
    time::{Duration, Instant},
};
fn fixture(status: Status) -> (tempfile::TempDir, Arc<Runtime>, Run) {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("pinned.txt"), "original input").unwrap();
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().canonicalize().unwrap().to_string_lossy().into();
    spec.input_files = vec!["pinned.txt".into()];
    let runtime =
        Arc::new(Runtime::new(Store::open(&root.path().join("state.db")).unwrap()).unwrap());
    let mut run = Run::new(id(), spec).unwrap();
    run.stage = Stage::Execution;
    run.attempt = 3;
    run.status = status;
    let content = "{\"artifact\":\"current executor output\"}".to_string();
    run.artifacts.push(Artifact {
        id: "current-artifact".into(),
        stage: Stage::Execution,
        attempt: 3,
        spec_revision: 1,
        session_id: "already-observed-executor".into(),
        hash: hash(&content),
        content,
    });
    runtime
        .store
        .lock()
        .unwrap()
        .save(&run, "independent_fixture", None)
        .unwrap();
    (root, runtime, run)
}
fn argv(script: &str) -> Vec<String> {
    vec!["python3".into(), "-c".into(), script.into()]
}
fn check(run: &Run, argv: Vec<String>) -> Command {
    Command::Check {
        run_id: run.id.clone(),
        scope_hash: run.check_scope_hash(&argv).unwrap(),
        argv,
    }
}
fn read(runtime: &Arc<Runtime>, run: &Run) -> Run {
    runtime.store.lock().unwrap().get(&run.id).unwrap()
}
fn wait(runtime: &Arc<Runtime>, run: &Run) -> Run {
    let until = Instant::now() + Duration::from_secs(8);
    loop {
        let current = read(runtime, run);
        if current.status != Status::Running && current.status != Status::Paused {
            return current;
        }
        assert!(Instant::now() < until, "supplemental worker did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn marker_script() -> &'static str {
    "from pathlib import Path; p=Path('effect-count'); p.write_text((p.read_text() if p.exists() else '')+'x'); print('actual check output')"
}
#[test]
fn stale_scope_and_swapped_argv_are_rejected_before_any_effect_or_approval() {
    let (root, runtime, run) = fixture(Status::AwaitingResult);
    let authorized = argv(marker_script());
    assert!(
        runtime
            .dispatch(
                &id(),
                Command::Check {
                    run_id: run.id.clone(),
                    argv: authorized.clone(),
                    scope_hash: "stale".into()
                }
            )
            .is_err()
    );
    assert!(
        runtime
            .dispatch(
                &id(),
                Command::Check {
                    run_id: run.id.clone(),
                    argv: argv("from pathlib import Path; Path('swapped').write_text('effect')"),
                    scope_hash: run.check_scope_hash(&authorized).unwrap()
                }
            )
            .is_err()
    );
    assert!(!root.path().join("effect-count").exists());
    assert!(!root.path().join("swapped").exists());
    let after = read(&runtime, &run);
    assert!(after.approvals.is_empty());
    assert_eq!(after.status, run.status);
    assert_eq!(after.evidence.len(), run.evidence.len());
}
#[test]
fn authorized_check_is_idempotent_and_receipt_binds_exact_argv_inputs_and_artifact() {
    let (root, runtime, mut run) = fixture(Status::AwaitingResult);
    let refresh_content = "{\"artifact\":\"later context refresh\"}".to_string();
    run.artifacts.push(Artifact {
        id: "later-refresh".into(),
        stage: Stage::Discovery,
        attempt: 3,
        spec_revision: 1,
        session_id: "already-observed-refresh".into(),
        hash: hash(&refresh_content),
        content: refresh_content,
    });
    runtime
        .store
        .lock()
        .unwrap()
        .save(&run, "later_refresh_fixture", None)
        .unwrap();
    let command = check(&run, argv(marker_script()));
    let key = id();
    let started = runtime.dispatch(&key, command.clone()).unwrap();
    assert_eq!(started.status, Status::Running);
    assert!(started.reconciliation_required);
    runtime.dispatch(&key, command.clone()).unwrap();
    let after = wait(&runtime, &run);
    runtime.dispatch(&key, command).unwrap();
    assert_eq!(
        std::fs::read_to_string(root.path().join("effect-count")).unwrap(),
        "x"
    );
    assert_eq!(after.status, Status::AwaitingResult);
    assert!(!after.reconciliation_required);
    assert_eq!(after.stage, run.stage);
    assert_eq!(after.spec_revision, run.spec_revision);
    assert_eq!(after.attempt, run.attempt);
    assert_eq!(after.artifacts.len(), run.artifacts.len());
    assert_eq!(after.reviews.len(), run.reviews.len());
    assert_eq!(after.session_ledger, run.session_ledger);
    let approved = &after.approvals[0];
    assert_eq!(
        approved.action,
        Run::check_action(&argv(marker_script())).unwrap()
    );
    assert_eq!(
        approved.scope_hash,
        run.check_scope_hash(&argv(marker_script())).unwrap()
    );
    let source = format!(
        "runner:{}",
        serde_json::to_string(&argv(marker_script())).unwrap()
    );
    let receipt = after.evidence.iter().find(|e| e.source == source).unwrap();
    assert!(receipt.success);
    assert_eq!(receipt.artifact_hash, run.artifacts[0].hash);
    assert_eq!(receipt.spec_revision, 1);
    assert_eq!(
        receipt.input_hash,
        Some(hash(serde_json::to_vec(&run.input_snapshot).unwrap()))
    );
    assert_eq!(receipt.output_hash, hash(&receipt.output));
    let output: serde_json::Value = serde_json::from_str(&receipt.output).unwrap();
    assert_eq!(output["argv"], serde_json::json!(argv(marker_script())));
    assert_eq!(output["exit_code"], 0);
    assert_eq!(output["stdout"], "actual check output\n");
    let events = runtime.store.lock().unwrap().events(0).unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|e| e.kind == "supplemental_check_started")
            .count(),
        1
    );
    assert!(!events.iter().any(|e| e.kind == "turn_reserved"
        || e.kind == "provider_event"
        || e.kind == "attempt_started"));
    assert!(
        runtime
            .dispatch(&key, check(&after, argv("print('different')")))
            .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("effect-count")).unwrap(),
        "x"
    );
}
#[test]
fn initially_paused_running_or_uncertain_states_never_launch_a_check() {
    for status in [Status::Paused, Status::Running, Status::Blocked] {
        let (root, runtime, mut run) = fixture(status);
        if status == Status::Blocked {
            run.reconciliation_required = true;
            runtime
                .store
                .lock()
                .unwrap()
                .save(&run, "uncertain_fixture", None)
                .unwrap();
        }
        assert!(
            runtime
                .dispatch(&id(), check(&run, argv(marker_script())))
                .is_err()
        );
        assert!(!root.path().join("effect-count").exists());
        assert!(read(&runtime, &run).approvals.is_empty());
    }
}
#[test]
fn successful_and_failed_checks_restore_previous_status_without_autonomous_progress() {
    for status in [
        Status::AwaitingStart,
        Status::Ready,
        Status::ChangesRequested,
        Status::Blocked,
    ] {
        for exit in [0, 7] {
            let (_root, runtime, run) = fixture(status);
            let arguments = argv(&format!("import sys; print('probe'); sys.exit({exit})"));
            runtime
                .dispatch(&id(), check(&run, arguments.clone()))
                .unwrap();
            let after = wait(&runtime, &run);
            assert_eq!(after.status, status);
            assert_eq!(after.stage, run.stage);
            assert_eq!(after.spec_revision, 1);
            assert_eq!(after.attempt, 3);
            assert!(!after.reconciliation_required);
            assert!(after.session_ledger.sessions.is_empty());
            let receipt = after
                .evidence
                .iter()
                .find(|e| {
                    e.source == format!("runner:{}", serde_json::to_string(&arguments).unwrap())
                })
                .unwrap();
            assert_eq!(receipt.success, exit == 0);
        }
    }
}
#[test]
fn check_that_edits_pinned_input_blocks_acceptance_and_repair_until_reconciliation() {
    let (root, runtime, run) = fixture(Status::AwaitingResult);
    runtime.dispatch(&id(),check(&run,argv("from pathlib import Path; Path('pinned.txt').write_text('changed by authorized check'); print('done')"))).unwrap();
    let blocked = wait(&runtime, &run);
    assert_eq!(blocked.status, Status::Blocked);
    assert!(blocked.reconciliation_required);
    assert_eq!(blocked.input_snapshot, run.input_snapshot);
    assert_eq!(blocked.attempt, 3);
    assert!(
        blocked
            .evidence
            .iter()
            .any(|e| e.source.starts_with("runner:") && !e.success)
    );
    assert!(
        runtime
            .dispatch(
                &id(),
                Command::AcceptResult {
                    run_id: run.id.clone(),
                    scope_hash: blocked.scope_hash("accept")
                }
            )
            .is_err()
    );
    assert!(
        runtime
            .dispatch(
                &id(),
                Command::ApproveRepair {
                    run_id: run.id.clone(),
                    scope_hash: blocked.scope_hash("repair")
                }
            )
            .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("pinned.txt")).unwrap(),
        "changed by authorized check"
    );
    let reconciled = runtime
        .dispatch(
            &id(),
            Command::Reconcile {
                run_id: run.id.clone(),
                scope_hash: blocked.scope_hash("reconcile"),
                input_hash: recovery::observe(&blocked).unwrap().hash,
            },
        )
        .unwrap();
    assert!(!reconciled.reconciliation_required);
    assert_eq!(reconciled.stage, Stage::Discovery);
    assert_eq!(reconciled.spec_revision, 2);
    assert_eq!(
        reconciled.input_snapshot["pinned.txt"],
        "changed by authorized check"
    );
}
#[test]
fn cancelling_hung_check_is_responsive_and_kills_descendants_without_replay() {
    let (root, runtime, run) = fixture(Status::AwaitingResult);
    let script = "import subprocess,pathlib,time; subprocess.Popen(['python3','-c',\"import time,pathlib; time.sleep(2); pathlib.Path('late-effect').write_text('orphan')\"]); pathlib.Path('started').write_text('ready'); time.sleep(30)";
    runtime.dispatch(&id(), check(&run, argv(script))).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !root.path().join("started").exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let start = Instant::now();
    let cancelled = runtime
        .dispatch(
            &id(),
            Command::Cancel {
                run_id: run.id.clone(),
            },
        )
        .unwrap();
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_eq!(cancelled.status, Status::Cancelled);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let finished = runtime
            .store
            .lock()
            .unwrap()
            .events(0)
            .unwrap()
            .iter()
            .any(|e| e.kind == "supplemental_check_finished");
        if finished {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let after = read(&runtime, &run);
    assert_eq!(after.status, Status::Cancelled);
    assert!(after.reconciliation_required);
    assert_eq!(after.attempt, 3);
    assert!(after.session_ledger.sessions.is_empty());
    std::thread::sleep(Duration::from_millis(2200));
    assert!(!root.path().join("late-effect").exists());
}
#[test]
fn cli_requires_exact_authorization_before_contacting_ipc() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("absent-state");
    let args = root.path().join("argv.json");
    std::fs::write(&args, serde_json::to_vec(&argv(marker_script())).unwrap()).unwrap();
    for suffix in [vec![], vec!["--wrong"], vec!["--approve-scoped", "extra"]] {
        let output = Process::new(env!("CARGO_BIN_EXE_antislop"))
            .arg("check")
            .arg(&state)
            .arg("run-id")
            .arg(&args)
            .args(suffix)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("--approve-scoped"));
        assert!(!state.exists());
        assert!(!root.path().join("effect-count").exists());
    }
}

#[test]
fn follow_does_not_invent_supplemental_check_authorization_for_a_blocked_run() {
    use architect::ipc::{self, Operation, Request};
    struct Daemon(std::process::Child);
    impl Drop for Daemon {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let (root, runtime, mut run) = fixture(Status::Blocked);
    run.diagnostic =
        Some("An additional local probe requires explicit supplemental check authorization".into());
    runtime
        .store
        .lock()
        .unwrap()
        .save(&run, "supplemental_required_fixture", None)
        .unwrap();
    drop(runtime);
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut daemon = Daemon(
        Process::new(env!("CARGO_BIN_EXE_antislop"))
            .arg("daemon")
            .arg(root.path())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if ipc::call(
            root.path(),
            &Request {
                version: ipc::PROTOCOL_VERSION,
                request_id: id(),
                idempotency_key: None,
                operation: Operation::List,
            },
        )
        .is_ok()
        {
            break;
        }
        assert!(daemon.0.try_wait().unwrap().is_none());
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    let error = architect::drive::follow(
        root.path(),
        &run.id,
        run.spec.clone(),
        9,
        Duration::from_secs(3),
    )
    .unwrap_err();
    assert!(error.to_string().contains("Blocked"));
    let persisted: Run =
        serde_json::from_str(&ipc::read_document(root.path(), &run.id, "run").unwrap()).unwrap();
    assert!(persisted.approvals.is_empty());
    assert_eq!(persisted.status, Status::Blocked);
    assert_eq!(persisted.attempt, 3);
    assert!(persisted.session_ledger.sessions.is_empty());
    assert!(!root.path().join("effect-count").exists());
}

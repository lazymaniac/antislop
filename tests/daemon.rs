use architect::{
    domain::Run,
    id,
    ipc::{Operation, Request, call},
    runtime::Command,
};
use std::{
    path::Path,
    process::{Child, Command as ProcessCommand, Stdio},
    time::{Duration, Instant},
};

struct Daemon(Child);
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn start(state: &Path) -> Daemon {
    let process = ProcessCommand::new(env!("CARGO_BIN_EXE_antislop"))
        .arg("daemon")
        .arg(state)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let daemon = Daemon(process);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if call(
            state,
            &Request {
                version: architect::ipc::PROTOCOL_VERSION,
                request_id: id(),
                idempotency_key: None,
                operation: Operation::List,
            },
        )
        .is_ok()
        {
            return daemon;
        }
        assert!(Instant::now() < deadline, "daemon startup failed");
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn mutate(state: &Path, key: &str, command: Command) -> Run {
    let response = call(
        state,
        &Request {
            version: architect::ipc::PROTOCOL_VERSION,
            request_id: id(),
            idempotency_key: Some(key.into()),
            operation: Operation::Mutate {
                command: Box::new(command),
            },
        },
    )
    .unwrap();
    assert!(response.error.is_none(), "{:?}", response.error);
    {
        let summary = response.data.unwrap();
        serde_json::from_str(
            &architect::ipc::read_document(state, summary["id"].as_str().unwrap(), "run").unwrap(),
        )
        .unwrap()
    }
}

#[test]
fn repeated_frontend_reads_reject_changed_database_bytes_with_a_retained_hash() {
    use architect::tui::backend::{Backend, IpcBackend};
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state");
    let workspace = root.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::write(workspace.join("source"), "original").unwrap();
    let mut spec: architect::domain::RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = workspace.to_string_lossy().into();
    spec.input_files = vec!["source".into()];
    let _daemon = start(&state);
    let first = mutate(&state, "first", Command::Create { spec: spec.clone() });
    let second = mutate(&state, "second", Command::Create { spec });
    let mut backend = IpcBackend::new(&state);
    for _ in 0..2 {
        assert_eq!(backend.snapshot(&first.id).unwrap().run_id, first.id);
    }
    let db = rusqlite::Connection::open(state.join("state.db")).unwrap();
    let (original, digest): (String, String) = db
        .query_row(
            "SELECT state,state_hash FROM runs WHERE id=?1",
            [&first.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let mut altered: serde_json::Value = serde_json::from_str(&original).unwrap();
    altered["diagnostic"] = serde_json::json!("changed canonical bytes");
    let altered = serde_json::to_string(&altered).unwrap();
    // Simulate an out-of-band row edit that omits the transactional hash.
    db.execute(
        "UPDATE runs SET state=?1 WHERE id=?2",
        (&altered, &first.id),
    )
    .unwrap();
    let error = backend.snapshot(&first.id).unwrap_err().to_string();
    assert!(error.contains("document hash mismatch"), "{error}");
    db.execute(
        "UPDATE runs SET state_hash=?1 WHERE id=?2",
        (architect::hash(&altered), &first.id),
    )
    .unwrap();
    assert_eq!(
        backend.snapshot(&first.id).unwrap().diagnostic,
        "changed canonical bytes"
    );
    assert_eq!(backend.snapshot(&second.id).unwrap().run_id, second.id);
    db.execute(
        "UPDATE runs SET state=?1,state_hash=?2 WHERE id=?3",
        (&original, &digest, &first.id),
    )
    .unwrap();
    let restored = backend.snapshot(&first.id).unwrap();
    assert_eq!(restored.run_id, first.id);
    assert!(restored.diagnostic.is_empty());
}

#[test]
fn daemon_crash_after_check_effect_does_not_replay_and_keeps_approval() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state");
    // The fixture exits when its parent daemon dies, avoiding an orphaned check.
    std::fs::write(root.path().join("check.py"),"import os,time,pathlib\np=pathlib.Path('effect')\np.write_text(p.read_text()+'x' if p.exists() else 'x')\nparent=os.getppid()\nwhile os.getppid()==parent: time.sleep(.01)\n").unwrap();
    let mut spec: architect::domain::RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().to_string_lossy().into();
    spec.check_command = vec!["python3".into(), "check.py".into()];
    spec.input_files = vec!["check.py".into()];
    let mut daemon = start(&state);
    let run = mutate(&state, "create", Command::Create { spec });
    mutate(
        &state,
        "approve",
        Command::ApproveStart {
            run_id: run.id.clone(),
            scope_hash: run.scope_hash("start"),
        },
    );
    mutate(
        &state,
        "advance",
        Command::Advance {
            run_id: run.id.clone(),
        },
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while !root.path().join("effect").exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    daemon.0.kill().unwrap();
    daemon.0.wait().unwrap();
    drop(daemon);
    let _daemon = start(&state);
    let response = call(
        &state,
        &Request {
            version: architect::ipc::PROTOCOL_VERSION,
            request_id: id(),
            idempotency_key: None,
            operation: Operation::Snapshot {
                run_id: run.id.clone(),
            },
        },
    )
    .unwrap();
    assert!(response.error.is_none());
    let recovered: Run =
        serde_json::from_str(&architect::ipc::read_document(&state, &run.id, "run").unwrap())
            .unwrap();
    assert_eq!(recovered.status, architect::domain::Status::Blocked);
    assert_eq!(recovered.approvals.len(), 1);
    // A retry of the original request returns its durable receipt, without relaunch.
    mutate(
        &state,
        "advance",
        Command::Advance {
            run_id: run.id.clone(),
        },
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("effect")).unwrap(),
        "x"
    );
    let response = call(
        &state,
        &Request {
            version: architect::ipc::PROTOCOL_VERSION,
            request_id: id(),
            idempotency_key: Some(id()),
            operation: Operation::Mutate {
                command: Box::new(Command::Advance { run_id: run.id }),
            },
        },
    )
    .unwrap();
    assert!(response.error.is_some());
}
#[test]
fn second_daemon_cannot_take_over_socket_or_database() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state");
    let _daemon = start(&state);
    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_antislop"))
        .arg("daemon")
        .arg(&state)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let response = call(
        &state,
        &Request {
            version: architect::ipc::PROTOCOL_VERSION,
            request_id: id(),
            idempotency_key: None,
            operation: Operation::List,
        },
    )
    .unwrap();
    assert!(response.error.is_none());
}

#[test]
fn restart_of_a_worker_waiting_at_pause_preserves_pause_and_never_replays_the_check() {
    use architect::domain::{RunSpec, Status};
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state");
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().to_string_lossy().into();
    spec.check_command = vec![
        "/bin/sh".into(),
        "-c".into(),
        "printf x >> effect; while [ ! -f release ]; do sleep 0.01; done".into(),
    ];
    let daemon = start(&state);
    let run = mutate(&state, "create", Command::Create { spec });
    mutate(
        &state,
        "approve",
        Command::ApproveStart {
            run_id: run.id.clone(),
            scope_hash: run.scope_hash("start"),
        },
    );
    mutate(
        &state,
        "advance",
        Command::Advance {
            run_id: run.id.clone(),
        },
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while !root.path().join("effect").exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    mutate(
        &state,
        "pause",
        Command::Pause {
            run_id: run.id.clone(),
        },
    );
    std::fs::write(root.path().join("release"), "").unwrap();
    loop {
        let response = call(
            &state,
            &Request {
                version: architect::ipc::PROTOCOL_VERSION,
                request_id: id(),
                idempotency_key: None,
                operation: Operation::Events { after: 0 },
            },
        )
        .unwrap();
        if response.data.unwrap()["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "work_waiting_paused")
        {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    drop(daemon);
    let _replacement = start(&state);
    let recovered: Run =
        serde_json::from_str(&architect::ipc::read_document(&state, &run.id, "run").unwrap())
            .unwrap();
    assert_eq!(recovered.status, Status::Paused);
    assert_eq!(recovered.paused_from, Some(Status::Blocked));
    assert!(recovered.reconciliation_required);
    assert!(recovered.session_ledger.sessions.is_empty());
    let resumed = mutate(
        &state,
        "resume",
        Command::Resume {
            run_id: run.id.clone(),
        },
    );
    assert_eq!(resumed.status, Status::Blocked);
    assert_eq!(resumed.attempt, 1);
    assert_eq!(resumed.approvals.len(), 1);
    assert_eq!(
        std::fs::read_to_string(root.path().join("effect")).unwrap(),
        "x"
    );
}

//! Independent client-state and deadline regressions; no live model calls.
use architect::{
    domain::{Run, RunSpec, Status},
    drive,
    ipc::{self, Operation, Request, Response},
};
use std::{
    io::{BufReader, Write},
    os::unix::{fs::PermissionsExt, net::UnixListener},
    path::Path,
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
fn spec(path: &Path) -> RunSpec {
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = path.canonicalize().unwrap().to_string_lossy().into();
    spec
}
fn scripted_follow(
    mut run: Run,
    expected: RunSpec,
    snapshot_status: Status,
    timeout: Duration,
    snapshot_delay: Duration,
) -> (architect::Result<Run>, Vec<String>) {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state");
    ipc::private_directory(&state).unwrap();
    run.id = "follow-run".into();
    let document = serde_json::to_string(&run).unwrap();
    let hash = architect::hash(&document);
    let listener = UnixListener::bind(state.join("architect.sock")).unwrap();
    listener.set_nonblocking(true).unwrap();
    std::fs::set_permissions(
        state.join("architect.sock"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let server_stop = stop.clone();
    let operations = Arc::new(Mutex::new(vec![]));
    let recorded = operations.clone();
    let server = std::thread::spawn(move || {
        while !server_stop.load(Ordering::SeqCst) {
            let (mut stream, _) = match listener.accept() {
                Ok(pair) => pair,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(2));
                    continue;
                }
                Err(error) => panic!("{error}"),
            };
            stream.set_nonblocking(false).unwrap();
            let request: Request = architect::transport::read(&mut BufReader::new(&stream))
                .unwrap()
                .unwrap();
            let mut error = None;
            let data = match request.operation {
                Operation::Snapshot { .. } => {
                    recorded.lock().unwrap().push("snapshot".into());
                    std::thread::sleep(snapshot_delay);
                    serde_json::json!({"run":{"status":snapshot_status,"stage":"discovery"}})
                }
                Operation::Read { offset, .. } => {
                    recorded.lock().unwrap().push("read".into());
                    assert_eq!(offset, 0);
                    serde_json::json!({"hash":hash,"chunk":document,"next_offset":null})
                }
                Operation::Mutate { command } => {
                    recorded.lock().unwrap().push(
                        serde_json::to_value(command).unwrap()["type"]
                            .as_str()
                            .unwrap()
                            .into(),
                    );
                    error = Some("verifier blocked unexpected mutation".into());
                    serde_json::Value::Null
                }
                _ => panic!("follow must not create/list/reset a run"),
            };
            let response = Response {
                version: ipc::PROTOCOL_VERSION,
                request_id: request.request_id,
                data: Some(data),
                error,
            };
            serde_json::to_writer(&mut stream, &response).unwrap();
            stream.write_all(b"\n").unwrap();
        }
    });
    let result = drive::follow(&state, "follow-run", expected, 3, timeout);
    stop.store(true, Ordering::SeqCst);
    server.join().unwrap();
    let ops = operations.lock().unwrap().clone();
    (result, ops)
}
#[test]
fn paused_full_document_wins_over_older_awaiting_start_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let expected = spec(root.path());
    let mut run = Run::new("original".into(), expected.clone()).unwrap();
    run.status = Status::Paused;
    run.paused_from = Some(Status::AwaitingStart);
    let (result, ops) = scripted_follow(
        run,
        expected,
        Status::AwaitingStart,
        Duration::MAX,
        Duration::ZERO,
    );
    assert!(result.unwrap_err().to_string().contains("Paused"));
    assert_eq!(ops, vec!["snapshot", "read"]);
}
#[test]
fn completed_full_document_does_not_reapprove_older_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let expected = spec(root.path());
    let mut run = Run::new("original".into(), expected.clone()).unwrap();
    run.status = Status::Completed;
    let (result, ops) = scripted_follow(
        run,
        expected,
        Status::AwaitingStart,
        Duration::MAX,
        Duration::ZERO,
    );
    assert_eq!(result.unwrap().status, Status::Completed);
    assert_eq!(ops, vec!["snapshot", "read"]);
}
#[test]
fn running_attach_checks_scope_before_its_first_wait() {
    let root = tempfile::tempdir().unwrap();
    let expected = spec(root.path());
    let mut run = Run::new("original".into(), expected.clone()).unwrap();
    run.status = Status::Running;
    run.reconciliation_required = true;
    run.spec.objective.push_str(" unauthorized drift");
    let (result, ops) = scripted_follow(
        run,
        expected,
        Status::Running,
        Duration::from_secs(2),
        Duration::ZERO,
    );
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("task scope changed")
    );
    assert_eq!(ops, vec!["snapshot", "read"]);
}
#[test]
fn uncertain_ready_document_never_advances_or_reconciles() {
    let root = tempfile::tempdir().unwrap();
    let expected = spec(root.path());
    let mut run = Run::new("original".into(), expected.clone()).unwrap();
    run.status = Status::Ready;
    run.reconciliation_required = true;
    let (result, ops) =
        scripted_follow(run, expected, Status::Ready, Duration::MAX, Duration::ZERO);
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("uncertain effects require explicit reconciliation")
    );
    assert_eq!(ops, vec!["snapshot", "read"]);
}
#[test]
fn explicit_deadline_expiry_during_observation_never_submits_a_control_action() {
    let root = tempfile::tempdir().unwrap();
    let expected = spec(root.path());
    let mut run = Run::new("original".into(), expected.clone()).unwrap();
    run.status = Status::Ready;
    let (result, ops) = scripted_follow(
        run,
        expected,
        Status::Ready,
        Duration::from_millis(30),
        Duration::from_millis(120),
    );
    assert!(
        result.unwrap_err().to_string().contains("driver timeout"),
        "operations: {ops:?}"
    );
    assert!(
        !ops.iter().any(|op| op == "advance"),
        "expired approval delegation still advanced: {ops:?}"
    );
}
#[test]
fn follow_cli_requires_authorization_and_rejects_invalid_deadline_before_ipc() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("no-daemon");
    let path = root.path().join("spec.json");
    std::fs::write(&path, serde_json::to_vec(&spec(root.path())).unwrap()).unwrap();
    for suffix in [
        vec![],
        vec!["--approve-scoped", "0", "0"],
        vec!["--approve-scoped", "0", "bad"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_antislop"))
            .arg("follow")
            .arg(&state)
            .arg("existing-run")
            .arg(&path)
            .args(&suffix)
            .output()
            .unwrap();
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            if suffix.is_empty() {
                error.contains("requires --approve-scoped")
            } else {
                error.contains("timeout")
            },
            "{error}"
        );
        assert!(!state.exists());
    }
}

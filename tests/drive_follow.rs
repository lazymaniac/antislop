use architect::{
    domain::{Run, RunSpec, Status},
    drive, id,
    ipc::{self, Operation, Request},
    runtime::Command,
};
use std::{
    path::Path,
    process::{Child, Command as Process, Stdio},
    time::{Duration, Instant},
};

struct Daemon(Child);
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn call(state: &Path, operation: Operation) -> serde_json::Value {
    let mutation = matches!(&operation, Operation::Mutate { .. });
    let response = ipc::call(
        state,
        &Request {
            version: ipc::PROTOCOL_VERSION,
            request_id: id(),
            idempotency_key: mutation.then(id),
            operation,
        },
    )
    .unwrap();
    assert!(response.error.is_none(), "{:?}", response.error);
    response.data.unwrap()
}
fn mutate(state: &Path, command: Command) -> serde_json::Value {
    call(
        state,
        Operation::Mutate {
            command: Box::new(command),
        },
    )
}
fn daemon(state: &Path) -> Daemon {
    let mut daemon = Daemon(
        Process::new(env!("CARGO_BIN_EXE_antislop"))
            .arg("daemon")
            .arg(state)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(8);
    while !state.join("architect.sock").exists() {
        assert!(daemon.0.try_wait().unwrap().is_none());
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    call(state, Operation::List);
    daemon
}
fn spec(root: &Path) -> RunSpec {
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.to_string_lossy().into();
    spec
}
fn read(state: &Path, run_id: &str) -> Run {
    serde_json::from_str(&ipc::read_document(state, run_id, "run").unwrap()).unwrap()
}

#[test]
fn attaches_to_running_run_without_creating_or_cancelling() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state");
    let _daemon = daemon(&state);
    let mut spec = spec(root.path());
    spec.check_command = vec![
        "python3".into(),
        "-c".into(),
        "import time; time.sleep(0.5)".into(),
    ];
    let created = mutate(&state, Command::Create { spec: spec.clone() });
    let run_id = created["id"].as_str().unwrap();
    let run = read(&state, run_id);
    mutate(
        &state,
        Command::ApproveStart {
            run_id: run_id.into(),
            scope_hash: run.scope_hash("start"),
        },
    );
    mutate(
        &state,
        Command::Advance {
            run_id: run_id.into(),
        },
    );
    assert_eq!(read(&state, run_id).status, Status::Running);
    let error = drive::follow(&state, run_id, spec.clone(), 0, Duration::ZERO).unwrap_err();
    assert!(error.to_string().contains("driver timeout"));
    assert_eq!(read(&state, run_id).status, Status::Running);
    // A real runner makes the fake provider's synthetic quality claims
    // insufficient. Following must stop honestly at that rejection.
    let error = drive::follow(&state, run_id, spec, 0, Duration::MAX).unwrap_err();
    assert!(error.to_string().contains("ChangesRequested"));
    let stopped = read(&state, run_id);
    assert_eq!(stopped.status, Status::ChangesRequested);
    assert!(stopped.artifacts.len() >= 3);
    assert_eq!(
        call(&state, Operation::List)["runs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn follows_completed_run_without_new_approvals_or_runs() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state");
    let _daemon = daemon(&state);
    let spec = spec(root.path());
    let completed = drive::run(&state, spec.clone(), 0, Duration::from_secs(30)).unwrap();
    let again = drive::follow(&state, &completed.id, spec, 0, Duration::MAX).unwrap();
    assert_eq!(again.status, Status::Completed);
    assert_eq!(again.approvals.len(), completed.approvals.len());
    assert_eq!(again.artifacts.len(), completed.artifacts.len());
    assert_eq!(
        call(&state, Operation::List)["runs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn reattachment_does_not_reset_persisted_repair_budget() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state");
    let _daemon = daemon(&state);
    let mut spec = spec(root.path());
    spec.objective.push_str(" [reject]");
    drive::run(&state, spec.clone(), 1, Duration::from_secs(15)).unwrap_err();
    let listing = call(&state, Operation::List);
    let run_id = listing["runs"][0]["id"].as_str().unwrap();
    let before = read(&state, run_id);
    let error = drive::follow(&state, run_id, spec, 1, Duration::MAX).unwrap_err();
    assert!(error.to_string().contains("ChangesRequested"));
    let after = read(&state, run_id);
    assert_eq!(after.attempt, before.attempt);
    assert_eq!(after.approvals.len(), before.approvals.len());
}

#[test]
fn different_scope_or_revision_receives_no_approval() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state");
    let _daemon = daemon(&state);
    let original = spec(root.path());
    let created = mutate(
        &state,
        Command::Create {
            spec: original.clone(),
        },
    );
    let run_id = created["id"].as_str().unwrap();
    let mut changed = original.clone();
    changed.objective.push_str(" unauthorized replacement");
    let error = drive::follow(&state, run_id, changed, 4, Duration::MAX).unwrap_err();
    assert!(error.to_string().contains("task scope changed"));
    assert!(read(&state, run_id).approvals.is_empty());
    mutate(
        &state,
        Command::Revise {
            run_id: run_id.into(),
            expected_revision: 1,
            spec: original.clone(),
        },
    );
    let error = drive::follow(&state, run_id, original, 4, Duration::MAX).unwrap_err();
    assert!(error.to_string().contains("task scope changed"));
    assert!(read(&state, run_id).approvals.is_empty());
}

#[test]
fn overflowing_finite_timeout_fails_before_creating_a_run() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("no-daemon");
    let duration = Duration::MAX - Duration::from_nanos(1);
    assert!(Instant::now().checked_add(duration).is_none());
    let error = drive::run(&state, spec(root.path()), 0, duration).unwrap_err();
    assert!(error.to_string().contains("clock range"), "{error}");
    assert!(!state.exists());
}

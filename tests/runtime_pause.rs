use architect::{
    domain::{Run, RunSpec, Status},
    id,
    recovery::WorkspaceLease,
    runtime::{Command, Runtime},
    storage::Store,
};
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

struct Fixture {
    root: tempfile::TempDir,
    runtime: Arc<Runtime>,
    run: Run,
}
impl Fixture {
    fn new(pinned: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let runtime =
            Arc::new(Runtime::new(Store::open(&root.path().join("state.db")).unwrap()).unwrap());
        let mut spec: RunSpec =
            serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
        spec.workspace = workspace.to_string_lossy().into();
        spec.check_command = vec![
            "/bin/sh".into(),
            "-c".into(),
            "touch started; while [ ! -f release ]; do sleep 0.01; done; touch finished".into(),
        ];
        if pinned {
            std::fs::write(workspace.join("source"), "before").unwrap();
            spec.input_files.push("source".into());
        }
        let run = runtime.dispatch(&id(), Command::Create { spec }).unwrap();
        Self { root, runtime, run }
    }
    fn workspace(&self) -> &Path {
        Path::new(&self.run.spec.workspace)
    }
    fn current(&self) -> Run {
        self.runtime
            .store
            .lock()
            .unwrap()
            .get(&self.run.id)
            .unwrap()
    }
    fn start(&self) {
        self.runtime
            .dispatch(
                &id(),
                Command::ApproveStart {
                    run_id: self.run.id.clone(),
                    scope_hash: self.run.scope_hash("start"),
                },
            )
            .unwrap();
        self.runtime
            .dispatch(
                &id(),
                Command::Advance {
                    run_id: self.run.id.clone(),
                },
            )
            .unwrap();
        wait(|| self.workspace().join("started").exists());
    }
    fn pause_at_boundary(&self) -> Run {
        let paused = self
            .runtime
            .dispatch(
                &id(),
                Command::Pause {
                    run_id: self.run.id.clone(),
                },
            )
            .unwrap();
        assert_eq!(paused.paused_from, Some(Status::Running));
        std::fs::write(self.workspace().join("release"), "").unwrap();
        wait(|| {
            self.runtime
                .store
                .lock()
                .unwrap()
                .events(0)
                .unwrap()
                .iter()
                .any(|event| event.run_id == self.run.id && event.kind == "work_waiting_paused")
        });
        let current = self.current();
        assert_eq!(current.status, Status::Paused);
        assert_eq!(current.paused_from, Some(Status::Running));
        assert!(current.artifacts.is_empty());
        assert!(current.session_ledger.sessions.is_empty());
        assert!(
            self.workspace().join("finished").exists(),
            "already admitted check may complete"
        );
        assert!(
            current
                .evidence
                .iter()
                .any(|e| e.source.starts_with("runner:baseline:") && e.success)
        );
        current
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.runtime.dispatch(
            &id(),
            Command::Cancel {
                run_id: self.run.id.clone(),
            },
        );
    }
}
fn wait(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !predicate() {
        assert!(
            Instant::now() < deadline,
            "runtime did not reach expected boundary"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn pause_finishes_active_check_but_admits_no_session_until_resume() {
    let f = Fixture::new(false);
    f.start();
    let paused = f.pause_at_boundary();
    let baseline_ids: Vec<_> = paused
        .evidence
        .iter()
        .filter(|e| e.source.starts_with("runner:baseline:"))
        .map(|e| e.id.clone())
        .collect();
    // A paused worker releases the store mutex; an unrelated workspace proceeds.
    let sibling = f.root.path().join("sibling");
    std::fs::create_dir(&sibling).unwrap();
    let mut spec = f.run.spec.clone();
    spec.workspace = sibling.to_string_lossy().into();
    spec.check_command.clear();
    let other = f.runtime.dispatch(&id(), Command::Create { spec }).unwrap();
    f.runtime
        .dispatch(
            &id(),
            Command::ApproveStart {
                run_id: other.id.clone(),
                scope_hash: other.scope_hash("start"),
            },
        )
        .unwrap();
    f.runtime
        .dispatch(
            &id(),
            Command::Advance {
                run_id: other.id.clone(),
            },
        )
        .unwrap();
    wait(|| {
        f.runtime
            .store
            .lock()
            .unwrap()
            .get(&other.id)
            .unwrap()
            .status
            != Status::Running
    });
    assert_eq!(
        f.runtime
            .store
            .lock()
            .unwrap()
            .get(&other.id)
            .unwrap()
            .status,
        Status::AwaitingResult
    );
    assert!(f.current().session_ledger.sessions.is_empty());
    assert!(
        WorkspaceLease::acquire(f.workspace().to_str().unwrap()).is_err(),
        "paused continuation retains its lease"
    );
    let key = id();
    let command = Command::Resume {
        run_id: f.run.id.clone(),
    };
    f.runtime.dispatch(&key, command.clone()).unwrap();
    f.runtime.dispatch(&key, command).unwrap();
    wait(|| f.current().status != Status::Running);
    let finished = f.current();
    assert_eq!(finished.status, Status::AwaitingResult);
    assert_eq!(finished.attempt, paused.attempt);
    assert_eq!(finished.approvals.len(), paused.approvals.len());
    assert_eq!(finished.session_ledger.sessions.len(), 2);
    assert_eq!(
        finished
            .evidence
            .iter()
            .filter(|e| e.source.starts_with("runner:baseline:"))
            .map(|e| e.id.clone())
            .collect::<Vec<_>>(),
        baseline_ids
    );
}

#[test]
fn cancelling_a_paused_worker_wakes_it_and_releases_the_lease_without_launching() {
    let f = Fixture::new(false);
    f.start();
    let paused = f.pause_at_boundary();
    f.runtime
        .dispatch(
            &id(),
            Command::Cancel {
                run_id: f.run.id.clone(),
            },
        )
        .unwrap();
    wait(|| WorkspaceLease::acquire(f.workspace().to_str().unwrap()).is_ok());
    let cancelled = f.current();
    assert_eq!(cancelled.status, Status::Cancelled);
    assert!(cancelled.session_ledger.sessions.is_empty());
    assert_eq!(cancelled.attempt, paused.attempt);
    assert!(cancelled.reconciliation_required);
}

#[test]
fn input_changes_during_pause_block_before_any_provider_turn() {
    let f = Fixture::new(true);
    f.start();
    f.pause_at_boundary();
    std::fs::write(f.workspace().join("source"), "external change").unwrap();
    f.runtime
        .dispatch(
            &id(),
            Command::Resume {
                run_id: f.run.id.clone(),
            },
        )
        .unwrap();
    wait(|| f.current().status != Status::Running);
    let blocked = f.current();
    assert_eq!(blocked.status, Status::Blocked);
    assert!(blocked.reconciliation_required);
    assert!(blocked.session_ledger.sessions.is_empty());
    assert_eq!(
        std::fs::read_to_string(f.workspace().join("source")).unwrap(),
        "external change"
    );
}

#[test]
fn failed_cancel_commit_does_not_signal_the_live_process() {
    let f = Fixture::new(false);
    f.start();
    let connection = rusqlite::Connection::open(f.root.path().join("state.db")).unwrap();
    connection.execute_batch("CREATE TRIGGER refuse_cancel BEFORE UPDATE ON runs BEGIN SELECT RAISE(ABORT, 'injected cancel failure'); END;").unwrap();
    assert!(
        f.runtime
            .dispatch(
                &id(),
                Command::Cancel {
                    run_id: f.run.id.clone()
                }
            )
            .is_err()
    );
    assert_eq!(f.current().status, Status::Running);
    connection
        .execute_batch("DROP TRIGGER refuse_cancel;")
        .unwrap();
    std::fs::write(f.workspace().join("release"), "").unwrap();
    wait(|| f.current().status != Status::Running);
    assert_eq!(f.current().status, Status::AwaitingResult);
    assert!(f.workspace().join("finished").exists());
}

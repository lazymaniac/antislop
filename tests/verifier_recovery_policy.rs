//! Independent regressions for findings found during source-contract review.
use architect::{
    config::{Config, Mode, WorkspaceMode},
    domain::{FileEdit, Run, RunSpec, Stage, Status},
    id,
    ipc::{self, Operation, Request},
    recovery::{self, EditState},
    runtime::{Command, Runtime},
    storage::Store,
};
use std::{path::Path, sync::Arc};

fn spec(root: &Path) -> RunSpec {
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.canonicalize().unwrap().to_str().unwrap().into();
    spec.input_files = vec!["a".into(), "b".into()];
    spec.editable_files = spec.input_files.clone();
    spec.check_command = vec!["true".into()];
    spec
}

#[test]
fn missing_and_binary_inputs_can_be_reconciled_then_explicitly_revised() {
    for binary in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("a"), "original a").unwrap();
        std::fs::write(temp.path().join("b"), "original b").unwrap();
        let mut run = Run::new(id(), spec(temp.path())).unwrap();
        run.stage = Stage::Execution;
        run.status = Status::Running;
        run.attempt = 1;
        run.reconciliation_required = true;
        run.pending_edits = recovery::prepare(
            &run,
            &[FileEdit {
                path: "b".into(),
                content: "new b".into(),
            }],
        )
        .unwrap();
        let mut store = Store::open(&temp.path().join("state.db")).unwrap();
        store.save(&run, "interrupted", None).unwrap();
        if binary {
            std::fs::write(temp.path().join("b"), [0xff, 0x80]).unwrap();
        } else {
            std::fs::remove_file(temp.path().join("b")).unwrap();
        }
        let runtime = Arc::new(Runtime::new(store).unwrap());
        let run = runtime.store.lock().unwrap().get(&run.id).unwrap();
        assert_eq!(run.pending_edits[0].state, EditState::Conflict);
        let response = ipc::handle(
            &runtime,
            Request {
                version: ipc::PROTOCOL_VERSION,
                request_id: id(),
                idempotency_key: None,
                operation: Operation::Snapshot {
                    run_id: run.id.clone(),
                },
            },
        );
        assert!(response.error.is_none(), "{:?}", response.error);
        let snapshot = response.data.unwrap();
        let scope = snapshot["scope_hashes"]["reconcile"].as_str().unwrap();
        let observed = snapshot["recovery"]["observed_input_hash"]
            .as_str()
            .unwrap();
        assert!(
            runtime
                .dispatch(
                    &id(),
                    Command::Reconcile {
                        run_id: run.id.clone(),
                        scope_hash: scope.into(),
                        input_hash: "stale".into()
                    }
                )
                .is_err()
        );
        let reconciled = runtime
            .dispatch(
                &id(),
                Command::Reconcile {
                    run_id: run.id.clone(),
                    scope_hash: scope.into(),
                    input_hash: observed.into(),
                },
            )
            .unwrap();
        assert!(!reconciled.reconciliation_required);
        assert_eq!(reconciled.status, Status::Blocked);
        assert!(reconciled.pending_edits.is_empty());
        assert!(
            runtime
                .dispatch(
                    &id(),
                    Command::ApproveRepair {
                        run_id: run.id.clone(),
                        scope_hash: reconciled.scope_hash("repair")
                    }
                )
                .is_err()
        );
        if binary {
            assert_eq!(std::fs::read(temp.path().join("b")).unwrap(), [0xff, 0x80]);
        } else {
            assert!(!temp.path().join("b").exists());
        }
        let mut revised_spec = reconciled.spec.clone();
        revised_spec.input_files = vec!["a".into()];
        revised_spec.editable_files = vec!["a".into()];
        let revised = runtime
            .dispatch(
                &id(),
                Command::Revise {
                    run_id: run.id.clone(),
                    expected_revision: reconciled.spec_revision,
                    spec: revised_spec,
                },
            )
            .unwrap();
        assert_eq!(revised.status, Status::AwaitingStart);
        assert_eq!(revised.input_snapshot.len(), 1);
    }
}

#[test]
fn pinned_local_policy_and_unimplemented_workspace_modes_fail_before_work() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("a"), "original a").unwrap();
    std::fs::write(temp.path().join("b"), "original b").unwrap();
    let mut config = Config::from_toml(include_str!("../examples/architect.toml")).unwrap();
    config.workspace.mode = WorkspaceMode::InPlace;
    let mut candidate = spec(temp.path());
    candidate.config = Some(config.pin().unwrap());
    candidate.validate().unwrap();
    for variant in 0..4 {
        let mut denied = config.clone();
        match variant {
            0 => denied.policy.local_checks = false,
            1 => denied.policy.local_write = false,
            2 => denied.workspace.mode = WorkspaceMode::Worktree,
            _ => denied.policy.mode = Mode::Autonomous,
        }
        candidate.config = Some(denied.pin().unwrap());
        assert!(
            Run::new(id(), candidate.clone()).is_err(),
            "variant {variant}"
        );
    }
    assert_eq!(
        std::fs::read_to_string(temp.path().join("a")).unwrap(),
        "original a"
    );
    assert_eq!(
        std::fs::read_to_string(temp.path().join("b")).unwrap(),
        "original b"
    );
}

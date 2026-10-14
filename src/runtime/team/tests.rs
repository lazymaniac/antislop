use super::*;
use crate::{domain::RunSpec, runtime::Command, storage::Store};

fn fixture() -> (tempfile::TempDir, Arc<Runtime>, Run) {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("source.txt"), "before").unwrap();
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../../../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().to_string_lossy().into();
    spec.input_files = vec!["source.txt".into()];
    spec.editable_files = spec.input_files.clone();
    spec.check_command = vec!["true".into()];
    let runtime =
        Arc::new(Runtime::new(Store::open(&root.path().join("state.db")).unwrap()).unwrap());
    let mut run = runtime.dispatch(&id(), Command::Create { spec }).unwrap();
    run.status = Status::Running;
    run.attempt = 1;
    run.reconciliation_required = true;
    runtime
        .store
        .lock()
        .unwrap()
        .save(&run, "synthetic_admitted_team_write", None)
        .unwrap();
    (root, runtime, run)
}
fn edit() -> FileEdit {
    FileEdit {
        path: "source.txt".into(),
        content: "after".into(),
    }
}

#[test]
fn mediated_team_write_retains_real_before_after_receipts_and_respects_cancel() {
    let (root, runtime, run) = fixture();
    runtime
        .apply_task_edits(&run, &[edit()], &AtomicBool::new(false))
        .unwrap();
    let current = runtime.store.lock().unwrap().get(&run.id).unwrap();
    assert_eq!(
        std::fs::read_to_string(root.path().join("source.txt")).unwrap(),
        "after"
    );
    assert_eq!(current.input_snapshot["source.txt"], "after");
    let receipt = &current.pending_edits[0];
    assert_eq!(receipt.before_hash, hash("before"));
    assert_eq!(receipt.after_hash, hash("after"));
    assert_eq!(receipt.state, crate::recovery::EditState::Applied);
    assert!(current.reconciliation_required);
    runtime
        .dispatch(
            &id(),
            Command::Cancel {
                run_id: run.id.clone(),
            },
        )
        .unwrap();
    assert!(
        runtime
            .apply_task_edits(
                &current,
                &[FileEdit {
                    path: "source.txt".into(),
                    content: "late".into()
                }],
                &AtomicBool::new(false)
            )
            .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("source.txt")).unwrap(),
        "after"
    );
}

#[test]
fn failed_intent_commit_has_no_filesystem_effect_and_failed_receipt_commit_is_recoverable() {
    for boundary in ["team_edit_intent", "team_edit_applied"] {
        let (root, runtime, run) = fixture();
        let connection = rusqlite::Connection::open(root.path().join("state.db")).unwrap();
        connection.execute_batch(&format!("CREATE TRIGGER refuse_edit_commit BEFORE INSERT ON events WHEN NEW.kind='{boundary}' BEGIN SELECT RAISE(ABORT, 'injected edit commit failure'); END;")).unwrap();
        assert!(
            runtime
                .apply_task_edits(&run, &[edit()], &AtomicBool::new(false))
                .is_err()
        );
        let mut durable = runtime.store.lock().unwrap().get(&run.id).unwrap();
        assert!(durable.reconciliation_required);
        if boundary == "team_edit_intent" {
            assert!(durable.pending_edits.is_empty());
            assert_eq!(
                std::fs::read_to_string(root.path().join("source.txt")).unwrap(),
                "before"
            );
        } else {
            assert_eq!(
                durable.pending_edits[0].state,
                crate::recovery::EditState::Pending
            );
            assert_eq!(durable.input_snapshot["source.txt"], "before");
            assert_eq!(
                std::fs::read_to_string(root.path().join("source.txt")).unwrap(),
                "after"
            );
            crate::recovery::inspect(&mut durable).unwrap();
            assert_eq!(
                durable.pending_edits[0].state,
                crate::recovery::EditState::Applied
            );
            assert_eq!(
                std::fs::read_to_string(root.path().join("source.txt")).unwrap(),
                "after"
            );
        }
    }
}

use architect::{
    domain::{FileEdit, FileOperation, RunSpec},
    recovery,
    runtime::{Command, Runtime},
    storage::Store,
    workspace,
};
use std::{path::Path, sync::Arc};
fn git(root: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn fixture(root: &Path) -> (tempfile::TempDir, Arc<Runtime>, architect::domain::Run) {
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.to_string_lossy().into();
    spec.input_files = vec![".".into()];
    spec.editable_files = spec.input_files.clone();
    spec.check_command = vec!["true".into()];
    // Keep runtime metadata outside the observed project.
    let state = tempfile::tempdir().unwrap();
    let runtime =
        Arc::new(Runtime::new(Store::open(&state.path().join("state.db")).unwrap()).unwrap());
    let run = runtime
        .dispatch(&architect::id(), Command::Create { spec })
        .unwrap();
    (state, runtime, run)
}
#[test]
fn directory_scope_writes_creates_and_deletes_without_expanding_declared_inputs_or_adopting_user_edits()
 {
    for repository in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let r = root.path();
        std::fs::write(r.join("old.txt"), "original").unwrap();
        std::fs::write(r.join("untracked.txt"), "untracked").unwrap();
        if repository {
            git(r, &["init", "-q"]);
            git(r, &["add", "old.txt"]);
        }
        let (_state, _, mut run) = fixture(r);
        let baseline = run.input_snapshot.clone();
        let edits = vec![
            FileEdit {
                path: "old.txt".into(),
                content: "changed".into(),
                operation: FileOperation::Write,
            },
            FileEdit {
                path: "new/sub.txt".into(),
                content: "created".into(),
                operation: FileOperation::Create,
            },
            FileEdit {
                path: "untracked.txt".into(),
                content: String::new(),
                operation: FileOperation::Delete,
            },
        ];
        let receipts = recovery::prepare(&run, &edits).unwrap();
        for (edit, receipt) in edits.iter().zip(&receipts) {
            workspace::apply_edit(r, edit, &receipt.before_hash).unwrap();
            workspace::observation::record_change(
                r,
                &mut run.input_snapshot,
                &mut run.input_observation,
                edit,
                &receipt.before_hash,
            )
            .unwrap();
        }
        assert_eq!(run.input_snapshot.len(), 1);
        assert_eq!(run.spec.input_files, vec!["."]);
        assert_eq!(
            workspace::read_inputs(r, &run.spec.input_files).unwrap(),
            run.input_snapshot
        );
        run.pending_edits = receipts;
        recovery::inspect(&mut run).unwrap();
        assert!(
            run.pending_edits
                .iter()
                .all(|e| e.state == recovery::EditState::Applied)
        );
        let deletion = FileEdit {
            path: "old.txt".into(),
            content: String::new(),
            operation: FileOperation::Delete,
        };
        let receipt = recovery::prepare(&run, std::slice::from_ref(&deletion))
            .unwrap()
            .remove(0);
        workspace::apply_edit(r, &deletion, &receipt.before_hash).unwrap();
        workspace::observation::record_change(
            r,
            &mut run.input_snapshot,
            &mut run.input_observation,
            &deletion,
            &receipt.before_hash,
        )
        .unwrap();
        assert_eq!(
            workspace::read_inputs(r, &run.spec.input_files).unwrap(),
            run.input_snapshot
        );
        let recreated = FileEdit {
            path: "old.txt".into(),
            content: "recreated".into(),
            operation: FileOperation::Create,
        };
        let receipt = recovery::prepare(&run, std::slice::from_ref(&recreated))
            .unwrap()
            .remove(0);
        workspace::apply_edit(r, &recreated, &receipt.before_hash).unwrap();
        workspace::observation::record_change(
            r,
            &mut run.input_snapshot,
            &mut run.input_observation,
            &recreated,
            &receipt.before_hash,
        )
        .unwrap();
        assert_eq!(
            workspace::read_inputs(r, &run.spec.input_files).unwrap(),
            run.input_snapshot
        );
        assert_eq!(
            workspace::inputs::captured_text(r, &baseline, "old.txt")
                .unwrap()
                .as_deref(),
            Some("original")
        );
        std::fs::write(r.join("other.txt"), "new user work").unwrap();
        assert!(
            recovery::prepare(
                &run,
                &[FileEdit {
                    path: "old.txt".into(),
                    content: "overwrite".into(),
                    operation: FileOperation::Write
                }]
            )
            .is_err()
        );
    }
}
#[test]
fn create_delete_admission_and_filesystem_compare_refuse_wrong_presence_payload_scope_and_links() {
    let root = tempfile::tempdir().unwrap();
    let r = root.path();
    std::fs::write(r.join("old"), "antislop-observed-absent-file-v1").unwrap();
    let (_state, _, run) = fixture(r);
    for edit in [
        FileEdit {
            path: "old".into(),
            content: "bad".into(),
            operation: FileOperation::Create,
        },
        FileEdit {
            path: "missing".into(),
            content: String::new(),
            operation: FileOperation::Delete,
        },
        FileEdit {
            path: "old".into(),
            content: "bad".into(),
            operation: FileOperation::Delete,
        },
        FileEdit {
            path: "../escape".into(),
            content: "bad".into(),
            operation: FileOperation::Create,
        },
        FileEdit {
            path: ".git/config".into(),
            content: "bad".into(),
            operation: FileOperation::Create,
        },
    ] {
        assert!(recovery::prepare(&run, &[edit]).is_err());
    }
    let create = FileEdit {
        path: "new".into(),
        content: "planned".into(),
        operation: FileOperation::Create,
    };
    let receipt = recovery::prepare(&run, std::slice::from_ref(&create))
        .unwrap()
        .remove(0);
    std::fs::write(r.join("new"), "new user entry").unwrap();
    assert!(workspace::apply_edit(r, &create, &receipt.before_hash).is_err());
    assert_eq!(std::fs::read(r.join("new")).unwrap(), b"new user entry");
    std::os::unix::fs::symlink(root.path(), r.join("alias")).unwrap();
    assert!(
        workspace::apply_edit(
            r,
            &FileEdit {
                path: "alias/escape".into(),
                content: "bad".into(),
                operation: FileOperation::Create
            },
            &workspace::inputs::absent_hash()
        )
        .is_err()
    );
    let mut direct = run.clone();
    direct.spec.input_files = vec!["old".into()];
    direct.spec.editable_files = direct.spec.input_files.clone();
    direct.input_snapshot = workspace::read_inputs(r, &direct.spec.input_files).unwrap();
    assert!(
        recovery::prepare(
            &direct,
            &[FileEdit {
                path: "old".into(),
                content: String::new(),
                operation: FileOperation::Delete
            }]
        )
        .is_err()
    );
}

#[test]
fn presence_is_part_of_effect_proof_even_if_file_bytes_hash_like_absence() {
    let root = tempfile::tempdir().unwrap();
    let r = root.path();
    let (_state, _, mut run) = fixture(r);
    let edit = FileEdit {
        path: "collision.txt".into(),
        content: "antislop-observed-absent-file-v1".into(),
        operation: FileOperation::Create,
    };
    let receipt = recovery::prepare(&run, std::slice::from_ref(&edit))
        .unwrap()
        .remove(0);
    assert_eq!(receipt.before_hash, receipt.after_hash);
    assert!(recovery::matches_version(r, &run.input_snapshot, &receipt, true).unwrap());
    assert!(!recovery::matches_version(r, &run.input_snapshot, &receipt, false).unwrap());
    workspace::apply_edit(r, &edit, &receipt.before_hash).unwrap();
    workspace::observation::record_change(
        r,
        &mut run.input_snapshot,
        &mut run.input_observation,
        &edit,
        &receipt.before_hash,
    )
    .unwrap();
    assert!(!recovery::matches_version(r, &run.input_snapshot, &receipt, true).unwrap());
    assert!(recovery::matches_version(r, &run.input_snapshot, &receipt, false).unwrap());
}

#[test]
fn ignored_or_unobserved_existing_creations_are_refused_before_effect_admission() {
    let root = tempfile::tempdir().unwrap();
    let r = root.path();
    git(r, &["init", "-q"]);
    std::fs::write(r.join(".gitignore"), "generated/\n").unwrap();
    let (_state, _, run) = fixture(r);
    let edit = FileEdit {
        path: "generated/new.txt".into(),
        content: "bad".into(),
        operation: FileOperation::Create,
    };
    assert!(recovery::prepare(&run, std::slice::from_ref(&edit)).is_err());
    assert!(!r.join("generated").exists());
    std::fs::create_dir(r.join("generated")).unwrap();
    std::fs::write(r.join("generated/new.txt"), "unobserved user file").unwrap();
    assert!(recovery::prepare(&run, &[edit]).is_err());
    assert_eq!(
        std::fs::read(r.join("generated/new.txt")).unwrap(),
        b"unobserved user file"
    );
}

#[test]
fn directory_effect_recovery_marks_binary_or_link_replacements_as_conflicts() {
    for link in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let r = root.path();
        std::fs::write(r.join("old"), "before").unwrap();
        let (_state, _, mut run) = fixture(r);
        run.pending_edits = recovery::prepare(
            &run,
            &[FileEdit {
                path: "old".into(),
                content: "planned".into(),
                operation: FileOperation::Write,
            }],
        )
        .unwrap();
        std::fs::remove_file(r.join("old")).unwrap();
        if link {
            std::os::unix::fs::symlink("/outside", r.join("old")).unwrap();
        } else {
            std::fs::write(r.join("old"), [0xff, 0x80]).unwrap();
        }
        recovery::inspect(&mut run).unwrap();
        assert_eq!(run.pending_edits[0].state, recovery::EditState::Conflict);
    }
}

#[test]
fn ignored_physical_targets_do_not_masquerade_as_unapplied_creations_during_recovery() {
    let root = tempfile::tempdir().unwrap();
    let r = root.path();
    git(r, &["init", "-q"]);
    std::fs::write(r.join(".gitignore"), "").unwrap();
    let (_state, _, mut run) = fixture(r);
    let edit = FileEdit {
        path: "created.txt".into(),
        content: "planned".into(),
        operation: FileOperation::Create,
    };
    run.pending_edits = recovery::prepare(&run, std::slice::from_ref(&edit)).unwrap();
    workspace::apply_edit(r, &edit, &run.pending_edits[0].before_hash).unwrap();
    std::fs::write(r.join(".gitignore"), "created.txt\n").unwrap();
    recovery::inspect(&mut run).unwrap();
    assert_eq!(run.pending_edits[0].state, recovery::EditState::Conflict);
    assert_eq!(std::fs::read(r.join("created.txt")).unwrap(), b"planned");
}

#[test]
fn editable_leaf_can_narrow_a_directory_input_without_pinning_each_file() {
    let root = tempfile::tempdir().unwrap();
    let r = root.path();
    std::fs::write(r.join("allowed.txt"), "before").unwrap();
    std::fs::write(r.join("protected.txt"), "protected").unwrap();
    let (_state, _, initial) = fixture(r);
    let mut spec = initial.spec.clone();
    spec.editable_files = vec!["allowed.txt".into()];
    let run = architect::domain::Run::new(architect::id(), spec.clone()).unwrap();
    assert_eq!(run.spec.input_files, vec!["."]);
    assert_eq!(run.input_snapshot.len(), 1);
    assert!(
        recovery::prepare(
            &run,
            &[FileEdit {
                path: "allowed.txt".into(),
                content: "after".into(),
                operation: FileOperation::Write
            }]
        )
        .is_ok()
    );
    assert!(
        recovery::prepare(
            &run,
            &[FileEdit {
                path: "protected.txt".into(),
                content: "bad".into(),
                operation: FileOperation::Write
            }]
        )
        .is_err()
    );
    spec.editable_files = vec!["../escape".into()];
    assert!(spec.validate().is_err());
    spec.input_files = vec!["allowed.txt".into()];
    spec.editable_files = vec!["allowed.txt/child".into()];
    assert!(spec.validate().is_err());
}

use architect::workspace;
use std::{os::unix::fs::PermissionsExt, path::Path};
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
#[test]
fn whole_repository_remains_one_source_and_changes_invalidate_its_observation() {
    let root = tempfile::tempdir().unwrap();
    git(root.path(), &["init", "-q"]);
    std::fs::write(root.path().join(".gitignore"), "generated/\n.antislop/\n").unwrap();
    std::fs::write(root.path().join("README.md"), "Architecture description").unwrap();
    std::fs::create_dir(root.path().join("src")).unwrap();
    std::fs::write(root.path().join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(root.path().join("image.bin"), [0, 255, 42]).unwrap();
    std::os::unix::fs::symlink("/outside/workspace", root.path().join("link")).unwrap();
    std::fs::create_dir(root.path().join("generated")).unwrap();
    std::fs::write(root.path().join("generated/output"), "ignored").unwrap();
    git(root.path(), &["add", "."]);
    let absolute = root.path().to_string_lossy().into_owned();
    let input = workspace::read_inputs(root.path(), std::slice::from_ref(&absolute)).unwrap();
    assert_eq!(input.len(), 1); // Preserve the user's directory scope.
    let directory: serde_json::Value = serde_json::from_str(&input[&absolute]).unwrap();
    assert_eq!(directory["kind"], "directory");
    assert_eq!(directory["files"]["src/main.rs"]["text"], "fn main() {}\n");
    assert_eq!(directory["files"]["image.bin"]["kind"], "binary");
    assert_eq!(
        directory["files"]["image.bin"]["sha256"],
        architect::hash([0, 255, 42])
    );
    assert_eq!(directory["files"]["link"]["kind"], "symlink");
    assert!(directory["files"].get("generated/output").is_none());
    assert!(
        directory["files"]
            .as_object()
            .unwrap()
            .keys()
            .all(|p| !p.starts_with(".git/"))
    );
    std::fs::write(root.path().join("src/added.rs"), "new source\n").unwrap();
    assert_ne!(
        workspace::read_inputs(root.path(), std::slice::from_ref(&absolute)).unwrap(),
        input
    );
    std::fs::remove_file(root.path().join("README.md")).unwrap();
    let now = workspace::read_inputs(root.path(), std::slice::from_ref(&absolute)).unwrap();
    let directory: serde_json::Value = serde_json::from_str(&now[&absolute]).unwrap();
    assert_eq!(directory["files"]["README.md"]["kind"], "missing");
    assert_eq!(
        workspace::inputs::relative(root.path(), &absolute).unwrap(),
        "."
    );
    assert!(workspace::read_inputs(root.path(), &["..".into()]).is_err());
    assert!(workspace::read_inputs(root.path(), &["/etc".into()]).is_err());
    assert!(workspace::read_inputs(root.path(), &["link".into()]).is_err());
}
#[test]
fn nongit_directory_captures_subdirectories_and_excludes_owned_runtime_state() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("sub")).unwrap();
    std::fs::write(root.path().join("sub/input.txt"), "all requested source").unwrap();
    let state = root.path().join(".antislop");
    std::fs::create_dir(&state).unwrap();
    std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(state.join("daemon.lock"), "").unwrap();
    std::fs::write(state.join("state.db"), [0, 255]).unwrap();
    let observed = workspace::read_inputs(root.path(), &[".".into()]).unwrap();
    let directory: serde_json::Value = serde_json::from_str(&observed["."]).unwrap();
    assert_eq!(
        directory["files"]["sub/input.txt"]["text"],
        "all requested source"
    );
    assert_eq!(
        directory["excluded_runtime_state"],
        serde_json::json!([".antislop"])
    );
    std::fs::write(state.join("state.db"), [0, 1, 2, 3]).unwrap();
    assert_eq!(
        workspace::read_inputs(root.path(), &[".".into()]).unwrap(),
        observed
    );
}

#[test]
fn directory_diff_reports_actual_changes_and_explicit_nontext_metadata() {
    use architect::workspace::DiffFileStatus;
    let root = tempfile::tempdir().unwrap();
    git(root.path(), &["init", "-q"]);
    std::fs::write(root.path().join(".gitignore"), "generated/\n").unwrap();
    std::fs::create_dir(root.path().join("src")).unwrap();
    std::fs::write(root.path().join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(root.path().join("remove.txt"), "original\n").unwrap();
    std::fs::write(root.path().join("image.bin"), [0, 255, 1]).unwrap();
    std::os::unix::fs::symlink("/outside/first", root.path().join("link")).unwrap();
    git(root.path(), &["add", "."]);
    let baseline = workspace::read_inputs(root.path(), &[".".into()]).unwrap();
    assert!(
        workspace::compute_diff(root.path(), &baseline)
            .unwrap()
            .files
            .is_empty()
    );
    std::fs::write(
        root.path().join("src/main.rs"),
        "fn main() { println!(\"ok\"); }\n",
    )
    .unwrap();
    std::fs::write(root.path().join("added.txt"), "new\n").unwrap();
    std::fs::remove_file(root.path().join("remove.txt")).unwrap();
    std::fs::write(root.path().join("image.bin"), [0, 255, 2]).unwrap();
    std::fs::remove_file(root.path().join("link")).unwrap();
    std::os::unix::fs::symlink("/outside/second", root.path().join("link")).unwrap();
    std::fs::create_dir(root.path().join("generated")).unwrap();
    std::fs::write(root.path().join("generated/result.txt"), "ignored").unwrap();
    let diff = workspace::compute_diff(root.path(), &baseline).unwrap();
    assert_eq!(diff.files.len(), 5);
    assert!(diff.files.iter().all(|file| file.path != "."));
    assert_eq!(
        diff.files
            .iter()
            .find(|f| f.path == "added.txt")
            .unwrap()
            .status,
        DiffFileStatus::Added
    );
    assert_eq!(
        diff.files
            .iter()
            .find(|f| f.path == "remove.txt")
            .unwrap()
            .status,
        DiffFileStatus::Deleted
    );
    assert_eq!(
        diff.files
            .iter()
            .find(|f| f.path == "src/main.rs")
            .unwrap()
            .status,
        DiffFileStatus::Modified
    );
    for path in ["image.bin", "link"] {
        let file = diff.files.iter().find(|f| f.path == path).unwrap();
        assert!(file.metadata_change.is_some());
        assert!(file.hunks.is_empty());
        assert_eq!((file.additions, file.deletions), (0, 0));
    }
}
#[test]
fn directory_diff_handles_deleted_scope_and_ignores_internal_state_changes() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("src")).unwrap();
    std::fs::write(root.path().join("src/a.rs"), "source\n").unwrap();
    let baseline = workspace::read_inputs(root.path(), &["src".into()]).unwrap();
    std::fs::remove_dir_all(root.path().join("src")).unwrap();
    let diff = workspace::compute_diff(root.path(), &baseline).unwrap();
    assert_eq!(diff.files.len(), 1);
    assert_eq!(diff.files[0].path, "src/a.rs");
    let state = root.path().join(".antislop");
    std::fs::create_dir(&state).unwrap();
    std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(state.join("daemon.lock"), "").unwrap();
    std::fs::write(state.join("state.db"), [0, 255]).unwrap();
    let baseline = workspace::read_inputs(root.path(), &[".".into()]).unwrap();
    std::fs::write(state.join("state.db"), [0, 255, 1]).unwrap();
    assert!(
        workspace::compute_diff(root.path(), &baseline)
            .unwrap()
            .files
            .is_empty()
    );
}

#[test]
fn directory_diff_tracks_type_changes_and_absolute_scopes_without_following_links() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("src")).unwrap();
    std::fs::write(root.path().join("src/a.rs"), "original\n").unwrap();
    let absolute = root.path().join("src").to_string_lossy().into_owned();
    let baseline = workspace::read_inputs(root.path(), &[absolute]).unwrap();
    std::fs::remove_dir_all(root.path().join("src")).unwrap();
    std::fs::write(root.path().join("src"), "replacement file\n").unwrap();
    let diff = workspace::compute_diff(root.path(), &baseline).unwrap();
    assert_eq!(diff.files.len(), 2);
    assert_eq!(diff.files[0].path, "src");
    assert_eq!(diff.files[1].path, "src/a.rs");
    std::fs::remove_file(root.path().join("src")).unwrap();
    std::fs::write(
        outside.path().join("secret"),
        "must never appear in the diff",
    )
    .unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("src")).unwrap();
    let diff = workspace::compute_diff(root.path(), &baseline).unwrap();
    assert_eq!(diff.files.len(), 2);
    assert!(diff.files[0].metadata_change.is_some());
    assert!(
        !serde_json::to_string(&diff)
            .unwrap()
            .contains("must never appear")
    );
}

#[test]
fn recovery_observes_a_directory_as_one_readable_version_and_detects_drift() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("source.txt"), "initial\n").unwrap();
    let mut spec: architect::domain::RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().to_string_lossy().into_owned();
    spec.input_files = vec![".".into()];
    spec.editable_files.clear();
    let run = architect::domain::Run::new(architect::id(), spec).unwrap();
    let observed = architect::recovery::observe(&run).unwrap();
    assert_eq!(observed.readable.as_ref(), Some(&run.input_snapshot));
    assert_eq!(observed.inputs.len(), 1);
    std::fs::write(root.path().join("source.txt"), "external change\n").unwrap();
    let changed = architect::recovery::observe(&run).unwrap();
    assert!(changed.readable.is_some());
    assert_ne!(changed.hash, observed.hash);
    assert_ne!(changed.readable.as_ref(), Some(&run.input_snapshot));
}

#[test]
fn managed_edit_updates_overlapping_directory_versions_without_absorbing_user_changes() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("src")).unwrap();
    std::fs::write(root.path().join("src/a.rs"), "original\n").unwrap();
    std::fs::write(root.path().join("src/b.rs"), "user source\n").unwrap();
    let scopes = vec![".".into(), "src".into(), "src/a.rs".into()];
    let baseline = workspace::read_inputs(root.path(), &scopes).unwrap();
    let mut current = baseline.clone();
    workspace::replace(
        root.path(),
        "src/a.rs",
        &architect::hash("original\n"),
        "managed\n",
    )
    .unwrap();
    workspace::inputs::record_edit(
        root.path(),
        &mut current,
        "src/a.rs",
        &architect::hash("original\n"),
        "managed\n",
    )
    .unwrap();
    assert_eq!(
        current,
        workspace::read_inputs(root.path(), &scopes).unwrap()
    );
    let diff = workspace::compute_diff(root.path(), &baseline).unwrap();
    assert_eq!(diff.files.len(), 1);
    assert_eq!(diff.files[0].path, "src/a.rs");
    std::fs::write(root.path().join("src/b.rs"), "concurrent user change\n").unwrap();
    assert_ne!(
        current,
        workspace::read_inputs(root.path(), &scopes).unwrap()
    );
    let retained = current.clone();
    assert!(
        workspace::inputs::record_edit(
            root.path(),
            &mut current,
            "src/a.rs",
            &architect::hash("original\n"),
            "stale edit\n"
        )
        .is_err()
    );
    assert_eq!(retained, current);
}

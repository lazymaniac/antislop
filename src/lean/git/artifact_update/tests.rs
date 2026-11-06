use super::*;

#[test]
fn owned_update_keeps_head_index_permissions_and_the_displaced_inode() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    ok(root, &["init", "-q"]).unwrap();
    std::fs::write(root.join("source"), "source").unwrap();
    let head = commit_all(root, CommitPurpose::TestFixture).unwrap();
    std::fs::write(root.join("report.md"), "old").unwrap();
    let previous = snapshot(root, CommitPurpose::TestFixture).unwrap();
    let mut editor = OpenOptions::new()
        .write(true)
        .open(root.join("report.md"))
        .unwrap();
    std::fs::write(root.join("report.md"), "new").unwrap();
    let change = snapshot(root, CommitPurpose::TestFixture).unwrap();
    std::fs::write(root.join("report.md"), "old").unwrap();
    let index = std::fs::read(root.join(".git/index")).unwrap();
    let expected = BTreeMap::from([("report.md".into(), previous.clone())]);
    let backups = root.join(".antislop/backups");
    assert_eq!(
        deliver_owned_artifacts(root, &previous, &change, &expected, &expected, &backups).unwrap(),
        Integration::Applied
    );
    assert_eq!(std::fs::read(root.join("report.md")).unwrap(), b"new");
    editor.write_all(b"USER").unwrap();
    editor.sync_all().unwrap();
    let saved = std::fs::read_dir(&backups)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("previous");
    assert_eq!(std::fs::read(saved).unwrap(), b"USER");
    assert_eq!(head, super::super::head(root).unwrap());
    assert_eq!(index, std::fs::read(root.join(".git/index")).unwrap());
    assert!(
        ok(root, &["ls-files", "--", "report.md"])
            .unwrap()
            .is_empty()
    );
    // An idempotent replay needs the committed new version, not the old lease.
    let replay = BTreeMap::from([("report.md".into(), change.clone())]);
    assert_eq!(
        deliver_owned_artifacts(root, &previous, &change, &expected, &replay, &backups).unwrap(),
        Integration::Applied
    );
}

#[test]
fn changed_tracked_missing_and_symlink_artifacts_are_never_adopted() {
    for kind in ["edited", "tracked", "deleted", "symlink", "parent"] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        ok(root, &["init", "-q"]).unwrap();
        std::fs::write(root.join("source"), "source").unwrap();
        commit_all(root, CommitPurpose::TestFixture).unwrap();
        std::fs::create_dir(root.join("docs")).unwrap();
        std::fs::write(root.join("docs/report.md"), "old").unwrap();
        let previous = snapshot(root, CommitPurpose::TestFixture).unwrap();
        std::fs::write(root.join("docs/report.md"), "new").unwrap();
        let change = snapshot(root, CommitPurpose::TestFixture).unwrap();
        std::fs::write(root.join("docs/report.md"), "old").unwrap();
        match kind {
            "edited" => std::fs::write(root.join("docs/report.md"), "USER").unwrap(),
            "tracked" => {
                ok(root, &["add", "docs/report.md"]).unwrap();
            }
            "deleted" => std::fs::remove_file(root.join("docs/report.md")).unwrap(),
            "symlink" => {
                std::fs::remove_file(root.join("docs/report.md")).unwrap();
                std::os::unix::fs::symlink("../source", root.join("docs/report.md")).unwrap();
            }
            "parent" => {
                std::fs::rename(root.join("docs"), root.join("elsewhere")).unwrap();
                std::os::unix::fs::symlink("elsewhere", root.join("docs")).unwrap();
            }
            _ => unreachable!(),
        }
        let index = std::fs::read(root.join(".git/index")).unwrap();
        let expected = BTreeMap::from([("docs/report.md".into(), previous.clone())]);
        assert!(
            matches!(
                deliver_owned_artifacts(
                    root,
                    &previous,
                    &change,
                    &expected,
                    &expected,
                    &root.join(".antislop/backups")
                )
                .unwrap(),
                Integration::UserEdits(_)
            ),
            "{kind}"
        );
        assert_eq!(index, std::fs::read(root.join(".git/index")).unwrap());
        assert_eq!(std::fs::read(root.join("source")).unwrap(), b"source");
        if kind == "edited" {
            assert_eq!(std::fs::read(root.join("docs/report.md")).unwrap(), b"USER");
        }
    }
}

#[test]
fn a_user_save_between_validation_and_move_is_restored_instead_of_overwritten() {
    for directory in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("report.md"), "old").unwrap();
        let error = publish(
            root,
            "report.md",
            b"new",
            Some(b"old"),
            &root.join(".antislop/backups"),
            |published| {
                if !published {
                    if directory {
                        std::fs::remove_file(root.join("report.md")).unwrap();
                        std::fs::create_dir(root.join("report.md")).unwrap();
                        std::fs::write(root.join("report.md/user"), "USER").unwrap();
                    } else {
                        std::fs::write(root.join("report.md"), "USER").unwrap();
                    }
                }
            },
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("artifact changed during update"), "{error}");
        assert_eq!(
            std::fs::read(if directory {
                root.join("report.md/user")
            } else {
                root.join("report.md")
            })
            .unwrap(),
            b"USER"
        );
    }
}

#[test]
fn a_new_destination_during_publication_wins_and_the_old_version_is_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("report.md"), "old").unwrap();
    let backups = root.join(".antislop/backups");
    let error = publish(
        root,
        "report.md",
        b"new",
        Some(b"old"),
        &backups,
        |published| {
            if published {
                std::fs::write(root.join("report.md"), "USER").unwrap();
            }
        },
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("prior file retained at"), "{error}");
    assert_eq!(std::fs::read(root.join("report.md")).unwrap(), b"USER");
    let saved = std::fs::read_dir(backups)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("previous");
    assert_eq!(std::fs::read(saved).unwrap(), b"old");
}

#[test]
fn index_lease_blocks_git_writers_and_never_removes_an_existing_user_lock() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    ok(root, &["init", "-q"]).unwrap();
    std::fs::write(root.join("source"), "source").unwrap();
    commit_all(root, CommitPurpose::TestFixture).unwrap();
    std::fs::write(root.join("report.md"), "report").unwrap();
    let index = std::fs::read(root.join(".git/index")).unwrap();
    let lease = index::Lease::acquire(root).unwrap();
    assert!(!git(root, &["add", "report.md"]).unwrap().status.success());
    drop(lease);
    assert!(!root.join(".git/index.lock").exists());
    std::fs::write(root.join(".git/index.lock"), "user git operation").unwrap();
    assert!(index::Lease::acquire(root).is_err());
    assert_eq!(
        std::fs::read_to_string(root.join(".git/index.lock")).unwrap(),
        "user git operation"
    );
    assert_eq!(std::fs::read(root.join(".git/index")).unwrap(), index);
}

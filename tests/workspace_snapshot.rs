use architect::{
    hash,
    workspace::snapshot::{self, FileKind},
};
use std::{
    collections::BTreeSet,
    os::unix::{
        ffi::OsStringExt,
        fs::{PermissionsExt, symlink},
    },
    path::Path,
    process::Command,
    sync::atomic::AtomicBool,
};
fn git(root: &Path, args: &[&str]) -> Vec<u8> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "commit.gpgSign=false",
            "-c",
            "user.name=Workspace Fixture",
            "-c",
            "user.email=fixture@localhost",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "fixture Git failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}
fn repository() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    git(root.path(), &["init", "-b", "main"]);
    std::fs::write(root.path().join("plain.txt"), "base\n").unwrap();
    std::fs::write(root.path().join("binary.dat"), b"\0\xffbinary").unwrap();
    std::fs::write(root.path().join(".gitignore"), "ignored-secret\n").unwrap();
    git(root.path(), &["add", "."]);
    git(root.path(), &["commit", "-m", "fixture base"]);
    root
}
fn observe(root: &Path) -> snapshot::Snapshot {
    snapshot::observe(root, &BTreeSet::new(), &AtomicBool::new(false)).unwrap()
}
#[test]
fn source_snapshot_distinguishes_head_staged_working_deleted_untracked_and_explicit_ignored_bytes_without_writes()
 {
    let root = repository();
    let clean = observe(root.path());
    assert!(clean.local_changes.is_empty());
    std::fs::write(root.path().join("plain.txt"), "staged\n").unwrap();
    git(root.path(), &["add", "plain.txt"]);
    std::fs::write(root.path().join("plain.txt"), "working\n").unwrap();
    std::fs::set_permissions(
        root.path().join("plain.txt"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    std::fs::remove_file(root.path().join("binary.dat")).unwrap();
    let name = "żółć \"quoted\"\n\tfile.txt";
    std::fs::write(root.path().join(name), "literal bytes").unwrap();
    std::fs::write(
        root.path().join("ignored-secret"),
        "DO_NOT_EXPORT_SECRET_BODY",
    )
    .unwrap();
    let index_path = root.path().join(".git/index");
    let before = std::fs::read(&index_path).unwrap();
    let index_metadata = std::fs::metadata(&index_path).unwrap();
    let refs = git(root.path(), &["show-ref"]);
    let observed = snapshot::observe(
        root.path(),
        &BTreeSet::from(["ignored-secret".into()]),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        observed.local_changes,
        BTreeSet::from([
            "plain.txt".into(),
            "binary.dat".into(),
            name.into(),
            "ignored-secret".into()
        ])
    );
    assert!(observed.files["binary.dat"].is_none());
    assert_eq!(
        observed.files[name]
            .as_ref()
            .unwrap()
            .content_hash
            .as_ref()
            .unwrap(),
        &hash("literal bytes")
    );
    let repo = observed.repository.as_ref().unwrap();
    let head = &repo.head_entries["plain.txt"];
    let index = &repo.index_entries["plain.txt"][&0];
    let working = observed.files["plain.txt"]
        .as_ref()
        .unwrap()
        .git_entry
        .as_ref()
        .unwrap();
    assert_ne!(head.object_id, index.object_id);
    assert_ne!(index.object_id, working.object_id);
    assert_eq!(working.mode, "100755");
    assert_eq!(std::fs::read(&index_path).unwrap(), before);
    assert_eq!(
        std::fs::metadata(&index_path).unwrap().modified().unwrap(),
        index_metadata.modified().unwrap()
    );
    assert_eq!(git(root.path(), &["show-ref"]), refs);
    assert!(!root.path().join(".git/index.lock").exists());
    assert!(
        !serde_json::to_string(&observed)
            .unwrap()
            .contains("DO_NOT_EXPORT_SECRET_BODY")
    );
    let include = BTreeSet::from([name.into()]);
    assert_eq!(observed.inclusion_scope(&include).unwrap().len(), 64);
    assert_ne!(
        observed.inclusion_scope(&include).unwrap(),
        observed.inclusion_scope(&BTreeSet::new()).unwrap()
    );
    assert!(
        observed
            .inclusion_scope(&BTreeSet::from(["unobserved".into()]))
            .is_err()
    );
    observed.validate_current(&AtomicBool::new(false)).unwrap();
    std::fs::write(root.path().join("plain.txt"), "new external change").unwrap();
    assert!(observed.validate_current(&AtomicBool::new(false)).is_err());
}
#[test]
fn source_snapshot_observes_unborn_detached_linked_worktree_and_nested_project_without_changing_branch()
 {
    let unborn = tempfile::tempdir().unwrap();
    git(unborn.path(), &["init", "-b", "new"]);
    std::fs::write(unborn.path().join("first"), "initial").unwrap();
    git(unborn.path(), &["add", "first"]);
    let s = observe(unborn.path());
    assert!(s.repository.as_ref().unwrap().head.is_none());
    assert_eq!(
        s.repository.as_ref().unwrap().branch.as_deref(),
        Some("refs/heads/new")
    );
    assert!(s.local_changes.contains("first"));
    let root = repository();
    git(root.path(), &["checkout", "--detach"]);
    let detached = observe(root.path());
    assert!(detached.repository.as_ref().unwrap().branch.is_none());
    let destination = tempfile::tempdir().unwrap();
    let linked = destination.path().join("linked");
    git(
        root.path(),
        &[
            "worktree",
            "add",
            "--detach",
            linked.to_str().unwrap(),
            "HEAD",
        ],
    );
    let s = observe(&linked);
    assert_eq!(
        s.repository.as_ref().unwrap().common_directory,
        detached.repository.as_ref().unwrap().common_directory
    );
    assert_ne!(s.source_inode, detached.source_inode);
    std::fs::create_dir(linked.join("module")).unwrap();
    std::fs::write(linked.join("module/input"), "nested").unwrap();
    let nested = observe(&linked.join("module"));
    assert_eq!(
        nested.repository.as_ref().unwrap().project_prefix,
        "module/"
    );
    assert_eq!(nested.files.len(), 1);
    assert!(nested.files.contains_key("input"));
    assert_eq!(
        git(root.path(), &["rev-parse", "HEAD"]),
        format!(
            "{}\n",
            detached.repository.as_ref().unwrap().head.as_ref().unwrap()
        )
        .as_bytes()
    );
}
#[test]
fn source_snapshot_never_runs_checkout_filters_or_fsmonitor_from_repository_configuration() {
    let root = repository();
    git(
        root.path(),
        &["config", "filter.fixture.clean", "touch filter-ran; cat"],
    );
    git(
        root.path(),
        &["config", "filter.fixture.smudge", "touch filter-ran; cat"],
    );
    git(
        root.path(),
        &["config", "core.fsmonitor", "touch monitor-ran"],
    );
    std::fs::write(root.path().join(".gitattributes"), "* filter=fixture\n").unwrap();
    std::fs::write(
        root.path().join(".git/hooks/post-checkout"),
        "#!/bin/sh\ntouch checkout-hook-ran\n",
    )
    .unwrap();
    std::fs::set_permissions(
        root.path().join(".git/hooks/post-checkout"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let s = observe(root.path());
    assert!(s.local_changes.contains(".gitattributes"));
    for path in ["filter-ran", "monitor-ran", "checkout-hook-ran"] {
        assert!(
            !root.path().join(path).exists(),
            "unapproved callback ran: {path}"
        );
    }
}
#[test]
fn nongit_snapshot_hashes_binary_and_link_target_without_following_external_file_or_creating_git() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("private");
    std::fs::write(&target, "EXTERNAL_BODY_MUST_NOT_BE_READ").unwrap();
    std::fs::write(root.path().join("binary"), b"\x00\xff\x80").unwrap();
    symlink(&target, root.path().join("link")).unwrap();
    let s = observe(root.path());
    assert!(s.repository.is_none());
    assert_eq!(
        s.files["binary"]
            .as_ref()
            .unwrap()
            .content_hash
            .as_ref()
            .unwrap(),
        &hash(b"\x00\xff\x80")
    );
    assert_eq!(s.files["link"].as_ref().unwrap().kind, FileKind::Symlink);
    assert!(
        !serde_json::to_string(&s)
            .unwrap()
            .contains("EXTERNAL_BODY_MUST_NOT_BE_READ")
    );
    assert!(!root.path().join(".git").exists());
    std::fs::remove_file(root.path().join("link")).unwrap();
    symlink(outside.path(), root.path().join("link")).unwrap();
    assert!(
        snapshot::observe(
            root.path(),
            &BTreeSet::from(["link/private".into()]),
            &AtomicBool::new(false)
        )
        .is_err()
    );
}
#[test]
fn conflicts_hash_tampering_and_nonutf8_paths_never_produce_an_inclusion_approval() {
    let root = repository();
    git(root.path(), &["checkout", "-b", "other"]);
    std::fs::write(root.path().join("plain.txt"), "other\n").unwrap();
    git(root.path(), &["add", "plain.txt"]);
    git(root.path(), &["commit", "-m", "other"]);
    git(root.path(), &["checkout", "main"]);
    std::fs::write(root.path().join("plain.txt"), "main\n").unwrap();
    git(root.path(), &["add", "plain.txt"]);
    git(root.path(), &["commit", "-m", "main"]);
    let output = Command::new("git")
        .arg("-C")
        .arg(root.path())
        .args(["-c", "core.hooksPath=/dev/null", "merge", "other"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let s = observe(root.path());
    assert_eq!(
        s.repository.as_ref().unwrap().index_entries["plain.txt"]
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([1, 2, 3])
    );
    assert!(s.inclusion_scope(&BTreeSet::new()).is_err());
    let mut forged = s.clone();
    forged.content_hash = "invented snapshot".into();
    assert!(forged.inclusion_scope(&BTreeSet::new()).is_err());
    let bad = std::ffi::OsString::from_vec(b"invalid-\xff".to_vec());
    // macOS refuses creating some byte names. A Git index can still arrive
    // with such a path from another platform; exercise the actual index bytes.
    let oid = String::from_utf8(git(root.path(), &["rev-parse", "HEAD:plain.txt"])).unwrap();
    let inserted = Command::new("git")
        .arg("-C")
        .arg(root.path())
        .args(["update-index", "--add", "--cacheinfo", "100644", oid.trim()])
        .arg(bad)
        .output()
        .unwrap();
    assert!(
        inserted.status.success(),
        "{}",
        String::from_utf8_lossy(&inserted.stderr)
    );
    assert!(snapshot::observe(root.path(), &BTreeSet::new(), &AtomicBool::new(false)).is_err());
}
#[test]
fn snapshot_cli_inspects_without_spec_daemon_or_source_mutation_and_cancel_refuses_before_launch() {
    let root = repository();
    let before = std::fs::read(root.path().join(".git/index")).unwrap();
    assert!(snapshot::observe(root.path(), &BTreeSet::new(), &AtomicBool::new(true)).is_err());
    for bad in ["../outside", "/absolute", ".git/index"] {
        assert!(
            snapshot::observe(
                root.path(),
                &BTreeSet::from([bad.into()]),
                &AtomicBool::new(false)
            )
            .is_err()
        );
    }
    let output = Command::new(env!("CARGO_BIN_EXE_antislop"))
        .arg("workspace-snapshot")
        .arg(root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let s: snapshot::Snapshot = serde_json::from_slice(&output.stdout).unwrap();
    assert!(s.local_changes.is_empty());
    assert_eq!(
        std::fs::read(root.path().join(".git/index")).unwrap(),
        before
    );
    assert!(!root.path().join(".antislop").exists());
}

#[test]
fn snapshot_retains_sha256_git_object_ids_and_blocks_unobserved_submodule_content() {
    let root = tempfile::tempdir().unwrap();
    git(
        root.path(),
        &["init", "--object-format=sha256", "-b", "main"],
    );
    std::fs::write(
        root.path().join("input"),
        "initial binary-compatible source",
    )
    .unwrap();
    git(root.path(), &["add", "input"]);
    git(root.path(), &["commit", "-m", "initial"]);
    let observed = observe(root.path());
    assert_eq!(
        observed
            .repository
            .as_ref()
            .unwrap()
            .head
            .as_ref()
            .unwrap()
            .len(),
        64
    );
    assert!(observed.local_changes.is_empty());
    let head = observed
        .repository
        .as_ref()
        .unwrap()
        .head
        .as_ref()
        .unwrap()
        .clone();
    git(
        root.path(),
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            "160000",
            &head,
            "submodule",
        ],
    );
    std::fs::create_dir(root.path().join("submodule")).unwrap();
    std::fs::write(
        root.path().join("submodule/unobserved"),
        "not recursively accepted",
    )
    .unwrap();
    let limited = observe(root.path());
    assert!(limited.gitlinks.contains("submodule"));
    assert_eq!(
        limited.files["submodule"].as_ref().unwrap().kind,
        FileKind::Gitlink
    );
    assert!(limited.inclusion_scope(&BTreeSet::new()).is_err());
}

#[test]
fn untracked_nested_repository_is_reported_without_implicitly_approving_its_contents() {
    let root = repository();
    let nested = root.path().join("nested-repository");
    std::fs::create_dir(&nested).unwrap();
    git(&nested, &["init", "-b", "main"]);
    std::fs::write(nested.join("unobserved"), "nested content").unwrap();
    let s = observe(root.path());
    assert!(!s.unobserved_directories.is_empty());
    assert!(s.inclusion_scope(&s.local_changes).is_err());
}

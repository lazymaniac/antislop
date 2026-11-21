use super::*;
use std::io::{BufReader, Cursor};

fn fixture() -> (tempfile::TempDir, PathBuf, Journal) {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("frontend");
    let candidate = root.path().join("new");
    for (path, version) in [(&target, "old"), (&candidate, "new")] {
        let metadata = serde_json::json!({"name":"antislop", "build":version, "protocol":crate::lean::daemon::version_info()["protocol"], "capabilities":["runtime_preflight", "runtime_activation_async_v1"]});
        fs::write(path, format!("#!/bin/sh\nprintf '%s\\n' '{metadata}'\n")).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let directory = root.path().join("journal");
    prepare(&directory, &target, &candidate).unwrap();
    let journal = load(&directory).unwrap();
    (root, directory, journal)
}

fn mutate(directory: &Path, journal: &mut Journal, action: &str) -> Result<()> {
    // Controlled fixture operator: no other publisher exists; new writes are
    // excluded until this synchronous operation durably settles its journal.
    let (input, mut writer) = std::os::unix::net::UnixStream::pair().unwrap();
    let (mut prompt, output) = std::os::unix::net::UnixStream::pair().unwrap();
    let responder = std::thread::spawn(move || {
        let mut reader = BufReader::new(&mut prompt);
        let mut line = String::new();
        loop {
            line.clear();
            assert!(reader.read_line(&mut line).unwrap() > 0);
            if let Some(challenge) = line.split("type: CONFIRM ").nth(1) {
                writeln!(writer, "CONFIRM {}", challenge.trim()).unwrap();
                break;
            }
        }
    });
    confirm_window(
        journal,
        action,
        "isolated fixture: no package manager, IDE, installers or other terminals; single controlled operator prevents writes",
        &mut BufReader::new(input),
        &mut &output,
    )?;
    responder.join().unwrap();
    save(directory, journal)?;
    operate(directory, journal, action)
}

#[test]
fn missing_and_stale_windows_preserve_every_version() {
    let (_root, directory, mut journal) = fixture();
    for action in ["publish", "resume", "cancel", "restore"] {
        for response in ["", "CONFIRM old-window\n"] {
            let error = confirm_window(
                &mut journal,
                action,
                "updaters stopped",
                &mut Cursor::new(response),
                &mut Vec::new(),
            )
            .unwrap_err();
            assert!(error.to_string().contains("missing or stale"));
            assert_eq!(identity(&journal.target).unwrap(), journal.original);
            assert_eq!(identity(&journal.stage).unwrap(), journal.candidate);
            assert!(directory.join("original").exists());
        }
    }
    mutate(&directory, &mut journal, "publish").unwrap();
    let confirmation = journal.confirmation.as_ref().unwrap();
    let stale = format!("CONFIRM {}\n", confirmation.challenge);
    assert!(
        confirm_window(
            &mut journal,
            "restore",
            "new window",
            &mut Cursor::new(stale),
            &mut Vec::new()
        )
        .is_err()
    );
    assert_eq!(identity(&journal.target).unwrap(), journal.candidate);
}

#[test]
fn publish_reopen_cancel_and_restore_preserve_running_inode() {
    let (_root, directory, mut journal) = fixture();
    let mut running_inode = File::open(&journal.target).unwrap();
    mutate(&directory, &mut journal, "publish").unwrap();
    assert_eq!(load(&directory).unwrap().phase, Phase::Published);
    assert_eq!(identity(&journal.target).unwrap(), journal.candidate);
    use std::io::Read;
    let mut running = String::new();
    running_inode.read_to_string(&mut running).unwrap();
    assert!(running.contains("old"));
    let mut journal = load(&directory).unwrap();
    mutate(&directory, &mut journal, "cancel").unwrap();
    assert_eq!(journal.phase, Phase::Cancelled);
    assert_eq!(identity(&journal.target).unwrap(), journal.candidate);
    assert!(mutate(&directory, &mut journal, "resume").is_err());
    mutate(&directory, &mut journal, "restore").unwrap();
    assert!(
        identity(&journal.target)
            .unwrap()
            .same_contents_and_metadata(&journal.original)
    );
    assert_ne!(identity(&journal.target).unwrap(), journal.original);
    assert_eq!(load(&directory).unwrap().phase, Phase::Restored);
}

#[test]
fn user_changes_before_publish_and_after_publish_or_restore_are_never_overwritten() {
    for phase in [Phase::Prepared, Phase::Published, Phase::Restored] {
        let (root, directory, mut journal) = fixture();
        if phase != Phase::Prepared {
            mutate(&directory, &mut journal, "publish").unwrap();
        }
        if phase == Phase::Restored {
            mutate(&directory, &mut journal, "restore").unwrap();
        }
        let user = root.path().join("user-version");
        fs::write(&user, "user changes").unwrap();
        fs::rename(user, &journal.target).unwrap();
        for action in ["publish", "resume", "cancel", "restore"] {
            let error = mutate(&directory, &mut journal, action).unwrap_err();
            assert!(error.to_string().contains("target identity changed"));
            assert_eq!(fs::read(&journal.target).unwrap(), b"user changes");
            assert!(directory.join("original").exists());
            assert!(directory.join("candidate").exists());
        }
    }
}

#[test]
fn interrupted_rename_sync_and_journal_settlement_recover_from_observed_identity() {
    // Durable intent, interruption before rename or after rename but before
    // directory sync/settlement: both require a new window, then resume.
    for renamed in [false, true] {
        let (_root, directory, mut journal) = fixture();
        journal.phase = Phase::Publishing;
        save(&directory, &journal).unwrap();
        if renamed {
            fs::rename(&journal.stage, &journal.target).unwrap();
        }
        let mut reopened = load(&directory).unwrap();
        mutate(&directory, &mut reopened, "resume").unwrap();
        assert_eq!(load(&directory).unwrap().phase, Phase::Published);
        // Restore crash after its rename, before sync/journal completion.
        copy_synced(
            &directory.join("original"),
            &reopened.restore_stage,
            &reopened.original,
        )
        .unwrap();
        reopened.replacement = Some(identity(&reopened.restore_stage).unwrap());
        reopened.phase = Phase::Restoring;
        save(&directory, &reopened).unwrap();
        fs::rename(&reopened.restore_stage, &reopened.target).unwrap();
        let mut restored = load(&directory).unwrap();
        mutate(&directory, &mut restored, "resume").unwrap();
        assert_eq!(restored.phase, Phase::Restored);
    }
}

#[test]
fn copy_validation_and_publication_failures_leave_target_and_backups() {
    let (_root, directory, mut journal) = fixture();
    let original = identity(&journal.target).unwrap();
    // Damaged candidate fails validation before publication.
    fs::write(
        &journal.stage,
        "#!/bin/sh\necho validation-failed >&2\nexit 19\n",
    )
    .unwrap();
    journal.candidate = identity(&journal.stage).unwrap();
    assert!(
        mutate(&directory, &mut journal, "publish")
            .unwrap_err()
            .to_string()
            .contains("validation-failed")
    );
    assert_eq!(identity(&journal.target).unwrap(), original);
    // create_new refuses to overwrite even a partial previous copy.
    let partial = directory.join("partial");
    fs::write(&partial, "partial").unwrap();
    assert!(copy_synced(&journal.target, &partial, &original).is_err());
    assert_eq!(fs::read(partial).unwrap(), b"partial");
    assert!(directory.join("original").exists());
    assert!(directory.join("candidate").exists());
}

thread_local! {
    static FAILURE: std::cell::RefCell<Option<(&'static str, PathBuf)>> = const { std::cell::RefCell::new(None) };
}

pub(super) fn fail(operation: &str, path: &Path) -> Result<()> {
    FAILURE.with_borrow_mut(|failure| {
        if failure
            .as_ref()
            .is_some_and(|(op, target)| *op == operation && target == path)
        {
            *failure = None;
            return Err(std::io::Error::other(format!(
                "injected {operation} failure at {}",
                path.display()
            ))
            .into());
        }
        Ok(())
    })
}

#[test]
fn raw_copy_rename_sync_and_journal_failures_can_be_reopened_without_guessing() {
    for operation in ["copy", "validation", "publication", "sync", "settlement"] {
        let (_root, directory, mut journal) = fixture();
        if operation == "copy" {
            let destination = directory.join("copy-failure");
            FAILURE.set(Some((operation, destination.clone())));
            assert!(
                copy_synced(&journal.target, &destination, &journal.original)
                    .unwrap_err()
                    .to_string()
                    .contains("injected copy failure")
            );
            assert_eq!(identity(&journal.target).unwrap(), journal.original);
            continue;
        }
        let path = match operation {
            "validation" => journal.stage.clone(),
            "publication" => journal.target.clone(),
            "sync" => journal.target.parent().unwrap().to_owned(),
            "settlement" => directory.clone(),
            _ => unreachable!(),
        };
        FAILURE.set(Some((operation, path)));
        let error = mutate(&directory, &mut journal, "publish").unwrap_err();
        assert!(
            error
                .to_string()
                .contains(&format!("injected {operation} failure")),
            "{error}"
        );
        let mut reopened = load(&directory).unwrap();
        let observed = identity(&journal.target).unwrap();
        assert_eq!(
            observed,
            if matches!(operation, "sync" | "settlement") {
                journal.candidate.clone()
            } else {
                journal.original.clone()
            }
        );
        mutate(&directory, &mut reopened, "resume").unwrap();
        assert_eq!(load(&directory).unwrap().phase, Phase::Published);
        assert!(directory.join("original").exists());
        assert!(directory.join("candidate").exists());
    }
    // A restore stage must be durable before its intent is committed. A sync
    // failure here leaves Published on disk; a fresh process can verify/reuse
    // the completed copy without assuming the previous confirmation survived.
    let (_root, directory, mut journal) = fixture();
    mutate(&directory, &mut journal, "publish").unwrap();
    FAILURE.set(Some(("sync", journal.target.parent().unwrap().to_owned())));
    assert!(
        mutate(&directory, &mut journal, "restore")
            .unwrap_err()
            .to_string()
            .contains("injected sync failure")
    );
    assert_eq!(identity(&journal.target).unwrap(), journal.candidate);
    let mut reopened = load(&directory).unwrap();
    assert_eq!(reopened.phase, Phase::Published);
    mutate(&directory, &mut reopened, "restore").unwrap();
    assert_eq!(reopened.phase, Phase::Restored);
}

#[test]
fn restore_rechecks_file_durability_after_complete_unsynced_copy_and_reopen() {
    for failure in ["copy_before_sync", "file_sync"] {
        let (_root, directory, mut journal) = fixture();
        mutate(&directory, &mut journal, "publish").unwrap();
        FAILURE.set(Some((failure, journal.restore_stage.clone())));
        let error = mutate(&directory, &mut journal, "restore").unwrap_err();
        assert!(
            error
                .to_string()
                .contains(&format!("injected {failure} failure"))
        );
        let stage = identity(&journal.restore_stage).unwrap();
        assert!(stage.same_contents_and_metadata(&journal.original));
        let mut reopened = load(&directory).unwrap();
        assert_eq!(reopened.phase, Phase::Published);
        assert!(reopened.replacement.is_none());
        assert_eq!(identity(&reopened.target).unwrap(), reopened.candidate);

        // Reuse must reach file sync even though hash and metadata match.
        FAILURE.set(Some(("file_sync", reopened.restore_stage.clone())));
        let error = mutate(&directory, &mut reopened, "restore").unwrap_err();
        assert!(error.to_string().contains("injected file_sync failure"));
        let mut reopened = load(&directory).unwrap();
        assert_eq!(reopened.phase, Phase::Published);
        assert!(reopened.replacement.is_none());
        assert_eq!(identity(&reopened.target).unwrap(), reopened.candidate);
        assert_eq!(identity(&reopened.restore_stage).unwrap(), stage);
        assert!(directory.join("original").exists());
        assert!(directory.join("candidate").exists());

        mutate(&directory, &mut reopened, "restore").unwrap();
        assert_eq!(load(&directory).unwrap().phase, Phase::Restored);
        assert_eq!(identity(&reopened.target).unwrap(), stage);
    }
}

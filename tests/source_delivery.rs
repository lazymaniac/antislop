use architect::{
    hash, id,
    workspace::delivery::{self, Change, EffectState, Entry, Identity, Kind, Stamp},
};
use std::{
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
};
fn entry(root: &Path, data: &[u8], kind: Kind, permissions: u32) -> Entry {
    let m = root.metadata().unwrap();
    Entry {
        stamp: Stamp {
            kind,
            sha256: hash(data),
            bytes: data.len() as u64,
            permissions,
            owner_uid: m.uid(),
            owner_gid: m.gid(),
        },
        data: data.to_vec(),
    }
}
fn modification(root: &Path, path: &str, data: &[u8]) -> Change {
    let before = delivery::stamp(root, path).unwrap();
    let permissions = before.as_ref().map_or(0o644, |s| s.permissions);
    Change {
        path: path.into(),
        before,
        after: Some(entry(root, data, Kind::File, permissions)),
    }
}
#[test]
fn delivery_transfers_text_binary_links_additions_and_deletions_and_retains_prior_versions() {
    let root = tempfile::tempdir().unwrap();
    let r = root.path();
    std::fs::write(r.join("old.txt"), "old\n").unwrap();
    std::fs::write(r.join("delete.txt"), "retained deletion\n").unwrap();
    std::fs::write(r.join("binary.bin"), [0, 255, 1]).unwrap();
    std::os::unix::fs::symlink("old.txt", r.join("link")).unwrap();
    let changes = vec![
        modification(r, "old.txt", b"new\n"),
        modification(r, "binary.bin", &[0, 255, 2]),
        modification(r, "new/sub.txt", b"added\n"),
        Change {
            path: "delete.txt".into(),
            before: delivery::stamp(r, "delete.txt").unwrap(),
            after: None,
        },
        Change {
            path: "link".into(),
            before: delivery::stamp(r, "link").unwrap(),
            after: Some(entry(
                r,
                b"new/sub.txt",
                Kind::Link,
                delivery::stamp(r, "link").unwrap().unwrap().permissions,
            )),
        },
    ];
    let operation = id();
    let mut events = vec![];
    let effects = delivery::apply(
        r,
        &Identity::read(r).unwrap(),
        &operation,
        &changes,
        &mut |e| {
            events.push(e.clone());
            Ok(())
        },
    )
    .unwrap();
    assert!(effects.iter().all(|e| e.state == EffectState::Applied));
    assert_eq!(std::fs::read(r.join("old.txt")).unwrap(), b"new\n");
    assert_eq!(std::fs::read(r.join("binary.bin")).unwrap(), [0, 255, 2]);
    assert_eq!(std::fs::read(r.join("new/sub.txt")).unwrap(), b"added\n");
    assert!(!r.join("delete.txt").exists());
    assert_eq!(
        std::fs::read_link(r.join("link")).unwrap(),
        Path::new("new/sub.txt")
    );
    assert_eq!(std::fs::read(r.join(&effects[0].backup)).unwrap(), b"old\n");
    assert_eq!(
        std::fs::read(r.join(&effects[3].backup)).unwrap(),
        b"retained deletion\n"
    );
    assert_eq!(
        std::fs::read_link(r.join(&effects[4].backup)).unwrap(),
        Path::new("old.txt")
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| e.state == EffectState::Intent)
            .count(),
        changes.len()
    );
    assert!(
        delivery::apply(
            r,
            &Identity::read(r).unwrap(),
            &operation,
            &changes,
            &mut |_| Ok(())
        )
        .is_err()
    );
}
#[test]
fn source_drift_and_parent_symlinks_are_rejected_before_any_file_effect() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let r = root.path();
    std::fs::write(r.join("a"), "before").unwrap();
    let change = modification(r, "a", b"after");
    std::fs::write(r.join("a"), "user edit").unwrap();
    let mut checkpoints = 0;
    assert!(
        delivery::apply(
            r,
            &Identity::read(r).unwrap(),
            &id(),
            &[change],
            &mut |_| {
                checkpoints += 1;
                Ok(())
            }
        )
        .is_err()
    );
    assert_eq!(checkpoints, 0);
    assert_eq!(std::fs::read(r.join("a")).unwrap(), b"user edit");
    std::os::unix::fs::symlink(outside.path(), r.join("alias")).unwrap();
    let change = Change {
        path: "alias/escape".into(),
        before: None,
        after: Some(entry(r, b"must not write", Kind::File, 0o644)),
    };
    assert!(
        delivery::apply(
            r,
            &Identity::read(r).unwrap(),
            &id(),
            &[change],
            &mut |_| Ok(())
        )
        .is_err()
    );
    assert!(!outside.path().join("escape").exists());
}
#[test]
fn concurrent_destination_creation_is_never_overwritten_and_both_source_versions_survive() {
    let root = tempfile::tempdir().unwrap();
    let r = root.path();
    std::fs::write(r.join("a"), "before").unwrap();
    let change = modification(r, "a", b"proposed");
    let mut backup = String::new();
    let result = delivery::apply(r, &Identity::read(r).unwrap(), &id(), &[change], &mut |e| {
        if e.state == EffectState::BackedUp {
            backup = e.backup.clone();
            std::fs::write(r.join("a"), "new user entry")?;
        }
        Ok(())
    });
    assert!(result.is_err());
    assert_eq!(std::fs::read(r.join("a")).unwrap(), b"new user entry");
    assert_eq!(std::fs::read(r.join(backup)).unwrap(), b"before");
}
#[test]
fn failed_intent_commit_writes_nothing_and_lost_post_capture_receipt_is_inspectable() {
    let root = tempfile::tempdir().unwrap();
    let r = root.path();
    std::fs::write(r.join("a"), "before").unwrap();
    let change = modification(r, "a", b"proposed");
    let operation = id();
    assert!(
        delivery::apply(
            r,
            &Identity::read(r).unwrap(),
            &operation,
            std::slice::from_ref(&change),
            &mut |_| Err(architect::Error::Invalid("failed intent".into()))
        )
        .is_err()
    );
    assert_eq!(std::fs::read(r.join("a")).unwrap(), b"before");
    assert!(!r.join(format!(".antislop-delivery-{operation}")).exists());
    let mut backup = String::new();
    assert!(
        delivery::apply(
            r,
            &Identity::read(r).unwrap(),
            &operation,
            &[change],
            &mut |e| {
                if e.state == EffectState::BackedUp {
                    backup = e.backup.clone();
                    return Err(architect::Error::Invalid("lost outcome commit".into()));
                }
                Ok(())
            }
        )
        .is_err()
    );
    assert!(!r.join("a").exists());
    assert_eq!(std::fs::read(r.join(backup)).unwrap(), b"before");
}
#[test]
fn directory_identity_payload_tampering_and_git_administration_paths_are_refused() {
    let root = tempfile::tempdir().unwrap();
    let r = root.path();
    let other = tempfile::tempdir().unwrap();
    std::fs::write(r.join("a"), "before").unwrap();
    let change = modification(r, "a", b"after");
    assert!(
        delivery::apply(
            r,
            &Identity::read(other.path()).unwrap(),
            &id(),
            std::slice::from_ref(&change),
            &mut |_| Ok(())
        )
        .is_err()
    );
    let mut forged = change;
    forged.after.as_mut().unwrap().data.push(0);
    assert!(
        delivery::apply(
            r,
            &Identity::read(r).unwrap(),
            &id(),
            &[forged],
            &mut |_| Ok(())
        )
        .is_err()
    );
    for path in ["../escape", ".git/config", "dir/../../escape"] {
        let c = Change {
            path: path.into(),
            before: None,
            after: Some(entry(r, b"bad", Kind::File, 0o644)),
        };
        assert!(
            delivery::apply(r, &Identity::read(r).unwrap(), &id(), &[c], &mut |_| Ok(())).is_err()
        );
    }
    std::fs::set_permissions(r.join("a"), std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(std::fs::read(r.join("a")).unwrap(), b"before");
}

#[test]
fn missing_source_is_restored_from_exact_retained_backup_without_overwriting_a_new_user_entry() {
    let root = tempfile::tempdir().unwrap();
    let r = root.path();
    std::fs::write(r.join("a"), "before").unwrap();
    let before = delivery::stamp(r, "a").unwrap().unwrap();
    let change = modification(r, "a", b"proposed");
    let operation = id();
    let mut backup = String::new();
    assert!(
        delivery::apply(
            r,
            &Identity::read(r).unwrap(),
            &operation,
            &[change],
            &mut |e| {
                if e.state == EffectState::BackedUp {
                    backup = e.backup.clone();
                    return Err(architect::Error::Invalid("crash before outcome".into()));
                }
                Ok(())
            }
        )
        .is_err()
    );
    let identity = Identity::read(r).unwrap();
    std::fs::write(r.join("a"), "new user file").unwrap();
    assert!(delivery::restore_missing(r, &identity, "a", &backup, &before).is_err());
    assert_eq!(std::fs::read(r.join("a")).unwrap(), b"new user file");
    std::fs::remove_file(r.join("a")).unwrap();
    delivery::restore_missing(r, &identity, "a", &backup, &before).unwrap();
    assert_eq!(std::fs::read(r.join("a")).unwrap(), b"before");
    assert_eq!(std::fs::read(r.join(backup)).unwrap(), b"before");
}

#[test]
fn completed_status_alone_never_authorizes_delivery_without_independent_result_proof() {
    use architect::{
        domain::*,
        runtime::{Command, Runtime},
        storage::Store,
    };
    use std::sync::Arc;
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("a"), "user data").unwrap();
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = source.to_string_lossy().into();
    spec.input_files = vec!["a".into()];
    let runtime =
        Arc::new(Runtime::new(Store::open(&root.path().join("state.db")).unwrap()).unwrap());
    let mut run = runtime.dispatch(&id(), Command::Create { spec }).unwrap();
    run.status = Status::Completed;
    runtime
        .store
        .lock()
        .unwrap()
        .save(&run, "fixture_forged_completed_status", None)
        .unwrap();
    assert!(
        runtime
            .dispatch(
                &id(),
                Command::InspectDelivery {
                    run_id: run.id.clone()
                }
            )
            .is_err()
    );
    assert!(
        runtime
            .dispatch(
                &id(),
                Command::DeliverSource {
                    run_id: run.id.clone(),
                    scope_hash: "forged".into()
                }
            )
            .is_err()
    );
    assert_eq!(std::fs::read(source.join("a")).unwrap(), b"user data");
}

#[test]
fn runtime_recovery_restores_only_missing_targets_and_requires_fresh_explicit_observation() {
    use architect::{
        domain::*,
        runtime::{Command, Runtime, delivery as transfer},
        storage::Store,
    };
    use std::sync::{Arc, atomic::AtomicBool};
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("a"), "original").unwrap();
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = source.to_string_lossy().into();
    spec.input_files = vec!["a".into()];
    let runtime =
        Arc::new(Runtime::new(Store::open(&root.path().join("state.db")).unwrap()).unwrap());
    let mut run = runtime.dispatch(&id(), Command::Create { spec }).unwrap();
    let operation = id();
    let identity = Identity::read(&source).unwrap();
    let candidate = architect::workspace::snapshot::observe(
        &source,
        &Default::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let change = modification(&source, "a", b"proposed");
    let mut effects = std::collections::BTreeMap::new();
    assert!(
        delivery::apply(
            &source,
            &identity,
            &operation,
            std::slice::from_ref(&change),
            &mut |effect| {
                effects.insert(effect.path.clone(), effect.clone());
                if effect.state == EffectState::BackedUp {
                    return Err(architect::Error::Invalid(
                        "injected lost checkpoint outcome".into(),
                    ));
                }
                Ok(())
            }
        )
        .is_err()
    );
    run.status = Status::Completed;
    run.reconciliation_required = true;
    run.delivery = Some(transfer::Record {
        state: transfer::State::Unknown,
        plan: Some(transfer::Plan {
            id: operation,
            run_id: run.id.clone(),
            spec_revision: run.spec_revision,
            task_binding: "retained historical proof; recovery does not execute tasks".into(),
            source_root: source.to_string_lossy().into(),
            source_identity: identity,
            candidate,
            changes: vec![change],
            conflicts: vec![],
            scope_hash: hash("historical transfer scope"),
        }),
        effects,
        diagnostic: Some("fixture interrupted after capture".into()),
    });
    runtime
        .store
        .lock()
        .unwrap()
        .save(&run, "fixture_interrupted_source_transfer", None)
        .unwrap();
    let missing = transfer::recovery(&run).unwrap();
    assert_eq!(missing.restorable, vec!["a"]);
    assert!(
        runtime
            .dispatch(
                &id(),
                Command::ReconcileSourceDelivery {
                    run_id: run.id.clone(),
                    scope_hash: missing.scope_hash.clone(),
                    observation_hash: missing.observation_hash.clone()
                }
            )
            .is_err()
    );
    std::fs::write(source.join("a"), "new user entry").unwrap();
    assert!(
        runtime
            .dispatch(
                &id(),
                Command::RestoreSourceDeliveryBackup {
                    run_id: run.id.clone(),
                    path: "a".into(),
                    scope_hash: missing.scope_hash.clone()
                }
            )
            .is_err()
    );
    assert_eq!(std::fs::read(source.join("a")).unwrap(), b"new user entry");
    std::fs::remove_file(source.join("a")).unwrap();
    let restored = runtime
        .dispatch(
            &id(),
            Command::RestoreSourceDeliveryBackup {
                run_id: run.id.clone(),
                path: "a".into(),
                scope_hash: missing.scope_hash.clone(),
            },
        )
        .unwrap();
    assert_eq!(std::fs::read(source.join("a")).unwrap(), b"original");
    assert!(restored.reconciliation_required);
    assert!(
        restored
            .approvals
            .iter()
            .any(|a| a.action == "restore-source-delivery-backup-intent")
    );
    assert!(
        runtime
            .dispatch(
                &id(),
                Command::ReconcileSourceDelivery {
                    run_id: run.id.clone(),
                    scope_hash: missing.scope_hash,
                    observation_hash: missing.observation_hash
                }
            )
            .is_err()
    );
    let observed = transfer::recovery(&restored).unwrap();
    assert!(!observed.desired_state_observed);
    let settled = runtime
        .dispatch(
            &id(),
            Command::ReconcileSourceDelivery {
                run_id: run.id.clone(),
                scope_hash: observed.scope_hash,
                observation_hash: observed.observation_hash,
            },
        )
        .unwrap();
    assert!(!settled.reconciliation_required);
    assert_eq!(settled.delivery.unwrap().state, transfer::State::Conflict);
    assert_eq!(std::fs::read(source.join("a")).unwrap(), b"original");
}

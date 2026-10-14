use architect::{domain::*, id, storage::Store};

fn fixture() -> (tempfile::TempDir, Store, Run) {
    let root = tempfile::tempdir().unwrap();
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().to_string_lossy().into();
    let run = Run::new(id(), spec).unwrap();
    let mut store = Store::open(&root.path().join("state.db")).unwrap();
    store.save(&run, "created", None).unwrap();
    (root, store, run)
}

#[test]
fn decoded_snapshot_observes_external_edits_even_with_unchanged_digest_and_rejects_corruption() {
    let (root, store, run) = fixture();
    let mut copy = store.get(&run.id).unwrap();
    copy.diagnostic = Some("caller-local change".into());
    assert!(store.get(&run.id).unwrap().diagnostic.is_none());
    let database = rusqlite::Connection::open(root.path().join("state.db")).unwrap();
    copy.diagnostic = Some("external committed edit".into());
    database
        .execute(
            "UPDATE runs SET state=?1 WHERE id=?2",
            rusqlite::params![serde_json::to_string(&copy).unwrap(), run.id],
        )
        .unwrap();
    assert_eq!(store.get(&run.id).unwrap().diagnostic, copy.diagnostic);
    database
        .execute(
            "UPDATE runs SET state='invalid JSON' WHERE id=?1",
            [&run.id],
        )
        .unwrap();
    assert!(store.get(&run.id).is_err());
    database
        .execute(
            "UPDATE runs SET state=?1 WHERE id=?2",
            rusqlite::params![serde_json::to_string(&run).unwrap(), run.id],
        )
        .unwrap();
    assert!(store.get(&run.id).unwrap().diagnostic.is_none());
    database
        .execute("DELETE FROM events WHERE run_id=?1", [&run.id])
        .unwrap();
    database
        .execute("DELETE FROM runs WHERE id=?1", [&run.id])
        .unwrap();
    assert!(
        store.get(&run.id).is_err(),
        "deleted state cannot be returned from the decoded snapshot"
    );
}

#[test]
fn failed_snapshot_commit_retains_authoritative_old_state_and_no_request_response() {
    let (root, mut store, run) = fixture();
    let database = rusqlite::Connection::open(root.path().join("state.db")).unwrap();
    database.execute_batch("CREATE TRIGGER refuse_snapshot BEFORE INSERT ON events WHEN NEW.kind='fail' BEGIN SELECT RAISE(ABORT, 'injected commit failure'); END;").unwrap();
    let mut proposed = run.clone();
    proposed.diagnostic = Some("uncommitted change".into());
    assert!(
        store
            .save(&proposed, "fail", Some(("failed-key", b"request")))
            .is_err()
    );
    assert!(store.get(&run.id).unwrap().diagnostic.is_none());
    assert!(store.cached("failed-key", b"request").unwrap().is_none());
    store.save(&proposed, "accepted", None).unwrap();
    assert_eq!(store.get(&run.id).unwrap().diagnostic, proposed.diagnostic);
    assert_eq!(
        Store::open(&root.path().join("state.db"))
            .unwrap()
            .get(&run.id)
            .unwrap()
            .diagnostic,
        proposed.diagnostic
    );
}

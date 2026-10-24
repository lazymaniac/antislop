use super::*;

#[test]
fn incompatible_databases_are_rejected_without_writes() {
    for version in [0, 1, 5, 10, 12] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("core.db");
        let db = Connection::open(&path).unwrap();
        db.execute_batch(
            "CREATE TABLE sentinel(value TEXT); INSERT INTO sentinel VALUES('preserved');",
        )
        .unwrap();
        db.pragma_update(None, "user_version", version).unwrap();
        drop(db);
        let bytes = std::fs::read(&path).unwrap();
        let error = Store::open(&path).err().unwrap().to_string();
        assert!(
            error.contains("incompatible schema")
                && error.contains("archive")
                && error.contains(".antislop/"),
            "{error}"
        );
        assert_eq!(bytes, std::fs::read(&path).unwrap());
        assert!(!path.with_extension("db-wal").exists());
    }
}

#[test]
fn current_schema_initializes_and_reopens_without_removed_columns() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.db");
    let store = Store::open(&path).unwrap();
    let conversation = store.create_conversation("project").unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.conversations().unwrap()[0].id, conversation.id);
    let db = store.db();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        SCHEMA
    );
    for (table, column) in [
        ("ledger_entries", "kind"),
        ("ledger_versions", "technical_goal"),
        ("ledger_versions", "execution_check"),
    ] {
        let count: i64 = db
            .query_row(
                "SELECT count(*) FROM pragma_table_info(?1) WHERE name=?2",
                params![table, column],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }
}

#[test]
fn failed_initialization_is_atomic() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE ledger_versions(conflict TEXT);")
        .unwrap();
    assert!(Store::initialize(&db).is_err());
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE type='table'",
            [],
            |r| r.get::<_, u32>(0)
        )
        .unwrap(),
        1
    );
    assert!(db.is_autocommit());
}

#[test]
fn deferred_plan_comment_claims_expire_after_user_input_or_plan_publication() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("core.db")).unwrap();
    let conversation = store.create_conversation("/project").unwrap();
    let id = &conversation.id;
    let user = store
        .add_message(id, "user", "revise only the first comment")
        .unwrap();
    assert_eq!(store.pending_plan_comment_ids(id).unwrap(), None);
    store
        .append_event(
            id,
            None,
            None,
            "plan_comments_addressed",
            &serde_json::json!({"ids":[1],"upto_message":user.seq}),
        )
        .unwrap();
    store
        .add_message(id, "options-planner", "regenerate the plan")
        .unwrap();
    assert_eq!(store.pending_plan_comment_ids(id).unwrap(), Some(vec![1]));
    let next = store.add_message(id, "user", "new scope").unwrap();
    assert_eq!(store.pending_plan_comment_ids(id).unwrap(), None);
    store
        .append_event(
            id,
            None,
            None,
            "plan_comments_addressed",
            &serde_json::json!({"ids":[],"upto_message":next.seq}),
        )
        .unwrap();
    assert_eq!(store.pending_plan_comment_ids(id).unwrap(), Some(vec![]));
    store
        .append_event(id, None, None, "plan_proposed", &serde_json::json!({}))
        .unwrap();
    assert_eq!(store.pending_plan_comment_ids(id).unwrap(), None);
}

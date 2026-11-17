use super::*;

#[test]
fn incompatible_databases_are_rejected_without_writes() {
    for version in [0, 1, 5, 10, 11, 12, 13, 14, 15, SCHEMA + 1] {
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
                && error.contains("runtime directory"),
            "{error}"
        );
        assert_eq!(bytes, std::fs::read(&path).unwrap());
        assert!(!path.with_extension("db-wal").exists());
    }
}

#[test]
fn database_without_ledger_rejections_is_rejected_before_use() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.db");
    let store = Store::open(&path).unwrap();
    let conversation = store.create_conversation("project").unwrap();
    {
        let db = store.db();
        db.execute_batch("DROP TABLE ledger_rejections;").unwrap();
        db.pragma_update(None, "user_version", 11).unwrap();
    }
    drop(store);
    let bytes = std::fs::read(&path).unwrap();
    let error = Store::open(&path).err().unwrap().to_string();
    assert!(error.contains("incompatible schema 11"), "{error}");
    assert_eq!(bytes, std::fs::read(&path).unwrap());
    let db = Connection::open(&path).unwrap();
    let id: String = db
        .query_row("SELECT id FROM conversations", [], |row| row.get(0))
        .unwrap();
    assert_eq!(id, conversation.id);
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
    db.prepare("SELECT proposal_id, comment_message, resume_message, diff_hash, created_ms FROM ledger_rejections")
        .unwrap();
    for (table, column) in [
        ("ledger_entries", "kind"),
        ("ledger_versions", "technical_goal"),
        ("ledger_versions", "execution_check"),
        ("run_contexts", "backend"),
        ("run_contexts", "provider_turn_id"),
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
fn comment_operation_retries_return_the_same_persisted_comment() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("core.db")).unwrap();
    let conversation = store.create_conversation("project").unwrap();
    let operation = uuid::Uuid::new_v4().to_string();
    let comment = Comment {
        id: 0,
        plan_id: None,
        target: "worker:task-uuid".into(),
        label: "worker task".into(),
        text: "address this line".into(),
        status: "open".into(),
        message: 0,
    };
    let send = |id| format!("[user comment #{id}] address this line");
    let first = store
        .add_comment_at_with_operation(&conversation.id, &operation, &comment, None, None, send)
        .unwrap();
    let retry = store
        .add_comment_at_with_operation(&conversation.id, &operation, &comment, None, None, send)
        .unwrap();
    assert_eq!(retry.id, first.id);
    assert_eq!(store.comments(&conversation.id, "open").unwrap().len(), 1);
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
        .add_message(id, "requirements-planner", "regenerate the plan")
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

#[test]
fn plan_review_progress_survives_restart_and_resets_for_new_user_input() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.db");
    let store = Store::open(&path).unwrap();
    let conversation = store.create_conversation("project").unwrap().id;
    store
        .add_message(&conversation, "user", "Plan this request")
        .unwrap();
    assert!(
        store
            .previous_plan_rejection(&conversation)
            .unwrap()
            .is_none()
    );
    store.append_event(&conversation,None,None,"plan_review",&serde_json::json!({"plan":"draft","approved":false,"findings":[{"id":"missing-input","severity":"high","issue":"Input is missing","where":"task"}]})).unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    let review = store
        .previous_plan_rejection(&conversation)
        .unwrap()
        .unwrap();
    assert_eq!(review.findings[0]["id"], "missing-input");
    let other = store.create_conversation("other-project").unwrap().id;
    assert!(store.previous_plan_rejection(&other).unwrap().is_none());
    store
        .add_message(&conversation, "user", "Reconsider using this new input")
        .unwrap();
    assert!(
        store
            .previous_plan_rejection(&conversation)
            .unwrap()
            .is_none()
    );
}

#[test]
fn an_operation_runs_once_and_a_failed_attempt_can_be_retried() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("core.db")).unwrap();
    let conversation = store.create_conversation("project").unwrap();
    let id = crate::id();
    let failed = store.idempotent(&conversation.id, &id, "send", || {
        Err(crate::Error::Invalid("daemon busy".into()))
    });
    assert!(failed.is_err());
    let mut runs = 0;
    for _ in 0..2 {
        let result = store
            .idempotent(&conversation.id, &id, "send", || {
                runs += 1;
                Ok(serde_json::json!({"seq": runs}))
            })
            .unwrap();
        assert_eq!(result, serde_json::json!({"seq": 1}));
    }
    assert_eq!(runs, 1);
}

#[test]
fn default_and_nondefault_team_choices_are_recorded_once_and_deleted_with_the_session() {
    use crate::lean::tasks::Team;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("core.db")).unwrap();
    let conversation = store.create_conversation("project").unwrap();
    let id = &conversation.id;
    assert_eq!(store.team(id).unwrap(), Team::Feature);
    assert_eq!(store.team_guidance(id).unwrap(), None);
    let selected = |store: &Store| {
        store
            .events_since(id, 0, 1000)
            .unwrap()
            .into_iter()
            .filter(|event| event.kind == "team_selected")
            .count()
    };
    store.select_team(id, Team::Feature, "").unwrap();
    assert_eq!(selected(&store), 1);
    store.select_team(id, Team::Feature, "").unwrap();
    assert_eq!(selected(&store), 1);
    for _ in 0..2 {
        store
            .select_team(id, Team::SmallChange, "one-line typo fix")
            .unwrap();
    }
    assert_eq!(selected(&store), 2);
    assert_eq!(store.team(id).unwrap(), Team::SmallChange);
    assert_eq!(
        store.team_guidance(id).unwrap().unwrap().reason,
        "one-line typo fix"
    );
    assert_eq!(
        store.user_selected_team(id).unwrap(),
        Some(Team::SmallChange)
    );
    store
        .select_planned_team(id, Team::Refactoring, "planner chose refactoring")
        .unwrap();
    assert_eq!(store.team(id).unwrap(), Team::SmallChange);
    assert_eq!(
        store.user_selected_team(id).unwrap(),
        Some(Team::SmallChange)
    );
    assert_eq!(
        store.team_guidance(id).unwrap().unwrap().origin,
        crate::lean::store::TeamOrigin::User
    );
    store
        .select_team(id, Team::Refactoring, "user changed their mind")
        .unwrap();
    assert_eq!(store.team(id).unwrap(), Team::Refactoring);
    assert_eq!(
        store.team_guidance(id).unwrap().unwrap().team,
        Team::Refactoring
    );
    store.delete_session(id).unwrap();
    let again = store.create_conversation("project").unwrap();
    assert_eq!(store.team(&again.id).unwrap(), Team::Feature);
}

#[test]
fn migrating_legacy_team_keeps_unknown_until_a_new_explicit_choice() {
    use crate::lean::tasks::Team;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.db");
    let conversation_id;
    {
        let store = Store::open(&path).unwrap();
        let conversation = store.create_conversation("project").unwrap();
        conversation_id = conversation.id;
        store
            .select_team(
                &conversation_id,
                Team::SmallChange,
                "User selected small-change rigor mode in UI before provenance existed",
            )
            .unwrap();
    }

    let legacy = Connection::open(&path).unwrap();
    legacy
        .execute_batch(
            "ALTER TABLE teams DROP COLUMN origin;
             UPDATE teams SET user_selected=1;
             CREATE TABLE migration_parent(id INTEGER PRIMARY KEY);
             CREATE TABLE migration_child(parent_id INTEGER REFERENCES migration_parent(id) DEFERRABLE INITIALLY DEFERRED);
             CREATE TRIGGER fail_team_origin_migration AFTER UPDATE OF origin ON teams
             BEGIN INSERT INTO migration_child(parent_id) VALUES(999); END;",
        )
        .unwrap();
    drop(legacy);

    let error = Store::open(&path).err().unwrap().to_string();
    assert!(error.contains("FOREIGN KEY constraint failed"), "{error}");
    let legacy = Connection::open(&path).unwrap();
    let origin_column: i64 = legacy
        .query_row(
            "SELECT count(*) FROM pragma_table_info('teams') WHERE name='origin'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(origin_column, 0, "failed migration must roll back its DDL");
    let (legacy_selected, team, reason): (i64, String, String) = legacy
        .query_row(
            "SELECT user_selected,team,reason FROM teams WHERE conversation_id=?1",
            [&conversation_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        legacy_selected, 1,
        "failed migration must roll back data updates"
    );
    assert_eq!(team, Team::SmallChange.name());
    assert_eq!(
        reason,
        "User selected small-change rigor mode in UI before provenance existed"
    );
    legacy
        .execute_batch("DROP TRIGGER fail_team_origin_migration")
        .unwrap();
    drop(legacy);

    let upgraded = Store::open(&path).unwrap();
    assert_eq!(upgraded.team(&conversation_id).unwrap(), Team::SmallChange);
    assert_eq!(upgraded.user_selected_team(&conversation_id).unwrap(), None);
    assert_eq!(
        upgraded.team_origin(&conversation_id).unwrap(),
        Some(crate::lean::store::TeamOrigin::Unknown)
    );
    let guidance = upgraded.team_guidance(&conversation_id).unwrap().unwrap();
    assert_eq!(guidance.origin, crate::lean::store::TeamOrigin::Unknown);
    assert_eq!(guidance.team, Team::SmallChange);
    assert_eq!(
        guidance.reason,
        "User selected small-change rigor mode in UI before provenance existed"
    );
    let row: (String, String) = upgraded
        .db()
        .query_row(
            "SELECT team,reason FROM teams WHERE conversation_id=?1",
            [&conversation_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(row.0, Team::SmallChange.name());
    assert_eq!(
        row.1,
        "User selected small-change rigor mode in UI before provenance existed"
    );
    drop(upgraded);

    let reopened = Store::open(&path).unwrap();
    assert_eq!(reopened.team(&conversation_id).unwrap(), Team::SmallChange);
    assert_eq!(reopened.user_selected_team(&conversation_id).unwrap(), None);
    assert_eq!(
        reopened.team_origin(&conversation_id).unwrap(),
        Some(crate::lean::store::TeamOrigin::Unknown)
    );
    let guidance = reopened.team_guidance(&conversation_id).unwrap().unwrap();
    assert_eq!(guidance.team, Team::SmallChange);
    assert_eq!(guidance.origin, crate::lean::store::TeamOrigin::Unknown);
    assert_eq!(
        guidance.reason,
        "User selected small-change rigor mode in UI before provenance existed"
    );
    reopened
        .select_team(&conversation_id, Team::Refactoring, "explicit new choice")
        .unwrap();
    assert_eq!(
        reopened.user_selected_team(&conversation_id).unwrap(),
        Some(Team::Refactoring)
    );
    assert_eq!(
        reopened.team_origin(&conversation_id).unwrap(),
        Some(crate::lean::store::TeamOrigin::User)
    );
    drop(reopened);

    let selected_reopened = Store::open(&path).unwrap();
    assert_eq!(
        selected_reopened
            .user_selected_team(&conversation_id)
            .unwrap(),
        Some(Team::Refactoring)
    );
}

#[test]
fn concurrent_legacy_opens_do_not_erase_a_new_user_team_choice() {
    use crate::lean::tasks::Team;
    use std::sync::{Arc, Barrier};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.db");
    let conversation_id;
    {
        let store = Store::open(&path).unwrap();
        let conversation = store.create_conversation("project").unwrap();
        conversation_id = conversation.id;
        store
            .select_planned_team(&conversation_id, Team::SmallChange, "planner choice")
            .unwrap();
    }
    Connection::open(&path)
        .unwrap()
        .execute_batch(
            "ALTER TABLE teams DROP COLUMN origin; ALTER TABLE teams DROP COLUMN user_selected",
        )
        .unwrap();

    let barrier = Arc::new(Barrier::new(3));
    let open_path = path.clone();
    let open_barrier = Arc::clone(&barrier);
    let opener = std::thread::spawn(move || {
        open_barrier.wait();
        let _store = Store::open(&open_path).unwrap();
    });

    let write_path = path.clone();
    let write_conversation = conversation_id.clone();
    let write_barrier = Arc::clone(&barrier);
    let writer = std::thread::spawn(move || {
        write_barrier.wait();
        let store = Store::open(&write_path).unwrap();
        store
            .select_team(
                &write_conversation,
                Team::Refactoring,
                "user selected after upgrade",
            )
            .unwrap();
    });

    barrier.wait();
    opener.join().unwrap();
    writer.join().unwrap();

    let reopened = Store::open(&path).unwrap();
    assert_eq!(reopened.team(&conversation_id).unwrap(), Team::Refactoring);
    assert_eq!(
        reopened.user_selected_team(&conversation_id).unwrap(),
        Some(Team::Refactoring)
    );
}

#[test]
fn adding_user_selection_flag_preserves_structured_user_origin() {
    use crate::lean::tasks::Team;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.db");
    let conversation_id;
    {
        let store = Store::open(&path).unwrap();
        let conversation = store.create_conversation("project").unwrap();
        conversation_id = conversation.id;
        store
            .select_team(&conversation_id, Team::SmallChange, "explicit UI selection")
            .unwrap();
    }

    Connection::open(&path)
        .unwrap()
        .execute_batch("ALTER TABLE teams DROP COLUMN user_selected")
        .unwrap();

    let upgraded = Store::open(&path).unwrap();
    assert_eq!(upgraded.team(&conversation_id).unwrap(), Team::SmallChange);
    assert_eq!(
        upgraded.team_origin(&conversation_id).unwrap(),
        Some(crate::lean::store::TeamOrigin::User)
    );
    assert_eq!(
        upgraded.user_selected_team(&conversation_id).unwrap(),
        Some(Team::SmallChange)
    );
}

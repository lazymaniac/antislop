use super::*;

#[test]
fn original_documents_remain_external_immutable_and_bound_to_the_scope_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.db");
    let store = Store::open(&path).unwrap();
    let conversation = store.create_conversation("project").unwrap().id;
    store
        .add_message(&conversation, "user", "Build an exporter 👩‍💻")
        .unwrap();
    let text = format!(
        "{}\nNever send private records over the network.\n{}",
        "before\n".repeat(10_000),
        "after\n".repeat(10_000)
    );
    let source = store.register_source(&conversation, &text).unwrap();
    assert_eq!(store.register_source(&conversation, &text).unwrap(), source);
    let scope = store.create_scope(&conversation, "Export locally").unwrap();
    store
        .register_source(&conversation, "Later document outside this scope")
        .unwrap();
    assert!(
        !store
            .scope_documents_current(&conversation, &scope.id)
            .unwrap()
    );
    assert!(
        store
            .additional_source_context(&conversation, Some(&scope.id))
            .unwrap()
            .contains("Additional original document")
    );
    store
        .add_message(&conversation, "user", "Later correction")
        .unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.source_text(&conversation, &source.id).unwrap(), text);
    let context = store.scope_sources(&conversation, &scope.id).unwrap();
    assert!(context.contains("Build an exporter 👩‍💻"));
    assert!(!context.contains("Later correction"));
    assert!(!context.contains("Never send private records"));
    assert_eq!(
        store
            .db()
            .query_row(
                "SELECT count(*) FROM scope_documents WHERE scope_id=?1",
                [&scope.id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    let reference: crate::lean::content::Reference =
        serde_json::from_str(&source.reference).unwrap();
    let artifact = store.content.path(&reference).unwrap();
    std::fs::write(&artifact, "tampered").unwrap();
    assert!(store.scope_sources(&conversation, &scope.id).is_err());
    let foreign = store.create_conversation("foreign").unwrap().id;
    assert!(store.source_text(&foreign, &source.id).is_err());
}

#[test]
fn deleting_a_scope_session_collects_documents_without_deleting_other_sources() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("core.db")).unwrap();
    let first = store.create_conversation("project").unwrap().id;
    let second = store.create_conversation("project").unwrap().id;
    let removed = store.register_source(&first, "Remove this source").unwrap();
    let retained = store.register_source(&second, "Keep this source").unwrap();
    store.create_scope(&first, "First scope").unwrap();
    store.create_scope(&second, "Second scope").unwrap();
    store.delete_session(&first).unwrap();
    assert!(store.source_text(&first, &removed.id).is_err());
    assert_eq!(
        store.source_text(&second, &retained.id).unwrap(),
        "Keep this source"
    );
    assert_eq!(
        store
            .db()
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE type='table' AND name LIKE 'ledger_%'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

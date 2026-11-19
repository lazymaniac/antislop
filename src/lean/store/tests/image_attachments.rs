use super::*;

#[test]
fn image_bytes_and_message_link_survive_store_reopen_and_source_removal() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("core.db");
    let source = directory.path().join("source.png");
    let png = crate::lean::content::valid_png_fixture();
    std::fs::write(&source, &png).unwrap();
    let store = Store::open(&database).unwrap();
    let conversation = store.create_conversation("project").unwrap();
    let message = store
        .add_message_with_images(
            &conversation.id,
            "user",
            "Inspect this",
            std::slice::from_ref(&source),
        )
        .unwrap();
    let linked = store.linked_images(&conversation.id).unwrap();
    assert_eq!(linked.len(), 1);
    assert_eq!(linked[0].0, message.seq);
    let reference = linked[0].1.clone();
    drop(store);
    std::fs::remove_file(&source).unwrap();

    let reopened = Store::open(&database).unwrap();
    let linked_after_restart = reopened.linked_images(&conversation.id).unwrap();
    assert_eq!(linked_after_restart, vec![(message.seq, reference.clone())]);
    assert_eq!(reopened.read_image(&reference).unwrap(), png);
    let db = reopened.db();
    let text: String = db
        .query_row(
            "SELECT text FROM messages WHERE seq=?1",
            [message.seq as i64],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!text.contains("iVBOR"));
    assert!(!text.contains("PNG"));
}

#[test]
fn invalid_and_unreadable_attachments_fail_without_storing_a_message() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("core.db");
    let store = Store::open(&database).unwrap();
    let conversation = store.create_conversation("project").unwrap();
    let invalid = directory.path().join("notes.txt");
    std::fs::write(&invalid, b"not an image").unwrap();
    let error = store
        .add_message_with_images(&conversation.id, "user", "draft survives", &[invalid])
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("unsupported or invalid image attachment")
    );
    let missing = directory.path().join("missing.png");
    assert!(
        store
            .add_message_with_images(&conversation.id, "user", "draft survives", &[missing])
            .unwrap_err()
            .to_string()
            .contains("cannot read image")
    );
    let count: i64 = store
        .db()
        .query_row(
            "SELECT count(*) FROM messages WHERE conversation_id=?1",
            [&conversation.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

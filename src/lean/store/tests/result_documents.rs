use super::*;
use serde_json::json;

#[test]
fn declared_text_outputs_survive_reopen_and_remain_distinct() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("core.db");
    let store = Store::open(&database).unwrap();
    let conversation = store
        .create_conversation(directory.path().to_str().unwrap())
        .unwrap();
    let task = store
        .add_plan(
            &conversation.id,
            &[json!({
                "id":"writer",
                "title":"Write documents",
                "goal":"Create two deliverables",
                "kind":"writing",
                "change_kind":"artifact",
                "outputs":["brief.md", "notes.txt"],
                "reviewer":{"summary":"Check the documents"}
            })],
        )
        .unwrap()
        .remove(0);

    store.test_completion(
        &task,
        "Internal helper report: no paths qualify automatically.",
    );
    assert!(store.result_documents(&conversation.id).unwrap().is_empty());
    store
        .record_result_document(
            &conversation.id,
            &task.id,
            "brief.md",
            "commit-a",
            "Plan for café",
        )
        .unwrap();
    store
        .record_result_document(
            &conversation.id,
            &task.id,
            "notes.txt",
            "commit-a",
            "Second deliverable",
        )
        .unwrap();
    store
        .record_result_document(
            &conversation.id,
            &task.id,
            "brief.md",
            "commit-a",
            "retry should be idempotent",
        )
        .unwrap();
    store
        .record_result_document(
            &conversation.id,
            &task.id,
            "brief.md",
            "commit-b",
            "Revised plan",
        )
        .unwrap();
    let listed = store.result_documents(&conversation.id).unwrap();
    assert_eq!(listed.len(), 3);
    assert_eq!(listed[0].path, "brief.md");
    assert_eq!(listed[1].path, "notes.txt");
    let first = store
        .result_document_chunk(&conversation.id, &listed[0].id, 0)
        .unwrap();
    assert_eq!(first.text, "Plan for café");
    assert!(first.done);
    assert_eq!(
        store
            .result_document_chunk(&conversation.id, &listed[2].id, 0)
            .unwrap()
            .text,
        "Revised plan"
    );

    drop(store);
    let reopened = Store::open(&database).unwrap();
    let restored = reopened.result_documents(&conversation.id).unwrap();
    assert_eq!(restored, listed);
    assert_eq!(
        reopened
            .result_document_chunk(&conversation.id, &restored[1].id, 0)
            .unwrap()
            .text,
        "Second deliverable"
    );
    // Crash window: publication succeeds but reference insertion fails. The
    // immutable orphan is collectible and does not replace existing versions.
    reopened.db().execute_batch("CREATE TRIGGER fail_document_reference BEFORE INSERT ON session_documents WHEN NEW.source_commit='injected-failure' BEGIN SELECT RAISE(FAIL,'reference commit failed'); END;").unwrap();
    assert!(
        reopened
            .record_result_document(
                &conversation.id,
                &task.id,
                "brief.md",
                "injected-failure",
                "orphaned publication"
            )
            .unwrap_err()
            .to_string()
            .contains("reference commit failed")
    );
    assert_eq!(
        reopened.result_documents(&conversation.id).unwrap().len(),
        3
    );
    assert_eq!(
        reopened
            .content
            .remove_unreferenced(&reopened.db())
            .unwrap(),
        1
    );
    // Deletion between preparation and publication cannot resurrect a document.
    let prepared = reopened
        .spool_text_content("prepared after deletion".as_bytes())
        .unwrap()
        .unwrap();
    reopened.delete_session(&conversation.id).unwrap();
    assert!(
        reopened
            .record_spooled_document(
                &conversation.id,
                &task.id,
                "brief.md",
                "deleted-session",
                prepared
            )
            .is_err()
    );
    assert!(
        reopened
            .result_documents(&conversation.id)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn undeclared_report_paths_cannot_become_result_documents() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("core.db")).unwrap();
    let conversation = store
        .create_conversation(directory.path().to_str().unwrap())
        .unwrap();
    let task = store
        .add_plan(
            &conversation.id,
            &[json!({
                "id":"writer",
                "title":"Write a document",
                "goal":"Create a deliverable",
                "kind":"writing",
                "change_kind":"artifact",
                "outputs":["deliverable.md"],
                "reviewer":{"summary":"Check the document"}
            })],
        )
        .unwrap()
        .remove(0);

    assert!(
        store
            .record_result_document(
                &conversation.id,
                &task.id,
                "agent-report.txt",
                "commit-a",
                "helper report"
            )
            .is_err()
    );
    assert!(store.result_documents(&conversation.id).unwrap().is_empty());
}

#[test]
fn publication_racing_session_delete_never_resurrects_or_leaks_content() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("core.db");
    let store = std::sync::Arc::new(Store::open(&database).unwrap());
    let conversation = store
        .create_conversation(directory.path().to_str().unwrap())
        .unwrap();
    let task = store
        .add_plan(
            &conversation.id,
            &[json!({
                "id":"writer",
                "title":"Write a document",
                "goal":"Create a deliverable",
                "kind":"writing",
                "change_kind":"artifact",
                "outputs":["deliverable.md"],
                "reviewer":{"summary":"Check the document"}
            })],
        )
        .unwrap()
        .remove(0);
    let prepared = store
        .spool_text_content("durable text".as_bytes())
        .unwrap()
        .unwrap();
    let start = std::sync::Arc::new(std::sync::Barrier::new(3));
    let publisher = {
        let store = store.clone();
        let conversation = conversation.id.clone();
        let task = task.id.clone();
        let start = start.clone();
        std::thread::spawn(move || {
            start.wait();
            store.record_spooled_document(
                &conversation,
                &task,
                "deliverable.md",
                "concurrent-commit",
                prepared,
            )
        })
    };
    let deleter = {
        let store = store.clone();
        let conversation = conversation.id.clone();
        let start = start.clone();
        std::thread::spawn(move || {
            start.wait();
            store.delete_session(&conversation)
        })
    };
    start.wait();
    let publication = publisher.join().unwrap();
    deleter.join().unwrap().unwrap();
    if let Err(error) = publication {
        assert!(error.to_string().contains("declared output"), "{error}");
    }
    assert!(store.result_documents(&conversation.id).unwrap().is_empty());
    assert_eq!(store.content.remove_unreferenced(&store.db()).unwrap(), 0);
    let content = database.with_file_name("session-content");
    assert!(!content.exists() || std::fs::read_dir(content).unwrap().count() == 0);
}

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

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
                "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Create two deliverables",
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
    // If reference insertion fails after publication, the unreferenced file is
    // collected before the failed write returns and cannot replace a version.
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
    let mut reopened_db = reopened.db();
    assert_eq!(
        reopened
            .content
            .remove_unreferenced(&mut reopened_db)
            .unwrap(),
        0
    );
    drop(reopened_db);
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
                "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Create a deliverable",
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
                "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Create a deliverable",
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
    let mut db = store.db();
    assert_eq!(store.content.remove_unreferenced(&mut db).unwrap(), 0);
    let content = database.with_file_name("session-content");
    assert!(!content.exists() || std::fs::read_dir(content).unwrap().count() == 0);
}

#[test]
fn paused_directory_sync_does_not_block_an_independent_publication() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("core.db");
    let store = std::sync::Arc::new(Store::open(&database).unwrap());
    let conversation = store
        .create_conversation(directory.path().to_str().unwrap())
        .unwrap();
    let tasks = store
        .add_plan(
            &conversation.id,
            &[
                json!({"id":"first","title":"First","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Write first","kind":"writing","change_kind":"artifact","outputs":["first.md"],"reviewer":{"summary":"Review"}}),
                json!({"id":"second","title":"Second","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Write second","kind":"writing","change_kind":"artifact","outputs":["second.md"],"reviewer":{"summary":"Review"}}),
            ],
        )
        .unwrap();
    let first = store
        .spool_text_content("first durable content".as_bytes())
        .unwrap()
        .unwrap();
    let second = store
        .spool_text_content("second durable content".as_bytes())
        .unwrap()
        .unwrap();

    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let release_rx = std::sync::Mutex::new(release_rx);
    let sync_count = std::sync::atomic::AtomicUsize::new(0);
    store.content.set_directory_sync_hook(move || {
        if sync_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            entered_tx.send(()).unwrap();
            release_rx.lock().unwrap().recv().unwrap();
        }
        Ok(())
    });

    let first_store = store.clone();
    let first_conversation = conversation.id.clone();
    let first_task = tasks[0].id.clone();
    let first_worker = std::thread::spawn(move || {
        first_store.record_spooled_document(
            &first_conversation,
            &first_task,
            "first.md",
            "commit-first",
            first,
        )
    });
    entered_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("first publication reached its blocked directory fsync");

    let second_store = store.clone();
    let second_conversation = conversation.id.clone();
    let second_task = tasks[1].id.clone();
    let second_worker = std::thread::spawn(move || {
        second_store.record_spooled_document(
            &second_conversation,
            &second_task,
            "second.md",
            "commit-second",
            second,
        )
    });
    let (finished_tx, finished_rx) = std::sync::mpsc::channel();
    let completion = std::thread::spawn(move || {
        finished_tx.send(second_worker.join().unwrap()).unwrap();
    });
    finished_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("independent publication completed while the first fsync was paused")
        .unwrap();
    release_tx.send(()).unwrap();
    first_worker.join().unwrap().unwrap();
    completion.join().unwrap();

    let documents = store.result_documents(&conversation.id).unwrap();
    assert_eq!(documents.len(), 2);
    drop(store);
    let reopened = Store::open(&database).unwrap();
    for document in reopened.result_documents(&conversation.id).unwrap() {
        let text = reopened
            .result_document_chunk(&conversation.id, &document.id, 0)
            .unwrap()
            .text;
        assert!(text == "first durable content" || text == "second durable content");
    }
}

#[test]
fn deletion_during_directory_sync_keeps_lease_then_collects_orphan_without_resurrection() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("core.db");
    let content_directory = database.with_file_name("session-content");
    let store = std::sync::Arc::new(Store::open(&database).unwrap());
    let conversation = store
        .create_conversation(directory.path().to_str().unwrap())
        .unwrap();
    let task = store
        .add_plan(
            &conversation.id,
            &[json!({"id":"writer","title":"Write","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Write a document","kind":"writing","change_kind":"artifact","outputs":["deliverable.md"],"reviewer":{"summary":"Review"}})],
        )
        .unwrap()
        .remove(0);
    let prepared = store
        .spool_text_content("must not become a dangling reference".as_bytes())
        .unwrap()
        .unwrap();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let release_rx = std::sync::Mutex::new(release_rx);
    store.content.set_directory_sync_hook(move || {
        entered_tx.send(()).unwrap();
        release_rx.lock().unwrap().recv().unwrap();
        Ok(())
    });

    let publisher_store = store.clone();
    let publisher_conversation = conversation.id.clone();
    let publisher_task = task.id.clone();
    let publisher = std::thread::spawn(move || {
        publisher_store.record_spooled_document(
            &publisher_conversation,
            &publisher_task,
            "deliverable.md",
            "commit-during-delete",
            prepared,
        )
    });
    entered_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("publisher reached its blocked directory fsync");
    store.delete_session(&conversation.id).unwrap();
    assert_eq!(std::fs::read_dir(&content_directory).unwrap().count(), 1);
    release_tx.send(()).unwrap();
    let error = publisher.join().unwrap().unwrap_err().to_string();
    assert!(error.contains("declared output"), "{error}");
    assert_eq!(std::fs::read_dir(&content_directory).unwrap().count(), 0);
    drop(store);

    let reopened = Store::open(&database).unwrap();
    let exists: bool = reopened
        .db()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM conversations WHERE id=?1)",
            [&conversation.id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!exists, "reopen must not recreate the deleted session");
    let mut reopened_db = reopened.db();
    assert_eq!(
        reopened
            .content
            .remove_unreferenced(&mut reopened_db)
            .unwrap(),
        0
    );
}

#[test]
fn directory_sync_failure_is_reported_and_published_orphan_is_collected() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("core.db");
    let content_directory = database.with_file_name("session-content");
    let store = Store::open(&database).unwrap();
    let conversation = store
        .create_conversation(directory.path().to_str().unwrap())
        .unwrap();
    let task = store
        .add_plan(
            &conversation.id,
            &[json!({"id":"writer","title":"Write","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Write a document","kind":"writing","change_kind":"artifact","outputs":["deliverable.md"],"reviewer":{"summary":"Review"}})],
        )
        .unwrap()
        .remove(0);
    store
        .content
        .set_directory_sync_hook(|| Err(std::io::Error::other("injected directory fsync failure")));
    let error = store
        .record_result_document(
            &conversation.id,
            &task.id,
            "deliverable.md",
            "sync-failure",
            "no reference before durable directory entry",
        )
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("injected directory fsync failure"),
        "{error}"
    );
    assert!(store.result_documents(&conversation.id).unwrap().is_empty());
    assert_eq!(std::fs::read_dir(&content_directory).unwrap().count(), 0);
}

#[test]
fn idempotency_read_failure_after_directory_sync_collects_published_orphan() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("core.db");
    let content_directory = database.with_file_name("session-content");
    let store = std::sync::Arc::new(Store::open(&database).unwrap());
    let conversation = store
        .create_conversation(directory.path().to_str().unwrap())
        .unwrap();
    let task = store
        .add_plan(
            &conversation.id,
            &[json!({"id":"writer","title":"Write","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Write a document","kind":"writing","change_kind":"artifact","outputs":["deliverable.md"],"reviewer":{"summary":"Review"}})],
        )
        .unwrap()
        .remove(0);

    let hook_store = store.clone();
    store.content.set_directory_sync_hook(move || {
        // Leave the task authorization query usable, but make the subsequent
        // idempotency lookup fail after the file has been directory-fsynced.
        hook_store
            .db()
            .execute_batch(
                "DROP TABLE session_documents; CREATE TABLE session_documents(unrelated TEXT);",
            )
            .map_err(std::io::Error::other)?;
        Ok(())
    });

    let error = store
        .record_result_document(
            &conversation.id,
            &task.id,
            "deliverable.md",
            "idempotency-read-failure",
            "published content must be collected when SQL lookup fails",
        )
        .unwrap_err()
        .to_string();
    assert!(error.contains("no such column: producer_task"), "{error}");
    assert!(
        !error.contains("unreferenced content cleanup also failed"),
        "cleanup should succeed after the injected lookup failure: {error}"
    );
    assert_eq!(std::fs::read_dir(&content_directory).unwrap().count(), 0);
}

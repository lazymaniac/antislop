use super::*;

fn test_tempdir() -> tempfile::TempDir {
    std::fs::create_dir_all(".antislop/tmp").unwrap();
    tempfile::tempdir_in(".antislop/tmp").unwrap()
}

#[test]
fn deleting_a_session_removes_history_and_its_external_content() {
    let dir = test_tempdir();
    let store = Store::open(&dir.path().join("core.db")).unwrap();
    let conversation = store.create_conversation("/p").unwrap();
    let run = store
        .create_run(&conversation.id, "Remove this recorded goal")
        .unwrap();
    assert_eq!(
        store.conversation_goal(&conversation.id).unwrap(),
        "Remove this recorded goal"
    );
    let task = store
        .add_plan(
            &conversation.id,
            &[serde_json::json!({"id":"remove-me","title":"Purge","goal":"Remove the stored plan","change_kind":"none","kind":"analysis","reviewer":{"summary":"Check purge"}})],
        )
        .unwrap()
        .remove(0);
    store
        .link_plan_run(&conversation.id, &task.plan_id, &run.id)
        .unwrap();
    store
        .add_message(&conversation.id, "user", &"history ".repeat(20_000))
        .unwrap();
    let content_dir = dir.path().join("session-content");
    assert_eq!(std::fs::read_dir(&content_dir).unwrap().count(), 1);
    store.delete_session(&conversation.id).unwrap();
    assert!(store.conversation(&conversation.id).is_err());
    assert!(
        store
            .messages_after(&conversation.id, 0)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store
            .db()
            .query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        store
            .db()
            .query_row("SELECT COUNT(*) FROM runs", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(
        store
            .events_since(&conversation.id, 0, 100)
            .unwrap()
            .is_empty()
    );
    assert_eq!(std::fs::read_dir(content_dir).unwrap().count(), 0);
}

#[test]
fn deleting_a_session_cannot_remove_content_from_a_concurrent_message_write() {
    let dir = test_tempdir();
    let store = std::sync::Arc::new(Store::open(&dir.path().join("core.db")).unwrap());
    let deleted = store.create_conversation("/deleted").unwrap();
    let retained = store.create_conversation("/retained").unwrap();
    let content_write = store.content_write.lock().unwrap();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let deleting_store = std::sync::Arc::clone(&store);
    let deleting_id = deleted.id.clone();
    let delete = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        deleting_store.delete_session(&deleting_id)
    });
    started_rx.recv().unwrap();

    // This is the window that used to let deletion clean up the newly written
    // file before its message row could reference it.
    store
        .add_message_with_content_lock(&retained.id, "user", &"concurrent ".repeat(20_000))
        .unwrap();
    drop(content_write);
    delete.join().unwrap().unwrap();

    let messages = store.messages_after(&retained.id, 0).unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].text, "concurrent ".repeat(20_000));
    assert!(store.conversation(&retained.id).is_ok());
}

#[test]
fn deleting_a_session_with_a_running_turn_is_rejected() {
    let dir = test_tempdir();
    let store = Store::open(&dir.path().join("core.db")).unwrap();
    let conversation = store.create_conversation("/p").unwrap();
    store
        .start_turn(&conversation.id, "orchestrator", "agy", 0, None)
        .unwrap();
    let error = store
        .delete_session(&conversation.id)
        .unwrap_err()
        .to_string();
    assert!(error.contains("stop this session's work"));
    assert!(store.conversation(&conversation.id).is_ok());
}

#[test]
fn large_agent_text_is_kept_out_of_the_sqlite_event_row() {
    let dir = test_tempdir();
    let store = Store::open(&dir.path().join("core.db")).unwrap();
    let conversation = store.create_conversation("/p").unwrap();
    let text = "x".repeat(200_000);
    let seq = store
        .append_event(
            &conversation.id,
            None,
            None,
            "agent",
            &serde_json::json!({"type": "text", "text": text}),
        )
        .unwrap();
    let stored: String = store
        .db()
        .query_row(
            "SELECT payload FROM events WHERE seq=?1",
            [seq as i64],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        stored.len() < 256,
        "event row grew to {} bytes",
        stored.len()
    );
    let payload: Value = serde_json::from_str(&stored).unwrap();
    assert!(payload["text"]["session_content"].is_object());
    let mut restored = String::new();
    let mut offset = 0;
    loop {
        let chunk = store
            .session_content(&conversation.id, seq, "text", offset)
            .unwrap();
        assert!(chunk.text.len() <= crate::lean::content::CHUNK_BYTES);
        restored.push_str(&chunk.text);
        offset = chunk.next_offset;
        if chunk.done {
            break;
        }
    }
    assert_eq!(restored, text);
}

#[test]
fn running_turns_become_interrupted_on_restart() {
    let dir = test_tempdir();
    let store = Store::open(&dir.path().join("core.db")).unwrap();
    let conversation = store.create_conversation("/p").unwrap();
    store
        .start_turn(&conversation.id, "orchestrator", "agy", 0, None)
        .unwrap();
    drop(store);
    let store = Store::open(&dir.path().join("core.db")).unwrap();
    assert_eq!(
        store.interrupt_running_turns().unwrap(),
        [conversation.id.as_str()]
    );
    let turn = store
        .last_turn(&conversation.id, "orchestrator")
        .unwrap()
        .unwrap();
    assert_eq!(turn.status, TurnStatus::Interrupted);
    let events: Vec<_> = store
        .events_since(&conversation.id, 0, 10)
        .unwrap()
        .into_iter()
        .filter(|event| event.kind != "workflow")
        .collect();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "turn_finished");
    assert_eq!(events[0].payload["status"], "interrupted");
    assert!(
        events[0].payload["error"]
            .as_str()
            .unwrap()
            .contains("daemon stopped")
    );
    assert!(store.interrupt_running_turns().unwrap().is_empty());
    assert_eq!(
        store.events_since(&conversation.id, 0, 10).unwrap().len(),
        events.len() + 1,
        "the interview's workflow event and the interruption, nothing more"
    );
}

#[test]
fn history_budget_keeps_oversized_first_event_and_continues_from_its_sequence() {
    let dir = test_tempdir();
    let store = Store::open(&dir.path().join("core.db")).unwrap();
    let conversation = store.create_conversation("/p").unwrap();
    let after_setup = store.latest_event_seq(&conversation.id).unwrap();
    let first = store
        .append_event(
            &conversation.id,
            None,
            None,
            "message",
            &serde_json::json!({"text": "é".repeat(80)}),
        )
        .unwrap();
    let second = store
        .append_event(
            &conversation.id,
            None,
            None,
            "message",
            &serde_json::json!({"text": "next"}),
        )
        .unwrap();

    let page = store
        .events_since_budget(&conversation.id, after_setup, 10, 8)
        .unwrap();
    assert_eq!(page.len(), 1, "the first event is never dropped for budget");
    assert_eq!(page[0].seq, first);
    assert_eq!(page[0].payload["text"], "é".repeat(80));
    assert_eq!(
        store
            .events_since_budget(&conversation.id, page[0].seq, 10, 8)
            .unwrap()[0]
            .seq,
        second
    );
}

#[test]
fn repeated_events_keep_distinct_sequences_and_exclusive_cursor_order() {
    let dir = test_tempdir();
    let store = Store::open(&dir.path().join("core.db")).unwrap();
    let conversation = store.create_conversation("/p").unwrap();
    let cursor = store.latest_event_seq(&conversation.id).unwrap();
    let payload = serde_json::json!({"type": "agent_status", "status": "working"});

    let first = store
        .append_event(&conversation.id, None, None, "agent", &payload)
        .unwrap();
    let second = store
        .append_event(&conversation.id, None, None, "agent", &payload)
        .unwrap();

    assert!(second > first, "repeated events remain append-only");
    let events = store.events_since(&conversation.id, cursor, 10).unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!((events[0].seq, events[1].seq), (first, second));
    assert_eq!(events[0].payload, payload);
    assert_eq!(events[1].payload, payload);
    let after_first = store.events_since(&conversation.id, first, 10).unwrap();
    assert_eq!(after_first.len(), 1);
    assert_eq!(after_first[0].seq, second);
    assert!(
        store
            .events_since(&conversation.id, second, 10)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn project_identity_metadata_survives_reopen_without_permission_tables() {
    let dir = test_tempdir();
    let path = dir.path().join("core.db");
    let store = Store::open(&path).unwrap();
    store
        .record_project_identity("/workspace/project", "git-key")
        .unwrap();
    store
        .record_project_identity("/workspace/project", "git-key")
        .unwrap();
    assert!(
        store
            .record_project_identity("/workspace/project", "other-key")
            .is_err()
    );
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store
            .repository_id("/workspace/project")
            .unwrap()
            .as_deref(),
        Some("git-key")
    );
    let obsolete: i64 = store
        .db()
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='trusted_projects'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(obsolete, 0);
}

#[test]
fn restarting_queued_tasks_does_not_invent_reports_or_accumulate_notices() {
    let dir = test_tempdir();
    let store = Store::open(&dir.path().join("core.db")).unwrap();
    let conversation = store.create_conversation("/p").unwrap();
    let mut tasks = store.add_plan(&conversation.id, &[
        serde_json::json!({"id":"fresh","title":"Fresh","goal":"Run","change_kind": "none", "kind":"analysis"}),
        serde_json::json!({"id":"legacy","title":"Legacy","goal":"Run","change_kind": "none", "kind":"analysis"}),
        serde_json::json!({"id":"checkpoint","title":"Checkpoint","goal":"Continue","change_kind": "none", "kind":"analysis"}),
        serde_json::json!({"id":"working","title":"Working","goal":"Continue","change_kind": "none", "kind":"analysis"}),
        serde_json::json!({"id":"reviewing","title":"Reviewing","goal":"Continue","change_kind": "none", "kind":"analysis"})
    ]).unwrap();
    for task in &mut tasks {
        task.status = "queued".into();
        store.update_task(task).unwrap();
    }
    tasks[1].report = Some("[interrupted] the daemon stopped while this task ran\n\n".repeat(13));
    tasks[1].feedback = Some(serde_json::json!({"resume_role":"reviewer"}));
    store.update_task(&tasks[1]).unwrap();
    tasks[2].report = Some("Actual retained worker findings".into());
    store.update_task(&tasks[2]).unwrap();
    for (index, task_status) in [(3, "working"), (4, "reviewing")] {
        tasks[index].status = task_status.into();
        tasks[index].report = Some("Actual work before interruption".into());
        store.update_task(&tasks[index]).unwrap();
    }
    store.interrupt_unfinished_tasks().unwrap();
    let once = store.plan(&tasks[0].plan_id).unwrap();
    assert!(once[0].report.is_none());
    assert!(
        once[1].report.is_none(),
        "{:?}",
        once.iter()
            .map(|t| (&t.spec["id"], &t.report))
            .collect::<Vec<_>>()
    );
    assert!(
        store.completed_task_output(&once[1]).is_err(),
        "runtime notices cannot become worker completions"
    );
    assert!(
        once[2]
            .report
            .as_ref()
            .unwrap()
            .starts_with("Actual retained worker findings")
    );
    assert_eq!(
        once[2].report, tasks[2].report,
        "an untouched queued checkpoint is not an interrupted execution"
    );
    for index in [3, 4] {
        assert_eq!(once[index].status, "queued");
        assert!(
            once[index]
                .report
                .as_ref()
                .unwrap()
                .contains("[interrupted]")
        );
    }
    store.interrupt_unfinished_tasks().unwrap();
    let twice = store.plan(&tasks[0].plan_id).unwrap();
    assert!(twice[0].report.is_none());
    assert!(twice[1].report.is_none());
    for index in [2, 3, 4] {
        assert_eq!(twice[index].report, once[index].report);
    }
}

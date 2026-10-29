use super::*;

fn test_tempdir() -> tempfile::TempDir {
    std::fs::create_dir_all(".antislop/tmp").unwrap();
    tempfile::tempdir_in(".antislop/tmp").unwrap()
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
fn workspace_trust_round_trips() {
    let dir = test_tempdir();
    let store = Store::open(&dir.path().join("core.db")).unwrap();
    assert!(!store.is_trusted("/workspace/proj1").unwrap());
    store.trust_project("/workspace/proj1").unwrap();
    assert!(store.is_trusted("/workspace/proj1").unwrap());
    assert_eq!(
        store.trusted_projects().unwrap(),
        vec!["/workspace/proj1".to_string()]
    );
    store.untrust_project("/workspace/proj1").unwrap();
    assert!(!store.is_trusted("/workspace/proj1").unwrap());
}

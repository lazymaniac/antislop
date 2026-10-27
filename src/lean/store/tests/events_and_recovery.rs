use super::*;

#[test]
fn large_agent_text_is_kept_out_of_the_sqlite_event_row() {
    let dir = tempfile::tempdir().unwrap();
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
    let dir = tempfile::tempdir().unwrap();
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
fn workspace_trust_round_trips() {
    let dir = tempfile::tempdir().unwrap();
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

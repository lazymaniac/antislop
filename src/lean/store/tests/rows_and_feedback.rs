use super::*;

#[test]
fn approved_outputs_follow_completion_order_and_stay_with_their_project() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("core.db")).unwrap();
    let conversation = store.create_conversation("/project").unwrap();
    let specs = [
        serde_json::json!({"id": "t1"}),
        serde_json::json!({"id": "t2"}),
    ];
    let mut tasks = store.add_plan(&conversation.id, &specs).unwrap();
    for index in [1, 0] {
        let task = &mut tasks[index];
        task.status = "approved".into();
        task.base = Some(format!("base-{index}"));
        task.result = Some(format!("result-{index}"));
        store.update_task(task).unwrap();
        store
            .append_event(
                &conversation.id,
                None,
                None,
                "task",
                &serde_json::json!({"task": task.id, "status": "approved"}),
            )
            .unwrap();
    }
    let other = store.create_conversation("/other-project").unwrap();
    let mut task = store.add_plan(&other.id, &specs[..1]).unwrap().remove(0);
    task.status = "approved".into();
    task.base = Some("other-base".into());
    task.result = Some("other-result".into());
    store.update_task(&task).unwrap();
    store
        .append_event(
            &other.id,
            None,
            None,
            "task",
            &serde_json::json!({"task": task.id, "status": "approved"}),
        )
        .unwrap();
    assert_eq!(
        store.approved_writer_results("/project").unwrap(),
        [
            ("base-0".into(), "result-0".into()),
            ("base-1".into(), "result-1".into()),
        ]
    );
    // An old approval cannot qualify a task that now needs a decision.
    tasks[0].status = "needs_user".into();
    store.update_task(&tasks[0]).unwrap();
    assert_eq!(
        store.approved_writer_results("/project").unwrap(),
        [("base-1".into(), "result-1".into()),]
    );
}

#[test]
fn a_newer_schema_is_refused_not_rewritten() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.db");
    drop(Store::open(&path).unwrap());
    let connection = Connection::open(&path).unwrap();
    let version: u32 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA);
    connection
        .pragma_update(None, "user_version", SCHEMA + 1)
        .unwrap();
    drop(connection);
    let error = Store::open(&path).err().unwrap();
    assert!(error.to_string().contains("incompatible schema"), "{error}");
}

#[test]
fn checks_keep_their_evidence_and_never_go_back_to_checking() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("core.db")).unwrap();
    assert!(
        store
            .set_check("u1", 1, "R1", "checking", None, "")
            .unwrap()
    );
    assert!(
        !store
            .set_check("u1", 1, "R1", "checking", None, "")
            .unwrap()
    );
    assert!(
        store
            .set_check("u1", 1, "R1", "settled", Some("ok"), "a.rs:3")
            .unwrap()
    );
    assert!(
        !store
            .set_check("u1", 1, "R1", "checking", None, "")
            .unwrap()
    );
    // The verdict repeats the check without a note: the evidence stays.
    store
        .set_check("u1", 1, "R1", "settled", Some("ok"), "")
        .unwrap();
    let rows = store.checks("u1", 1).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["note"], "a.rs:3");
    assert_eq!(rows[0]["status"], "ok");
    assert!(store.checks("u1", 2).unwrap().is_empty());
}

#[test]
fn card_feedback_round_trips_and_rows_without_it_stay_readable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.db");
    let store = Store::open(&path).unwrap();
    store
        .db()
        .execute_batch(
            "INSERT INTO conversations VALUES('c','/p',0);
            INSERT INTO tasks(id,conversation_id,plan_id,position,spec,status,created_ms)
            VALUES('u1','c','p1',0,'{\"id\":\"t1\"}','queued',0);",
        )
        .unwrap();
    let before = store.plan("p1").unwrap().remove(0);
    assert_eq!(before.status, "queued");
    assert_eq!(
        before.feedback, None,
        "a task without feedback shows no card result"
    );
    assert_eq!(store.task_by_id("u1").unwrap().unwrap().feedback, None);

    // Feedback round-trips through both read paths.
    let feedback = serde_json::json!({"round": 2, "integration": {"outcome": "applied"}});
    store
        .update_task(&Task {
            feedback: Some(feedback.clone()),
            ..before.clone()
        })
        .unwrap();
    assert_eq!(
        store.plan("p1").unwrap()[0].feedback.as_ref(),
        Some(&feedback)
    );
    assert_eq!(
        store.task_by_id("u1").unwrap().unwrap().feedback.as_ref(),
        Some(&feedback)
    );
    // Clearing a retry's card really clears the stored row.
    store
        .update_task(&Task {
            feedback: None,
            ..before.clone()
        })
        .unwrap();
    assert_eq!(store.plan("p1").unwrap()[0].feedback, None);
    // The task row serializes without the key when there is no card data.
    assert!(
        serde_json::to_value(&before)
            .unwrap()
            .get("feedback")
            .is_none()
    );
}

#[test]
fn rows_and_events_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("core.db")).unwrap();
    let conversation = store.create_conversation("/p").unwrap();
    let first = store
        .add_message(&conversation.id, "user", "hello")
        .unwrap();
    let turn = store
        .start_turn(&conversation.id, "orchestrator", "codex", first.seq, None)
        .unwrap();
    store.set_session(&turn.id, "s1").unwrap();
    store
        .finish_turn(&turn.id, TurnStatus::Failed, Some("boom"))
        .unwrap();
    let last = store
        .last_turn(&conversation.id, "orchestrator")
        .unwrap()
        .unwrap();
    assert_eq!(last.status, TurnStatus::Failed);
    assert_eq!(last.error.as_deref(), Some("boom"));
    assert_eq!(
        store
            .last_session(&conversation.id, "orchestrator", "codex")
            .unwrap()
            .as_deref(),
        Some("s1")
    );
    // Another CLI cannot resume a codex session.
    assert_eq!(
        store
            .last_session(&conversation.id, "orchestrator", "agy")
            .unwrap(),
        None
    );
    let events: Vec<_> = store
        .events_since(&conversation.id, 0, 10)
        .unwrap()
        .into_iter()
        .filter(|event| event.kind != "workflow")
        .collect();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].payload["text"], "hello");
    assert!(
        store
            .events_since(&conversation.id, events[0].seq, 10)
            .unwrap()
            .is_empty()
    );
}

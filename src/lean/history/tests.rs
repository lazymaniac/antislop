use super::*;

#[cfg(test)]
mod mail_tests {
    use super::*;

    #[test]
    fn orchestrator_mail_keeps_its_user_message_source_without_schema_changes() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(&directory.path().join("core.db")).unwrap();
        let conversation = store.create_conversation("project").unwrap();
        let other = store.create_conversation("other").unwrap();
        let mut pairs = vec![];
        for _ in 0..2 {
            let receipt = store
                .enqueue_agent_message(&AgentMessage {
                    conversation: conversation.id.clone(),
                    plan: None,
                    recipient: "orchestrator".into(),
                    operation_id: crate::id(),
                    expected_round: None,
                    expected_run: None,
                    text: "same correction 👩‍💻".into(),
                })
                .unwrap();
            let mail = receipt["mail"].as_u64().unwrap();
            let message = store
                .messages_by(&conversation.id, "user", 0)
                .unwrap()
                .last()
                .unwrap()
                .seq;
            assert_eq!(
                store
                    .agent_mail_user_message(&conversation.id, mail)
                    .unwrap(),
                Some(message)
            );
            assert_eq!(
                store.agent_mail_user_message(&other.id, mail).unwrap(),
                None
            );
            pairs.push((mail, message));
        }
        assert_ne!(pairs[0].1, pairs[1].1);
        // Schema 11 records made before source metadata was added retain
        // their atomic mail/message event pair and need no migration.
        store.db().execute("UPDATE events SET payload=json_remove(payload,'$.user_message') WHERE conversation_id=?1 AND kind='mail'",[&conversation.id]).unwrap();
        for (mail, message) in pairs {
            assert_eq!(
                store
                    .agent_mail_user_message(&conversation.id, mail)
                    .unwrap(),
                Some(message)
            );
        }
        let schema: u32 = store
            .db()
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(schema, 11);
    }

    #[test]
    fn session_events_page_references_and_enforce_ownership() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("core.db");
        let store = Store::open(&path).unwrap();
        let conversation = store.create_conversation("project").unwrap();
        let other = store.create_conversation("other").unwrap();
        let turn = store
            .start_turn(&conversation.id, "orchestrator", "codex", 0, None)
            .unwrap();
        let text = "é👩‍💻".repeat(600_000);
        let seq = store
            .append_event(
                &conversation.id,
                Some(&turn.id),
                None,
                "agent",
                &serde_json::json!({"type":"prompt","text":text}),
            )
            .unwrap();
        store
            .append_event(
                &conversation.id,
                Some(&turn.id),
                None,
                "agent_final",
                &serde_json::json!({"text":text}),
            )
            .unwrap();
        let page = store
            .session_events(&conversation.id, &turn.id, 0, 1)
            .unwrap();
        assert_eq!(page["has_more"], true);
        assert_eq!(page["events"][0]["seq"], seq);
        assert!(page.to_string().len() < 1000);
        assert!(store.session_events(&other.id, &turn.id, 0, 1).is_err());
        assert!(store.session_content(&other.id, seq, "text", 0).is_err());
        let mut restored = String::new();
        let mut offset = 0;
        loop {
            let chunk = store
                .session_content(&conversation.id, seq, "text", offset)
                .unwrap();
            restored.push_str(&chunk.text);
            if chunk.done {
                break;
            }
            offset = chunk.next_offset;
        }
        assert_eq!(restored, text);
        drop(store);
        let reopened = Store::open(&path).unwrap();
        let events = reopened.events_since(&conversation.id, seq - 1, 2).unwrap();
        assert_eq!(events[0].payload["text"], text);
        assert_eq!(events[1].payload["text"], text);
    }
}

#[cfg(test)]
mod round_tests {
    use super::*;

    #[test]
    fn rounds_keep_results_turn_scope_and_idempotent_messages() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("core.db");
        let store = Store::open(&path).unwrap();
        let conversation = store.create_conversation("project").unwrap();
        let mut task = store
            .add_plan(
                &conversation.id,
                &[serde_json::json!({"id":"t1","title":"task"})],
            )
            .unwrap()
            .remove(0);
        task.round = 1;
        task.status = "working".into();
        task.report = Some("report one".into());
        store.update_task(&task).unwrap();
        let role = format!("worker:{}", task.id);
        let turn = store
            .start_turn(&conversation.id, &role, "codex", 0, Some(&task.plan_id))
            .unwrap();
        task.status = "approved".into();
        task.base = Some("base-one".into());
        task.result = Some("result-one".into());
        store.update_task(&task).unwrap();
        let request = AgentMessage {
            conversation: conversation.id.clone(),
            plan: Some(task.plan_id.clone()),
            recipient: role,
            operation_id: uuid::Uuid::new_v4().to_string(),
            expected_round: Some(1),
            expected_run: None,
            text: "fix 👩‍💻".into(),
        };
        let receipt = store.enqueue_agent_message(&request).unwrap();
        assert_eq!(
            store.enqueue_agent_message(&request).unwrap()["mail"],
            receipt["mail"]
        );
        assert_eq!(
            store
                .pending_mail(&conversation.id, &request.recipient)
                .unwrap()
                .len(),
            1
        );
        let queued = store.task_by_id(&task.id).unwrap().unwrap();
        assert_eq!(queued.status, "queued");
        assert!(queued.result.is_none());
        let old = store.task_round(&task.id, 1).unwrap();
        assert_eq!(old["status"], "approved");
        assert_eq!(old["result"], "result-one");
        assert_eq!(old["turns"][0]["turn_id"], turn.id);
        let mut conflicting = request.clone();
        conflicting.text = "different".into();
        assert!(store.enqueue_agent_message(&conflicting).is_err());
        conflicting.operation_id = uuid::Uuid::new_v4().to_string();
        conflicting.expected_round = Some(0);
        assert!(store.enqueue_agent_message(&conflicting).is_err());
        task.round = 2;
        task.report = Some("report two".into());
        store.update_task(&task).unwrap();
        assert_eq!(store.task_rounds(&task.id).unwrap().len(), 2);
        assert_eq!(
            store.task_round(&task.id, 1).unwrap()["report"],
            "report one"
        );
        let new_turn = store
            .start_turn(
                &conversation.id,
                &request.recipient,
                "codex",
                0,
                Some(&task.plan_id),
            )
            .unwrap();
        store
            .mark_delivered(
                &conversation.id,
                &[receipt["mail"].as_u64().unwrap()],
                &new_turn.id,
            )
            .unwrap();
        assert!(
            store.task_round(&task.id, 1).unwrap()["messages"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            store.task_round(&task.id, 2).unwrap()["messages"][0]["id"],
            receipt["mail"]
        );
        let page = store
            .history_messages(
                &conversation.id,
                &HistoryScope::Round {
                    task: task.id.clone(),
                    round: 2,
                },
                0,
                1,
            )
            .unwrap();
        assert_eq!(page["messages"][0]["id"], receipt["mail"]);
        assert!(page["messages"][0]["content"]["session_content"].is_object());
        let foreign = store.create_conversation("foreign").unwrap();
        assert!(
            store
                .history_content(
                    &foreign.id,
                    &ContentOwner::Mail {
                        id: receipt["mail"].as_u64().unwrap()
                    },
                    0
                )
                .is_err()
        );
        assert!(
            store
                .history_messages(
                    &foreign.id,
                    &HistoryScope::Round {
                        task: task.id.clone(),
                        round: 2
                    },
                    0,
                    1
                )
                .is_err()
        );
        let refs = store.task_round_with_references(&task.id, 2, true).unwrap();
        assert!(refs["report"]["session_content"].is_object());
        assert_eq!(
            store
                .history_content(
                    &conversation.id,
                    &ContentOwner::Round {
                        task: task.id.clone(),
                        round: 2,
                        field: "report".into()
                    },
                    0
                )
                .unwrap()
                .text,
            "report two"
        );
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store.task_round(&task.id, 2).unwrap()["report"],
            "report two"
        );
        assert_eq!(
            store.enqueue_agent_message(&request).unwrap()["mail"],
            receipt["mail"]
        );
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;

    #[test]
    fn failed_round_attribution_rolls_back_the_turn() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("core.db")).unwrap();
        let conversation = store.create_conversation("project").unwrap();
        let task = store
            .add_plan(
                &conversation.id,
                &[serde_json::json!({"id":"t1","title":"task"})],
            )
            .unwrap()
            .remove(0);
        store.db().execute_batch("CREATE TRIGGER reject_scope BEFORE INSERT ON turn_scopes BEGIN SELECT RAISE(ABORT,'scope failure'); END;").unwrap();
        assert!(
            store
                .start_turn(
                    &conversation.id,
                    &format!("worker:{}", task.id),
                    "codex",
                    0,
                    Some(&task.plan_id)
                )
                .is_err()
        );
        let count: i64 = store
            .db()
            .query_row("SELECT count(*) FROM turns", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn restart_does_not_infer_missing_round_attribution() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("core.db");
        let store = Store::open(&path).unwrap();
        let conversation = store.create_conversation("project").unwrap();
        let mut task = store
            .add_plan(
                &conversation.id,
                &[serde_json::json!({"id":"t1","title":"task"})],
            )
            .unwrap()
            .remove(0);
        task.round = 7;
        task.status = "working".into();
        store.update_task(&task).unwrap();
        store
            .append_event(
                &conversation.id,
                None,
                Some(&task.plan_id),
                "task",
                &serde_json::json!({"task":task.id,"round":7,"status":"working"}),
            )
            .unwrap();
        let role = format!("worker:{}", task.id);
        let known = store
            .start_turn(&conversation.id, &role, "codex", 0, Some(&task.plan_id))
            .unwrap();
        store
            .append_event(
                &conversation.id,
                Some(&known.id),
                Some(&task.plan_id),
                "turn_started",
                &serde_json::json!({"role":role}),
            )
            .unwrap();
        store
            .append_event(
                &conversation.id,
                None,
                Some(&task.plan_id),
                "task",
                &serde_json::json!({"task":task.id,"round":7,"status":"approved"}),
            )
            .unwrap();
        let unknown = store
            .start_turn(&conversation.id, &role, "codex", 0, Some(&task.plan_id))
            .unwrap();
        store
            .append_event(
                &conversation.id,
                Some(&unknown.id),
                Some(&task.plan_id),
                "turn_started",
                &serde_json::json!({"role":role}),
            )
            .unwrap();
        task.round = 8;
        store.update_task(&task).unwrap();
        store
            .db()
            .execute_batch("DELETE FROM turn_scopes; DELETE FROM task_rounds WHERE round=7;")
            .unwrap();
        drop(store);
        let store = Store::open(&path).unwrap();
        assert!(store.task_round(&task.id, 7).is_err());
        let assigned: bool = store
            .db()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM turn_scopes WHERE turn_id=?1)",
                [unknown.id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!assigned);
    }
}

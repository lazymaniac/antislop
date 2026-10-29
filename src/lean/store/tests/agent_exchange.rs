use super::*;
use crate::lean::turn_result::TurnResult;
use serde_json::json;

fn fixture() -> (tempfile::TempDir, Store, String, Task, Turn) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("core.db")).unwrap();
    let conversation = store
        .create_conversation(dir.path().to_str().unwrap())
        .unwrap()
        .id;
    let mut task = store.add_plan(&conversation,&[json!({"id":"local","title":"Local change","goal":"Expose an API","kind":"implementation","outputs":["api"],"verifier":{"focus":"Inspect API"}})]).unwrap().remove(0);
    task.round = 1;
    task.status = "working".into();
    store.update_task(&task).unwrap();
    let turn = store
        .start_turn(
            &conversation,
            &format!("worker:{}", task.id),
            "fixture",
            0,
            Some(&task.plan_id),
        )
        .unwrap();
    (dir, store, conversation, task, turn)
}

#[test]
fn questions_are_durable_idempotent_and_scoped() {
    let (dir, store, conversation, _, turn) = fixture();
    let id = store
        .ask_agent_question(&conversation, &turn.id, "api", "Which error type?")
        .unwrap();
    assert_eq!(
        store
            .ask_agent_question(&conversation, &turn.id, "api", "Which error type?")
            .unwrap(),
        id
    );
    assert!(
        store
            .ask_agent_question(&conversation, &turn.id, "api", "Different question")
            .is_err()
    );
    assert!(store.agent_question("other", &id, None).is_err());
    assert!(
        store
            .agent_question(&conversation, &id, Some("other"))
            .is_err()
    );
    drop(store);
    let store = Store::open(&dir.path().join("core.db")).unwrap();
    assert_eq!(
        store
            .agent_question(&conversation, &id, Some(&turn.id))
            .unwrap()["state"],
        "pending"
    );
    store
        .answer_agent_question(&conversation, &id, "Use the existing error type")
        .unwrap();
    store
        .answer_agent_question(&conversation, &id, "Use the existing error type")
        .unwrap();
    assert!(
        store
            .answer_agent_question(&conversation, &id, "Use another type")
            .is_err()
    );
}

#[test]
fn a_turn_ending_without_waiting_closes_its_open_questions() {
    let (_dir, store, conversation, _, turn) = fixture();
    let answered = store
        .ask_agent_question(&conversation, &turn.id, "api", "Which error type?")
        .unwrap();
    store
        .answer_agent_question(&conversation, &answered, "The existing one")
        .unwrap();
    let open = store
        .ask_agent_question(&conversation, &turn.id, "scope", "Is the revision applied?")
        .unwrap();
    let deferred = store
        .ask_agent_question(&conversation, &turn.id, "user", "May I widen the scope?")
        .unwrap();
    store
        .defer_question_to_user(&conversation, &deferred, "Needs the user")
        .unwrap();

    assert_eq!(store.invalidate_open_questions(&turn.id).unwrap(), 2);
    let state = |id: &str| store.agent_question(&conversation, id, None).unwrap()["state"].clone();
    assert_eq!(state(&answered), "answered");
    assert_eq!(state(&open), "invalidated");
    assert_eq!(state(&deferred), "invalidated");
    let error = store
        .answer_agent_question(&conversation, &open, "Yes")
        .unwrap_err();
    assert!(error.to_string().contains("invalidated"), "{error}");
}

#[test]
fn answering_a_checkpoint_resumes_only_its_owner() {
    let (_dir, store, conversation, mut task, turn) = fixture();
    let id = store
        .ask_agent_question(&conversation, &turn.id, "api", "Which error type?")
        .unwrap();
    task.status = "waiting".into();
    task.feedback = Some(json!({"round":task.round,"waiting_request":id}));
    store.update_task(&task).unwrap();
    store
        .answer_agent_question(&conversation, &id, "Use the existing type")
        .unwrap();
    let resumed = store.task_by_id(&task.id).unwrap().unwrap();
    assert_eq!(resumed.status, "queued");
    assert!(resumed.verdict.is_none());
}

#[test]
fn a_submission_is_recorded_once_before_acknowledgement() {
    let (_dir, store, _, task, turn) = fixture();
    let result = TurnResult::Completed {
        summary: "API defined".into(),
        report: "Result".into(),
        selection: None,
    };
    store.save_turn_result(&turn.id, &result).unwrap();
    store.save_turn_result(&turn.id, &result).unwrap();
    assert!(
        store
            .save_turn_result(
                &turn.id,
                &TurnResult::Report {
                    text: "changed".into(),
                    addressed: None,
                }
            )
            .is_err()
    );
    assert_eq!(
        store.completed_worker_report(&task.id, 1).unwrap(),
        Some((1, "Result".into()))
    );
    let count: i64 = store
        .db()
        .query_row("SELECT count(*) FROM agent_results", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn a_dependent_receives_the_summary_of_its_dependency_not_its_report() {
    let (_dir, store, conversation, mut producer, turn) = fixture();
    store
        .save_turn_result(
            &turn.id,
            &TurnResult::Completed {
                summary: "Defined Result<T, Error> in src/api.rs".into(),
                report: "PRIVATE REPORT".into(),
                selection: None,
            },
        )
        .unwrap();
    producer.status = "approved".into();
    store.update_task(&producer).unwrap();
    let spec: crate::lean::tasks::TaskContract = serde_json::from_value(json!({"id":"consumer","title":"Use API","goal":"Use API","kind":"implementation","depends_on":["local"]})).unwrap();
    let inputs = store.dependency_inputs(&producer.plan_id, &spec).unwrap();
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].id, "local");
    assert_eq!(inputs[0].report, "Defined Result<T, Error> in src/api.rs");
    assert!(!inputs[0].report.contains("PRIVATE"));
    assert_eq!(
        store
            .messages_by(&conversation, "runtime", 0)
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn a_dependency_without_a_summary_hands_over_the_opening_line_of_its_report() {
    let (_dir, store, _conversation, mut producer, _turn) = fixture();
    producer.status = "approved".into();
    producer.report = Some("\nDefined the API\n\nPRIVATE REPORT".into());
    store.update_task(&producer).unwrap();
    let spec: crate::lean::tasks::TaskContract = serde_json::from_value(json!({"id":"consumer","title":"Use API","goal":"Use API","kind":"implementation","depends_on":["local"]})).unwrap();
    let inputs = store.dependency_inputs(&producer.plan_id, &spec).unwrap();
    assert_eq!(inputs[0].report, "Defined the API");
}

#[test]
fn checkpoint_and_reply_are_safe_in_either_order() {
    for answer_first in [true, false] {
        let (_dir, store, conversation, mut task, turn) = fixture();
        let request = store
            .ask_agent_question(&conversation, &turn.id, "api", "Which API?")
            .unwrap();
        task.feedback = Some(json!({"waiting_request":request,"resume_role":"verifier"}));
        store.update_task(&task).unwrap();
        if answer_first {
            store
                .answer_agent_question(&conversation, &request, "Existing API")
                .unwrap();
        }
        let state = store.park_task_for_question(&task, &request).unwrap();
        assert_eq!(state, if answer_first { "queued" } else { "waiting" });
        if !answer_first {
            store
                .answer_agent_question(&conversation, &request, "Existing API")
                .unwrap();
        }
        let resumed = store.task_by_id(&task.id).unwrap().unwrap();
        assert_eq!(resumed.status, "queued");
        assert_eq!(resumed.feedback.unwrap()["resume_role"], "verifier");
    }
}

#[test]
fn awaiting_user_preserves_checkpoint_until_an_answer_arrives() {
    let (_dir, store, conversation, mut task, turn) = fixture();
    let request = store
        .ask_agent_question(&conversation, &turn.id, "scope", "May scope expand?")
        .unwrap();
    store
        .defer_question_to_user(&conversation, &request, "User approval is required")
        .unwrap();
    task.feedback = Some(json!({"waiting_request":request}));
    store.update_task(&task).unwrap();
    assert_eq!(
        store.park_task_for_question(&task, &request).unwrap(),
        "waiting"
    );
    assert_eq!(
        store
            .agent_question(&conversation, &request, Some(&turn.id))
            .unwrap()["state"],
        "awaiting_user"
    );
    store
        .answer_agent_question(&conversation, &request, "Keep the accepted scope")
        .unwrap();
    assert_eq!(
        store.task_by_id(&task.id).unwrap().unwrap().status,
        "queued"
    );
}

#[test]
fn acknowledged_worker_result_is_reused_after_interruption() {
    let (_dir, store, _, task, turn) = fixture();
    store
        .save_turn_result(
            &turn.id,
            &TurnResult::Completed {
                summary: "Done".into(),
                report: "API defined".into(),
                selection: None,
            },
        )
        .unwrap();
    store.interrupt_unfinished_tasks().unwrap();
    let recovered = store.task_by_id(&task.id).unwrap().unwrap();
    assert_eq!(recovered.status, "queued");
    assert_eq!(
        recovered.feedback.as_ref().unwrap()["resume_role"],
        "verifier"
    );
    assert_eq!(
        store.completed_task_output(&recovered).unwrap()["report"],
        "API defined"
    );
}

#[test]
fn input_changes_invalidate_questions_without_leaking_them_into_the_new_turn() {
    let (_dir, store, conversation, task, turn) = fixture();
    let run = store.create_run(&conversation, "Local inputs").unwrap();
    store
        .db()
        .execute(
            "INSERT INTO task_inputs(task_id,run_id) VALUES(?1,?2)",
            params![task.id, run.id],
        )
        .unwrap();
    let request = store
        .ask_agent_question(&conversation, &turn.id, "api", "Which API?")
        .unwrap();
    store
        .defer_question_to_user(&conversation, &request, "Awaiting choice")
        .unwrap();
    store
        .db()
        .execute(
            "UPDATE task_inputs SET generation=generation+1 WHERE task_id=?1",
            [&task.id],
        )
        .unwrap();
    assert_eq!(
        store
            .agent_question(&conversation, &request, Some(&turn.id))
            .unwrap()["state"],
        "invalidated"
    );
    assert!(
        store
            .answer_agent_question(&conversation, &request, "Old answer")
            .is_err()
    );
    let next = store
        .start_turn(
            &conversation,
            &format!("worker:{}", task.id),
            "fixture",
            0,
            Some(&task.plan_id),
        )
        .unwrap();
    assert!(store.local_questions(&next.id).unwrap().is_empty());
    assert!(
        store
            .agent_question(&conversation, &request, Some(&next.id))
            .is_err()
    );
}

#[test]
fn unknown_question_ids_are_actionable_and_cannot_cross_conversations() {
    let (_dir, store, conversation, _, turn) = fixture();
    let text = "Please provide the reviewed delivery revision";
    let error = store
        .agent_question(&conversation, text, Some(&turn.id))
        .unwrap_err()
        .to_string();
    assert!(error.contains("request ID the agent's question message names"));
    let error = store
        .answer_agent_question(&conversation, text, "Answer")
        .unwrap_err()
        .to_string();
    assert!(error.contains("unknown question ID"));
    let id = store
        .ask_agent_question(&conversation, &turn.id, "revision", "Which revision?")
        .unwrap();
    assert!(
        store
            .answer_agent_question("other", &id, "Answer")
            .unwrap_err()
            .to_string()
            .contains("unknown question ID")
    );
    assert_eq!(
        store
            .agent_question(&conversation, &id, Some(&turn.id))
            .unwrap()["state"],
        "pending"
    );
    store
        .db()
        .execute(
            "UPDATE agent_questions SET state='invalidated' WHERE id=?1",
            [&id],
        )
        .unwrap();
    assert!(
        store
            .answer_agent_question(&conversation, &id, "Answer")
            .unwrap_err()
            .to_string()
            .contains("invalidated")
    );
}

use super::*;

#[test]
fn completed_followup_restarts_interview_without_erasing_history() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("state.db")).unwrap();
    let conversation = store.create_conversation("project").unwrap();
    let first = store
        .add_message(&conversation.id, "user", "First goal")
        .unwrap();
    store
        .interview_summary(&conversation.id, "First scope", first.seq)
        .unwrap();
    store.finish_interview(&conversation.id).unwrap();
    let run = store.create_run(&conversation.id, "First goal").unwrap();
    store
        .select_planning_run(&conversation.id, &run.id)
        .unwrap();
    let tasks = store
        .add_plan(
            &conversation.id,
            &[serde_json::json!({"id":"implementation","title":"Implementation","goal":"Deliver behavior","kind":"implementation","verifier":{"focus":"Inspect behavior"}})],
        )
        .unwrap();
    let plan = &tasks[0].plan_id;
    store.queue_plan(&conversation.id, plan).unwrap();
    // A message while work remains must not reset its workflow.
    store
        .add_message(&conversation.id, "user", "More context")
        .unwrap();
    assert_eq!(
        store.workflow(&conversation.id).unwrap().phase,
        "implementation"
    );
    store.set_plan_status(plan, "queued", "approved").unwrap();
    store
        .add_message(&conversation.id, "runtime", "Finished")
        .unwrap();
    assert_eq!(
        store.workflow(&conversation.id).unwrap().phase,
        "implementation"
    );
    store
        .add_message(&conversation.id, "user", "Next goal")
        .unwrap();
    let workflow = store.workflow(&conversation.id).unwrap();
    assert_eq!(workflow.phase, "interview");
    assert!(workflow.summary.is_empty());
    assert_eq!(workflow.upto_message, 0);
    let next_run = store.planning_run(&conversation.id).unwrap().unwrap();
    assert_ne!(next_run.id, run.id);
    assert_eq!(next_run.goal, "Next goal");
    assert!(store.run(&conversation.id, &run.id).is_ok());
    assert_eq!(store.plan(plan).unwrap()[0].status, "approved");
    store
        .add_message(&conversation.id, "user", "Next constraint")
        .unwrap();
    assert_eq!(store.workflow(&conversation.id).unwrap().phase, "interview");
}

#[test]
fn acceptance_requires_current_summary_and_survives_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let store = Store::open(&path).unwrap();
    let conversation = store.create_conversation("project").unwrap();
    assert!(store.approve_requirements(&conversation.id).is_err());
    let first = store
        .add_message(&conversation.id, "user", "Goal 👩‍💻")
        .unwrap();
    store
        .interview_summary(&conversation.id, "Scope and acceptance criteria", first.seq)
        .unwrap();
    let next = store
        .add_message(&conversation.id, "user", "Also preserve drafts")
        .unwrap();
    assert!(store.approve_requirements(&conversation.id).is_err());
    store
        .interview_summary(
            &conversation.id,
            "Scope, draft preservation, criteria",
            next.seq,
        )
        .unwrap();
    let accepted = store.approve_requirements(&conversation.id).unwrap();
    assert!(
        accepted
            .text
            .contains("Scope, draft preservation, criteria")
    );
    assert!(
        store
            .messages_by(&conversation.id, "user", 0)
            .unwrap()
            .last()
            .unwrap()
            .text
            .contains("Scope, draft preservation, criteria")
    );
    assert!(store.approve_requirements(&conversation.id).is_err());
    drop(store);
    let store = Store::open(&path).unwrap();
    let workflow = store.workflow(&conversation.id).unwrap();
    assert_eq!(workflow.phase, "planning");
    assert!(workflow.summary.contains("draft preservation"));
    let events = store.events_since(&conversation.id, 0, 100).unwrap();
    assert_eq!(events.last().unwrap().payload["phase"], "planning");
}

#[test]
fn automatic_planning_is_fresh_idempotent_and_does_not_forge_user_approval() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("state.db")).unwrap();
    let conversation = store.create_conversation("project").unwrap();
    assert!(!store.finish_interview(&conversation.id).unwrap());
    let first = store
        .add_message(&conversation.id, "user", "Inspect code")
        .unwrap();
    store
        .interview_summary(&conversation.id, "Inspect source", first.seq)
        .unwrap();
    let newer = store
        .add_message(&conversation.id, "user", "Also docs")
        .unwrap();
    assert!(!store.finish_interview(&conversation.id).unwrap());
    store
        .interview_summary(&conversation.id, "Inspect source and docs", newer.seq)
        .unwrap();
    assert!(store.finish_interview(&conversation.id).unwrap());
    assert!(!store.finish_interview(&conversation.id).unwrap());
    assert_eq!(store.workflow(&conversation.id).unwrap().phase, "planning");
    assert_eq!(
        store
            .messages_by(&conversation.id, "user", 0)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        store
            .messages_by(&conversation.id, "runtime", 0)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn every_conversation_begins_in_its_interview() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("state.db")).unwrap();
    let conversation = store.create_conversation("project").unwrap();
    let workflow = store.workflow(&conversation.id).unwrap();
    assert!(workflow.interviewing() && workflow.summary.is_empty());
    assert!(store.workflow("missing").is_err());
}

#[test]
fn malformed_requirements_retry_is_idempotent_and_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("state.db")).unwrap();
    let conversation = store.create_conversation("project").unwrap();
    store
        .add_message(&conversation.id, "user", "Keep the same requirements")
        .unwrap();
    for attempt in 0..3 {
        store
            .append_event(
                &conversation.id,
                None,
                None,
                "requirements_failed",
                &serde_json::json!({"error":"the requirements analyst ended its turn without submit_ledger"}),
            )
            .unwrap();
        assert_eq!(
            store
                .queue_requirements_format_repair(&conversation.id)
                .unwrap(),
            attempt < 2
        );
        assert!(
            !store
                .queue_requirements_format_repair(&conversation.id)
                .unwrap()
        );
    }
    assert_eq!(
        store
            .messages_by(&conversation.id, "requirements-format-repair", 0)
            .unwrap()
            .len(),
        2
    );
    store
        .add_message(&conversation.id, "user", "Retry preparation")
        .unwrap();
    store
        .append_event(
            &conversation.id,
            None,
            None,
            "requirements_failed",
            &serde_json::json!({"error":"requirements proposal awaits user approval"}),
        )
        .unwrap();
    // An approval blocker must not be treated as a malformed response.
    assert!(
        !store
            .queue_requirements_format_repair(&conversation.id)
            .unwrap()
    );
}

#[test]
fn execution_decisions_are_durable_idempotent_and_do_not_change_worker_questions() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("state.db")).unwrap();
    let conversation = store.create_conversation("project").unwrap();
    let turn = store
        .start_turn(&conversation.id, "orchestrator", "fixture", 0, None)
        .unwrap();
    let request = store
        .ask_agent_question(&conversation.id, &turn.id, "old", "Old question?")
        .unwrap();
    store
        .answer_agent_question(&conversation.id, &request, "Answered")
        .unwrap();
    let before = store.workflow(&conversation.id).unwrap();
    let question = serde_json::from_value(serde_json::json!({"id":"metadata-scope","context":"Read-only metadata access needs a scope decision.","question":"Allow this exact metadata scope?","options":[{"label":"Allow"},{"label":"Reject"}]})).unwrap();
    store
        .ask_user_decision(&conversation.id, &question)
        .unwrap();
    store
        .ask_user_decision(&conversation.id, &question)
        .unwrap();
    drop(store);
    let store = Store::open(&dir.path().join("state.db")).unwrap();
    let events = store.events_since(&conversation.id, 0, 100).unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == "user_decision_requested")
            .count(),
        1
    );
    assert_eq!(
        store.workflow(&conversation.id).unwrap().phase,
        before.phase
    );
    assert_eq!(
        store
            .agent_question(&conversation.id, &request, None)
            .unwrap()["state"],
        "answered"
    );
    store.add_message(&conversation.id, "user", "Interview answer (metadata-scope)\nQuestion: Allow this exact metadata scope?\nAnswer: Reject").unwrap();
    assert_eq!(
        store
            .agent_question(&conversation.id, &request, None)
            .unwrap()["answer"],
        "Answered"
    );
    let mut changed = question.clone();
    changed.question = "Different decision?".into();
    assert!(
        store
            .ask_user_decision(&conversation.id, &changed)
            .unwrap_err()
            .to_string()
            .contains("already used")
    );
    let mut invalid = question;
    invalid.options.clear();
    assert!(store.ask_user_decision(&conversation.id, &invalid).is_err());
}

#[test]
fn every_planning_cycle_awaits_its_own_first_plan() {
    let pause = || std::thread::sleep(std::time::Duration::from_millis(3));
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("state.db")).unwrap();
    let conversation = store.create_conversation("project").unwrap();
    let task = serde_json::json!({"id":"t1","title":"T","goal":"g","kind":"analysis","verifier":{"focus":"f"}});
    let settle = |request: &str| {
        let message = store
            .add_message(&conversation.id, "user", request)
            .unwrap();
        store
            .interview_summary(&conversation.id, request, message.seq)
            .unwrap();
        assert!(store.finish_interview(&conversation.id).unwrap());
    };
    assert!(!store.awaiting_first_plan(&conversation.id).unwrap());
    settle("first request");
    assert!(store.awaiting_first_plan(&conversation.id).unwrap());
    pause();
    let mut plan = store
        .add_plan(&conversation.id, std::slice::from_ref(&task))
        .unwrap()
        .remove(0);
    assert!(!store.awaiting_first_plan(&conversation.id).unwrap());

    // The plan ran to the end; the next request interviews again, and its
    // planning cycle is not satisfied by the previous plan.
    store.queue_plan(&conversation.id, &plan.plan_id).unwrap();
    plan.status = "approved".into();
    store.update_task(&plan).unwrap();
    pause();
    store
        .add_message(&conversation.id, "user", "second request")
        .unwrap();
    assert!(store.workflow(&conversation.id).unwrap().interviewing());
    assert!(!store.awaiting_first_plan(&conversation.id).unwrap());
    pause();
    settle("second request, agreed");
    assert!(store.awaiting_first_plan(&conversation.id).unwrap());
    pause();
    store.add_plan(&conversation.id, &[task]).unwrap();
    assert!(!store.awaiting_first_plan(&conversation.id).unwrap());
}

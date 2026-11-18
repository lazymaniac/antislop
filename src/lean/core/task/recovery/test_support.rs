use super::*;

pub(super) fn recovery_fixture() -> (tempfile::TempDir, Arc<Core>, String, Task, store::Turn) {
    let directory = tempfile::tempdir().unwrap();
    let config = Config::from_toml(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/examples/antislop.toml"
    )))
    .unwrap();
    let core = Core::open(&directory.path().join("state"), config).unwrap();
    let conversation = core
        .store
        .create_conversation(directory.path().to_str().unwrap())
        .unwrap()
        .id;
    let message = core
        .store
        .add_message(&conversation, "user", "Validate results")
        .unwrap();
    core.store
        .interview_summary(&conversation, &message.text, message.seq)
        .unwrap();
    core.store.confirm_scope_for_test(&conversation);
    let run = core
        .store
        .create_scope(&conversation, &message.text)
        .unwrap();
    let spec = json!({"id":"integration-gate","title":"Integration","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Validate","change_kind": "none", "kind":"analysis",});
    let mut accepted = core
        .store
        .add_plan(&conversation, &[spec])
        .unwrap()
        .remove(0);
    core.store
        .bind_plan_scope(&conversation, &accepted.plan_id, &run.id)
        .unwrap();
    accepted.status = status::FAILED.into();
    core.store.update_task(&accepted).unwrap();
    let authorize = |plan: &str| {
        core.store
            .db()
            .execute(
                "INSERT INTO plan_activations(plan_id,conversation_id) VALUES(?1,?2)",
                rusqlite::params![plan, conversation],
            )
            .unwrap();
    };
    authorize(&accepted.plan_id);
    let upto = core
        .store
        .messages_after(&conversation, 0)
        .unwrap()
        .last()
        .unwrap()
        .seq;
    let turn = core
        .store
        .start_turn(
            &conversation,
            ORCHESTRATOR,
            "fixture",
            upto,
            Some(&accepted.plan_id),
        )
        .unwrap();
    core.pause(&conversation, true).unwrap();
    core.activity
        .running
        .lock()
        .unwrap()
        .insert(conversation.clone(), Arc::new(AtomicBool::new(false)));
    core.activity
        .plans
        .lock()
        .unwrap()
        .insert(conversation.clone(), Arc::new(AtomicBool::new(false)));
    (directory, core, conversation, accepted, turn)
}

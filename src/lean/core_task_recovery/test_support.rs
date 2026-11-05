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
    assert!(core.store.finish_interview(&conversation).unwrap());
    let run = core.store.create_run(&conversation, &message.text).unwrap();
    let source = core
        .store
        .register_message_source(&conversation, &run.id, message.seq)
        .unwrap();
    let change = serde_json::from_value(json!({"conversation":conversation,"run_id":run.id,"expected_spec_revision":0,"operation_id":"recovery-fixture","reason":"Synthetic recovery scope","entries":[{"id":"R1","source":{"source_id":source.id,"start":0,"end":message.text.len()},"interpretation":message.text,"criteria":[{"id":"R1.accept","text":"Validate results without changing the contract"}]}]})).unwrap();
    core.store
        .apply_ledger_change(&change, &core.state, true)
        .unwrap();
    let spec = json!({"id":"integration-gate","title":"Integration","goal":"Validate","kind":"analysis","scope":["results"],"requirement_ids":["R1"],"criterion_ids":["R1.accept"]});
    let mut accepted = core
        .store
        .add_plan(&conversation, &[spec])
        .unwrap()
        .remove(0);
    core.store
        .link_plan_run(&conversation, &accepted.plan_id, &run.id)
        .unwrap();
    core.store
        .bind_task_ledger(&conversation, &run.id, &accepted.plan_id)
        .unwrap();
    accepted.status = status::FAILED.into();
    core.store.update_task(&accepted).unwrap();
    let authorize = |plan: &str| {
        core.store.db().execute("INSERT INTO plan_authorizations(plan_id,conversation_id,scope,write_scope) VALUES(?1,?2,'[\"results\"]','[]')",rusqlite::params![plan,conversation]).unwrap();
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
    core.running
        .lock()
        .unwrap()
        .insert(conversation.clone(), Arc::new(AtomicBool::new(false)));
    core.plans
        .lock()
        .unwrap()
        .insert(conversation.clone(), Arc::new(AtomicBool::new(false)));
    (directory, core, conversation, accepted, turn)
}

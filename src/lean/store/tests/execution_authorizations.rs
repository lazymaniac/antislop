use super::*;
use crate::lean::ledger::{LedgerChange, RuntimeConstraint};
use crate::lean::tasks::TaskContract;
use serde_json::json;

fn fixture() -> (tempfile::TempDir, Store, ExecutionAuthorization, Vec<Task>) {
    let (dir, store, conversation, run, tasks) = super::plan_revision::fixture();
    let mut entry = store
        .ledger_entries(&conversation, &run, None)
        .unwrap()
        .remove(0)
        .spec;
    entry.constraints = vec![RuntimeConstraint::NoNewExecutions];
    store
        .apply_ledger_change(
            &LedgerChange {
                conversation: conversation.clone(),
                run_id: run,
                expected_spec_revision: 1,
                operation_id: crate::id(),
                reason: "Hold execution pending explicit decision".into(),
                entries: vec![entry],
            },
            dir.path(),
            true,
        )
        .unwrap();
    let plan = tasks[0].plan_id.clone();
    store.db().execute("INSERT INTO plan_authorizations(plan_id,conversation_id,scope,write_scope) VALUES(?1,?2,'[]','[]')",params![plan,conversation]).unwrap();
    let tasks = store.plan(&plan).unwrap();
    let mut contract: TaskContract = serde_json::from_value(tasks[0].spec.clone()).unwrap();
    contract.writes = true;
    contract.change_kind = Some(crate::lean::tasks::ChangeKind::Code);
    contract.goal = "Only the explicitly bounded instrumentation repair".into();
    let grant = ExecutionAuthorization {
        conversation,
        plan: plan.clone(),
        base_revision: store.plan_revision(&plan).unwrap(),
        decision: "bounded-repair".into(),
        selected_option: "Allow bounded repair".into(),
        requirement: "r".into(),
        contract,
    };
    store
        .ask_user_decision(
            &grant.conversation,
            &crate::lean::interview::Question {
                id: grant.decision.clone(),
                context: "One build; four invocations; preserved runtime state".into(),
                question: "Allow bounded repair?".into(),
                options: vec![
                    crate::lean::interview::Choice {
                        label: grant.selected_option.clone(),
                        description: String::new(),
                    },
                    crate::lean::interview::Choice {
                        label: "Keep execution held".into(),
                        description: String::new(),
                    },
                ],
            },
        )
        .unwrap();
    (dir, store, grant, tasks)
}

fn answer(store: &Store, grant: &ExecutionAuthorization) {
    store
        .add_message(
            &grant.conversation,
            "user",
            &format!(
                "Interview answer ({})\nQuestion: Allow bounded repair?\nAnswer: {}",
                grant.decision, grant.selected_option
            ),
        )
        .unwrap();
}

#[test]
fn explicit_execution_grant_is_scoped_persistent_and_cannot_be_replayed_for_other_work() {
    let (dir, store, grant, tasks) = fixture();
    assert!(
        store
            .authorize_task_execution(&grant)
            .unwrap_err()
            .to_string()
            .contains("no user answer")
    );
    answer(&store, &grant);
    let entries = store
        .ledger_entries(
            &grant.conversation,
            &store.plan_run(&grant.plan).unwrap().unwrap(),
            None,
        )
        .unwrap();
    let check = |store: &Store, specs: &[TaskContract], baseline: &[Task]| {
        store.validate_authorized_plan_constraints(
            &grant.conversation,
            &grant.plan,
            specs,
            baseline,
            &[],
            &entries,
        )
    };
    assert!(check(&store, std::slice::from_ref(&grant.contract), &tasks).is_err());
    store.authorize_task_execution(&grant).unwrap();
    store.authorize_task_execution(&grant).unwrap(); // idempotent, one immutable row/event
    assert!(check(&store, std::slice::from_ref(&grant.contract), &tasks).is_ok());
    let count: i64 = store
        .db()
        .query_row(
            "SELECT count(*) FROM events WHERE kind='task_execution_authorized'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    let mut expanded = grant.clone();
    expanded
        .contract
        .goal
        .push_str(" and run the full baseline");
    assert!(store.authorize_task_execution(&expanded).is_err());
    assert!(check(&store, std::slice::from_ref(&expanded.contract), &tasks).is_err());
    let mut neighbor = grant.contract.clone();
    neighbor.id = "b".into();
    assert!(check(&store, &[grant.contract.clone(), neighbor], &tasks).is_err());
    let mut stronger = entries.clone();
    stronger[0]
        .spec
        .constraints
        .push(RuntimeConstraint::NoFileChanges);
    assert!(
        store
            .validate_authorized_plan_constraints(
                &grant.conversation,
                &grant.plan,
                std::slice::from_ref(&grant.contract),
                &tasks,
                &[],
                &stronger
            )
            .is_err()
    );
    let mut newer = entries.clone();
    newer[0].version += 1;
    assert!(
        store
            .validate_authorized_plan_constraints(
                &grant.conversation,
                &grant.plan,
                std::slice::from_ref(&grant.contract),
                &tasks,
                &[],
                &newer
            )
            .is_err()
    );
    let generation = tasks[0].input_generation + 1;
    let before = store
        .authorized_task_constraints(
            &grant.conversation,
            &tasks[0],
            &grant.contract,
            generation - 1,
            &entries,
        )
        .unwrap();
    assert!(
        before[0]
            .spec
            .constraints
            .contains(&RuntimeConstraint::NoNewExecutions)
    );
    let after = store
        .authorized_task_constraints(
            &grant.conversation,
            &tasks[0],
            &grant.contract,
            generation,
            &entries,
        )
        .unwrap();
    assert!(
        !after[0]
            .spec
            .constraints
            .contains(&RuntimeConstraint::NoNewExecutions)
    );
    let redo = store
        .authorized_task_constraints(
            &grant.conversation,
            &tasks[0],
            &grant.contract,
            generation + 1,
            &entries,
        )
        .unwrap();
    assert!(
        redo[0]
            .spec
            .constraints
            .contains(&RuntimeConstraint::NoNewExecutions)
    );
    // An independently reviewed graph may now apply; the task remains unapproved.
    let mut current = tasks[0].clone();
    current.spec = serde_json::to_value(&grant.contract).unwrap();
    current.input_generation = generation;
    store
        .db()
        .execute(
            "UPDATE task_inputs SET generation=?2 WHERE task_id=?1",
            params![current.id, generation],
        )
        .unwrap();
    store
        .db()
        .execute(
            "UPDATE tasks SET spec=?2 WHERE id=?1",
            params![current.id, serde_json::to_string(&grant.contract).unwrap()],
        )
        .unwrap();
    store.update_task(&current).unwrap();
    assert_eq!(
        store.task_by_id(&current.id).unwrap().unwrap().spec,
        current.spec
    );
    store.validate_task_execution_constraints(&current).unwrap();
    let path = dir.path().join("core.db");
    drop(store);
    let reopened = Store::open(&path).unwrap();
    reopened
        .validate_task_execution_constraints(&reopened.task_by_id(&current.id).unwrap().unwrap())
        .unwrap();
    assert_ne!(
        reopened.task_by_id(&current.id).unwrap().unwrap().status,
        "approved"
    );
    assert!(
        reopened
            .ledger_entries(
                &grant.conversation,
                &reopened.plan_run(&grant.plan).unwrap().unwrap(),
                None
            )
            .unwrap()[0]
            .spec
            .constraints
            .contains(&RuntimeConstraint::NoNewExecutions)
    );
}

#[test]
fn execution_grant_rejects_mismatched_answers_conversations_and_stale_plan_heads() {
    let (_dir, store, grant, _tasks) = fixture();
    store.add_message(&grant.conversation,"user","Interview answer (bounded-repair)\nQuestion: Allow bounded repair?\nAnswer: Keep execution held").unwrap();
    assert!(store.authorize_task_execution(&grant).is_err());
    answer(&store, &grant);
    let mut stale = grant.clone();
    stale.base_revision += 1;
    assert!(store.authorize_task_execution(&stale).is_err());
    let mut other = grant.clone();
    other.conversation = store.create_conversation("other").unwrap().id;
    assert!(store.authorize_task_execution(&other).is_err());
    let mut held = grant.clone();
    held.contract.deferred = true;
    assert!(store.authorize_task_execution(&held).is_err());
    let mut changed = grant.clone();
    changed.selected_option = "Unknown option".into();
    assert!(store.authorize_task_execution(&changed).is_err());
    let encoded = json!({"op":"authorize_task_execution","authorization":grant});
    let request: crate::lean::daemon::Request = serde_json::from_value(encoded.clone()).unwrap();
    assert_eq!(serde_json::to_value(request).unwrap(), encoded);
}

#[test]
fn planned_inputs_can_refresh_only_before_the_first_authorized_writer_turn() {
    let (_dir, store, grant, tasks) = fixture();
    answer(&store, &grant);
    store.authorize_task_execution(&grant).unwrap();
    let mut task = tasks[0].clone();
    task.spec = serde_json::to_value(&grant.contract).unwrap();
    task.input_generation += 2; // options completed after the reviewed graph update
    store
        .db()
        .execute(
            "UPDATE task_inputs SET generation=?2 WHERE task_id=?1",
            params![task.id, task.input_generation],
        )
        .unwrap();
    store
        .db()
        .execute(
            "UPDATE tasks SET spec=?2 WHERE id=?1",
            params![task.id, serde_json::to_string(&grant.contract).unwrap()],
        )
        .unwrap();
    store.update_task(&task).unwrap();
    assert_eq!(store.task_by_id(&task.id).unwrap().unwrap().spec, task.spec);
    store.validate_task_execution_constraints(&task).unwrap();
    let sealed: i64 = store
        .db()
        .query_row(
            "SELECT generation FROM execution_authorizations WHERE decision_id=?1",
            [&grant.decision],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(sealed, task.input_generation);
    let role = format!("worker:{}", task.id);
    let turn = store
        .start_turn(&grant.conversation, &role, "fixture", 0, Some(&grant.plan))
        .unwrap();
    store.finish_turn(&turn.id, TurnStatus::Done, None).unwrap();
    store.validate_task_execution_constraints(&task).unwrap(); // same-input continuation
    task.input_generation += 1;
    store
        .db()
        .execute(
            "UPDATE task_inputs SET generation=?2 WHERE task_id=?1",
            params![task.id, task.input_generation],
        )
        .unwrap();
    store.update_task(&task).unwrap();
    assert!(
        store.validate_task_execution_constraints(&task).is_err(),
        "a started allowance cannot be renewed by refreshing inputs"
    );
}

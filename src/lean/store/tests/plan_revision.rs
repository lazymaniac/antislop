use super::*;
use crate::lean::ledger::LedgerChange;
use crate::lean::store::RevisionLease;
use crate::lean::tasks::TaskContract;
use serde_json::json;
use std::collections::BTreeSet;

fn fixture() -> (tempfile::TempDir, Store, String, String, Vec<Task>) {
    fixture_for(false)
}

fn fixture_for(dependent: bool) -> (tempfile::TempDir, Store, String, String, Vec<Task>) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("core.db")).unwrap();
    let conversation = store
        .create_conversation(dir.path().to_str().unwrap())
        .unwrap()
        .id;
    let message = store
        .add_message(&conversation, "user", "Deliver local behavior")
        .unwrap();
    let run = store
        .create_run(&conversation, "Deliver local behavior")
        .unwrap();
    let source = store
        .register_message_source(&conversation, &run.id, message.seq)
        .unwrap();
    let entry=serde_json::from_value(json!({"id":"r","source":{"source_id":source.id,"start":0,"end":22},"interpretation":"Deliver local behavior","criteria":[{"id":"r.accept","text":"Behavior is checked","mandatory":true}]})).unwrap();
    store
        .apply_ledger_change(
            &LedgerChange {
                conversation: conversation.clone(),
                run_id: run.id.clone(),
                expected_spec_revision: 0,
                operation_id: crate::id(),
                reason: "Accepted scope".into(),
                entries: vec![entry],
            },
            dir.path(),
            true,
        )
        .unwrap();
    let tasks=store.add_plan(&conversation,&[json!({"id":"a","title":"A","goal":"Inspect A","kind":"analysis","requirement_ids":["r"],"criterion_ids":["r.accept"],"verifier":{"focus":"Check A"}}),json!({"id":"b","title":"B","goal":"Inspect B","kind":"analysis","depends_on":if dependent {vec!["a"]} else {vec![]},"requirement_ids":["r"],"verifier":{"focus":"Check B"}})]).unwrap();
    store
        .link_plan_run(&conversation, &tasks[0].plan_id, &run.id)
        .unwrap();
    store
        .bind_task_ledger(&conversation, &run.id, &tasks[0].plan_id)
        .unwrap();
    (dir, store, conversation, run.id, tasks)
}

#[test]
fn revision_commits_contracts_generations_events_and_receipt_together() {
    let (dir, store, conversation, _, mut baseline) = fixture();
    baseline[1].status = "approved".into();
    store.update_task(&baseline[1]).unwrap();
    let mut specs: Vec<TaskContract> = baseline
        .iter()
        .map(|t| serde_json::from_value(t.spec.clone()).unwrap())
        .collect();
    specs[0].goal = "Inspect revised A".into();
    let revision = PlanRevision {
        plan: baseline[0].plan_id.clone(),
        base_revision: 0,
        reason: "Clarify local contract".into(),
        tasks: vec![specs[0].clone()],
        retire: vec![],
    };
    let (request, fresh) = store
        .reserve_revision_job(&conversation, "rev1", &revision)
        .unwrap();
    assert!(fresh);
    let affected = BTreeSet::from([baseline[0].id.clone()]);
    let lease = RevisionLease {
        requirement_revision: 1,
        request: &request,
    };
    store
        .apply_plan_revision(
            &conversation,
            &revision,
            &baseline,
            &specs,
            &affected,
            &lease,
        )
        .unwrap();
    let current = store.plan(&revision.plan).unwrap();
    assert_eq!(
        current[0].input_generation,
        baseline[0].input_generation + 1
    );
    assert_eq!(current[1].input_generation, baseline[1].input_generation);
    assert_eq!(current[1].status, "approved");
    assert_eq!(store.plan_revision(&revision.plan).unwrap(), 1);
    assert_eq!(store.revision_job(&request).unwrap()["state"], "applied");
    assert!(
        !store
            .reserve_revision_job(&conversation, "rev1", &revision)
            .unwrap()
            .1
    );
    assert!(
        store
            .apply_plan_revision(
                &conversation,
                &revision,
                &baseline,
                &specs,
                &affected,
                &lease
            )
            .is_err()
    );
    drop(store);
    let store = Store::open(&dir.path().join("core.db")).unwrap();
    store.interrupt_revision_jobs().unwrap();
    assert_eq!(store.revision_job(&request).unwrap()["state"], "applied");
    let versions: i64 = store
        .db()
        .query_row(
            "SELECT count(*) FROM task_contracts WHERE task_id=?1",
            [&baseline[0].id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(versions, 2);
    assert!(
        store
            .events_since(&conversation, 0, 1000)
            .unwrap()
            .iter()
            .any(|e| e.kind == "graph_updated")
    );
}

#[test]
fn changed_requirements_rollback_revision_and_restart_preserves_proposal() {
    let (_dir, store, conversation, run, baseline) = fixture();
    let specs: Vec<TaskContract> = baseline
        .iter()
        .map(|t| serde_json::from_value(t.spec.clone()).unwrap())
        .collect();
    let revision = PlanRevision {
        plan: baseline[0].plan_id.clone(),
        base_revision: 0,
        reason: "Review".into(),
        tasks: vec![],
        retire: vec![],
    };
    let (request, _) = store
        .reserve_revision_job(&conversation, "rev1", &revision)
        .unwrap();
    store
        .db()
        .execute("UPDATE runs SET spec_revision=2 WHERE id=?1", [run])
        .unwrap();
    let error = store
        .apply_plan_revision(
            &conversation,
            &revision,
            &baseline,
            &specs,
            &BTreeSet::new(),
            &RevisionLease {
                requirement_revision: 1,
                request: &request,
            },
        )
        .unwrap_err();
    assert!(error.to_string().contains("requirements changed"));
    assert_eq!(store.plan_revision(&revision.plan).unwrap(), 0);
    store.interrupt_revision_jobs().unwrap();
    assert_eq!(
        store.revision_job(&request).unwrap()["state"],
        "interrupted"
    );
    assert!(
        store
            .reserve_revision_job(&conversation, "rev1", &revision)
            .unwrap()
            .1
    );
    let mut conflicting = revision.clone();
    conflicting.reason = "Different proposal".into();
    assert!(
        store
            .reserve_revision_job(&conversation, "rev1", &conflicting)
            .is_err()
    );
    assert!(
        store
            .reserve_revision_job(&conversation, "parallel", &revision)
            .is_err()
    );
}

#[test]
fn revision_cannot_retire_a_mandatory_criterion_or_introduce_a_cycle() {
    let (_dir, store, conversation, _, baseline) = fixture();
    let revision = PlanRevision {
        plan: baseline[0].plan_id.clone(),
        base_revision: 0,
        reason: "Retire".into(),
        tasks: vec![],
        retire: vec!["a".into()],
    };
    let specs = revision.merge(&baseline).unwrap();
    let (request, _) = store
        .reserve_revision_job(&conversation, "retire", &revision)
        .unwrap();
    assert!(
        store
            .apply_plan_revision(
                &conversation,
                &revision,
                &baseline,
                &specs,
                &BTreeSet::from([baseline[0].id.clone()]),
                &RevisionLease {
                    requirement_revision: 1,
                    request: &request
                }
            )
            .is_err()
    );
    assert_eq!(store.plan_revision(&revision.plan).unwrap(), 0);
    let mut cycle: TaskContract = serde_json::from_value(baseline[0].spec.clone()).unwrap();
    cycle.depends_on = vec!["a".into()];
    let revision = PlanRevision {
        tasks: vec![cycle],
        retire: vec![],
        ..revision
    };
    assert!(revision.merge(&baseline).is_err());
}

#[test]
fn changed_dependency_inputs_discard_partial_commit_references_but_keep_history() {
    for state in ["working", "verifying", "waiting", "failed"] {
        let (_dir, store, conversation, _, mut tasks) = fixture_for(true);
        tasks[1].round = 1;
        tasks[1].status = state.into();
        tasks[1].base = Some("old-base".into());
        tasks[1].result = Some("old-partial-result".into());
        tasks[1].report = Some("Partial work on previous inputs".into());
        store.update_task(&tasks[1]).unwrap();
        store
            .invalidate_task_dependents(&conversation, &tasks[0])
            .unwrap();
        let fresh = store.task_by_id(&tasks[1].id).unwrap().unwrap();
        assert_eq!(fresh.input_generation, tasks[1].input_generation + 1);
        assert_eq!(fresh.status, "queued");
        assert!(fresh.base.is_none() && fresh.result.is_none());
        let archived = store.task_round(&tasks[1].id, 1).unwrap();
        assert_eq!(archived["base"], "old-base");
        assert_eq!(archived["result"], "old-partial-result");
        assert!(
            store.update_task(&tasks[1]).is_err(),
            "late result from previous inputs must be rejected"
        );
    }
}

#[test]
fn retired_tasks_cannot_reenter_the_queue_through_dependency_corrections_or_mail() {
    let (_dir, store, conversation, _, mut tasks) = fixture_for(true);
    tasks[1].round = 1;
    tasks[1].status = "approved".into();
    store.update_task(&tasks[1]).unwrap();
    let revision = PlanRevision {
        plan: tasks[0].plan_id.clone(),
        base_revision: 0,
        reason: "Retire supporting work without removing acceptance coverage".into(),
        tasks: vec![],
        retire: vec!["b".into()],
    };
    let specs = revision.merge(&tasks).unwrap();
    let (request, _) = store
        .reserve_revision_job(&conversation, "retire-support", &revision)
        .unwrap();
    store
        .apply_plan_revision(
            &conversation,
            &revision,
            &tasks,
            &specs,
            &BTreeSet::from([tasks[1].id.clone()]),
            &RevisionLease {
                requirement_revision: 1,
                request: &request,
            },
        )
        .unwrap();
    let retired = store.task_by_id(&tasks[1].id).unwrap().unwrap();
    store
        .invalidate_task_dependents(&conversation, &tasks[0])
        .unwrap();
    let after = store.task_by_id(&retired.id).unwrap().unwrap();
    assert_eq!(after.status, "superseded");
    assert_eq!(after.input_generation, retired.input_generation);
    assert_eq!(store.plan(&revision.plan).unwrap().len(), 1);
    let error = store
        .enqueue_agent_message(&crate::lean::store::AgentMessage {
            conversation: conversation.clone(),
            plan: Some(revision.plan.clone()),
            recipient: format!("worker:{}", retired.id),
            operation_id: crate::id(),
            expected_round: Some(retired.round),
            expected_run: None,
            text: "Resume retired work".into(),
        })
        .unwrap_err();
    assert!(error.to_string().contains("retired"));
    assert_eq!(
        store.task_by_id(&retired.id).unwrap().unwrap().status,
        "superseded"
    );
    // Deliberate reintroduction is a new reviewed contract with its own UUID.
    let baseline = store.plan(&revision.plan).unwrap();
    let replacement: TaskContract = serde_json::from_value(tasks[1].spec.clone()).unwrap();
    let next = PlanRevision {
        plan: revision.plan.clone(),
        base_revision: 1,
        reason: "Explicitly restore supporting work under a fresh execution identity".into(),
        tasks: vec![replacement],
        retire: vec![],
    };
    let specs = next.merge(&baseline).unwrap();
    let (request, _) = store
        .reserve_revision_job(&conversation, "restore-support", &next)
        .unwrap();
    store
        .apply_plan_revision(
            &conversation,
            &next,
            &baseline,
            &specs,
            &BTreeSet::new(),
            &RevisionLease {
                requirement_revision: 1,
                request: &request,
            },
        )
        .unwrap();
    let fresh = store.task_in_plan_by_id(&next.plan, "b").unwrap().unwrap();
    assert_ne!(fresh.id, retired.id);
    assert_eq!(fresh.status, "queued");
    assert_eq!(
        store
            .task_in_plan_by_id(&next.plan, &retired.id)
            .unwrap()
            .unwrap()
            .status,
        "superseded"
    );
    let event = store
        .events_since(&conversation, 0, 1000)
        .unwrap()
        .into_iter()
        .rfind(|e| e.kind == "graph_task" && e.payload["spec"]["id"] == "b")
        .unwrap();
    assert_eq!(event.payload["id"], fresh.id);
}

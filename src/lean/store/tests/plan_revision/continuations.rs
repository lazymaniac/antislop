use super::*;

fn apply(
    store: &Store,
    conversation: &str,
    revision: &PlanRevision,
    baseline: &[Task],
    specs: &[TaskContract],
    affected: &BTreeSet<String>,
) {
    let (request, _) = store
        .reserve_revision_job(conversation, &crate::id(), revision)
        .unwrap();
    store
        .apply_plan_revision(
            conversation,
            revision,
            baseline,
            specs,
            affected,
            &RevisionLease {
                requirement_revision: 1,
                request: &request,
            },
        )
        .unwrap();
}

#[test]
fn inserting_a_task_before_completed_work_reassesses_the_retained_result() {
    let (_dir, store, conversation, _, mut baseline) = fixture_for(true);
    baseline[0].status = "approved".into();
    baseline[1].status = "approved".into();
    baseline[1].round = 2;
    baseline[1].report = Some("Original report and all evidence".into());
    baseline[1].base = Some("integrated-base".into());
    baseline[1].result = Some("integrated-result".into());
    for t in &baseline {
        store.update_task(t).unwrap();
    }
    let worker = store
        .start_turn(
            &conversation,
            &format!("worker:{}", baseline[1].id),
            "fixture",
            0,
            Some(&baseline[1].plan_id),
        )
        .unwrap();
    store
        .save_turn_result(
            &worker.id,
            &crate::lean::turn_result::TurnResult::Completed {
                summary: "Original".into(),
                report: "Original report and all evidence".into(),
                selection: None,
            },
        )
        .unwrap();
    store
        .finish_turn(&worker.id, TurnStatus::Done, None)
        .unwrap();
    let mut middle: TaskContract = serde_json::from_value(baseline[0].spec.clone()).unwrap();
    middle.id = "middle".into();
    middle.depends_on = vec!["a".into()];
    let mut downstream: TaskContract = serde_json::from_value(baseline[1].spec.clone()).unwrap();
    downstream.depends_on = vec!["middle".into()];
    let revision = PlanRevision {
        plan: baseline[0].plan_id.clone(),
        base_revision: 0,
        reason: "Insert a new prerequisite".into(),
        tasks: vec![middle, downstream],
        retire: vec![],
        redo: vec![],
        replacements: Default::default(),
    };
    let specs = revision.merge(&baseline).unwrap();
    apply(
        &store,
        &conversation,
        &revision,
        &baseline,
        &specs,
        &BTreeSet::from([baseline[1].id.clone()]),
    );
    let current = store.plan(&revision.plan).unwrap();
    assert_eq!(
        current
            .iter()
            .map(|t| t.spec["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["a", "middle", "b"]
    );
    let resumed = &current[2];
    assert_eq!(resumed.status, "queued");
    assert_eq!(resumed.round, 2);
    assert_eq!(resumed.base, baseline[1].base);
    assert_eq!(resumed.result, baseline[1].result);
    assert_eq!(
        resumed.feedback.as_ref().unwrap()["resume_role"],
        "reviewer"
    );
    assert_eq!(
        store.completed_task_output(resumed).unwrap()["report"],
        "Original report and all evidence"
    );
    assert_eq!(
        store.retained_result_ref(resumed).unwrap().as_deref(),
        Some(worker.id.as_str())
    );
    assert_eq!(current[0].status, "approved");
}

#[test]
fn replacement_inherits_unfinished_work_but_cannot_replace_completed_work() {
    let (_dir, store, conversation, _, mut baseline) = fixture();
    baseline[1].status = "failed".into();
    baseline[1].round = 3;
    baseline[1].report = Some("Partial report".into());
    baseline[1].base = Some("old-base".into());
    baseline[1].result = Some("old-checkpoint".into());
    store.update_task(&baseline[1]).unwrap();
    let mut next: TaskContract = serde_json::from_value(baseline[1].spec.clone()).unwrap();
    next.id = "successor".into();
    let revision = PlanRevision {
        plan: baseline[0].plan_id.clone(),
        base_revision: 0,
        reason: "Continue partial work under a new contract".into(),
        tasks: vec![next],
        retire: vec!["b".into()],
        redo: vec![],
        replacements: std::collections::BTreeMap::from([("successor".into(), "b".into())]),
    };
    let specs = revision.merge(&baseline).unwrap();
    apply(
        &store,
        &conversation,
        &revision,
        &baseline,
        &specs,
        &BTreeSet::from([baseline[1].id.clone()]),
    );
    let successor = store
        .task_in_plan_by_id(&revision.plan, "successor")
        .unwrap()
        .unwrap();
    assert_eq!(successor.report, baseline[1].report);
    assert_eq!(successor.result, baseline[1].result);
    assert_eq!(successor.round, 3);
    assert_eq!(
        successor.feedback.as_ref().unwrap()["resume_role"],
        "reviewer"
    );
    assert_eq!(
        store.task_by_id(&baseline[1].id).unwrap().unwrap().status,
        "superseded"
    );
    baseline[1].status = "approved".into();
    assert!(
        revision
            .merge(&baseline)
            .unwrap_err()
            .to_string()
            .contains("completed tasks cannot be replaced")
    );
}

#[test]
fn requirements_staleness_does_not_drop_an_unintegrated_checkpoint_on_replan() {
    let (_dir, store, conversation, run, mut tasks) = fixture();
    store.queue_plan(&conversation, &tasks[0].plan_id).unwrap();
    tasks[0].status = "stale".into();
    tasks[0].round = 2;
    tasks[0].report = Some("Retained unfinished work".into());
    tasks[0].base = Some("partial-base".into());
    tasks[0].result = Some("partial-checkpoint".into());
    store.update_task(&tasks[0]).unwrap();
    let next = store
        .add_plan(&conversation, &[tasks[0].spec.clone()])
        .unwrap();
    store
        .link_plan_run(&conversation, &next[0].plan_id, &run)
        .unwrap();
    store.queue_plan(&conversation, &next[0].plan_id).unwrap();
    let continued = store.task_by_id(&next[0].id).unwrap().unwrap();
    assert_eq!(continued.base, tasks[0].base);
    assert_eq!(continued.result, tasks[0].result);
    assert_eq!(
        continued.feedback.as_ref().unwrap()["resume_role"],
        "reviewer"
    );
}

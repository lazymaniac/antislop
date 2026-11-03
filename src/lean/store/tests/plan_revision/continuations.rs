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
fn replanning_preserves_unintegrated_checkpoints_without_approving_them() {
    for state in ["stale", "coordinating", "failed", "waiting"] {
        let (_dir, store, conversation, run, mut tasks) = fixture();
        store.queue_plan(&conversation, &tasks[0].plan_id).unwrap();
        tasks[0].status = state.into();
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
        assert_eq!(continued.status, "queued", "{state}");
        assert_eq!(continued.base, tasks[0].base);
        assert_eq!(continued.result, tasks[0].result);
        assert_eq!(
            continued.feedback.as_ref().unwrap()["resume_role"],
            "reviewer"
        );
    }
}

#[test]
fn replanning_prefers_the_latest_unfinished_checkpoint_over_an_older_exact_contract() {
    let (_dir, store, conversation, run, mut original) = fixture();
    original[0].status = "approved".into();
    original[0].report = Some("Older approved report".into());
    store.update_task(&original[0]).unwrap();
    let mut changed = original[0].spec.clone();
    changed["goal"] = "Repair the existing instrument".into();
    let mut recent = store.add_plan(&conversation, &[changed]).unwrap().remove(0);
    store
        .link_plan_run(&conversation, &recent.plan_id, &run)
        .unwrap();
    recent.status = "coordinating".into();
    recent.report = Some("Latest repair needs source review".into());
    recent.base = Some("repair-base".into());
    recent.result = Some("latest-repair".into());
    store.update_task(&recent).unwrap();
    let next = store
        .add_plan(&conversation, &[original[0].spec.clone()])
        .unwrap();
    store
        .link_plan_run(&conversation, &next[0].plan_id, &run)
        .unwrap();
    store.queue_plan(&conversation, &next[0].plan_id).unwrap();
    let continued = store.task_by_id(&next[0].id).unwrap().unwrap();
    assert_eq!(continued.status, "queued");
    assert_eq!(continued.report, recent.report);
    assert_eq!(continued.base, recent.base);
    assert_eq!(continued.result, recent.result);
}

#[test]
fn a_reviewed_successor_recovers_historical_code_without_approval_or_retirement() {
    let (_dir, store, conversation, run, mut previous) = fixture();
    let source = &mut previous[1];
    source.spec["writes"] = true.into();
    source.spec["change_kind"] = "code".into();
    source.status = "coordinating".into();
    source.round = 3;
    source.report = Some("Technical repair complete; source audit still pending".into());
    source.base = Some("repair-base".into());
    source.result = Some("repair-checkpoint".into());
    store.update_task(source).unwrap();
    store
        .db()
        .execute(
            "UPDATE tasks SET spec=?2 WHERE id=?1",
            rusqlite::params![source.id, source.spec.to_string()],
        )
        .unwrap();
    let source = source.clone();
    let next = store
        .add_plan(&conversation, &[previous[0].spec.clone()])
        .unwrap();
    store
        .link_plan_run(&conversation, &next[0].plan_id, &run)
        .unwrap();
    store
        .bind_task_ledger(&conversation, &run, &next[0].plan_id)
        .unwrap();
    store.queue_plan(&conversation, &next[0].plan_id).unwrap();
    let baseline = store.plan(&next[0].plan_id).unwrap();
    let mut successor: TaskContract = serde_json::from_value(source.spec.clone()).unwrap();
    successor.id = "recover-repair".into();
    successor.goal =
        "Recover and assess the existing technical repair; source audit is separate".into();
    let revision = PlanRevision {
        plan: next[0].plan_id.clone(),
        base_revision: 0,
        reason: "Recover code omitted by the earlier replan".into(),
        tasks: vec![successor.clone()],
        retire: vec![],
        redo: vec![],
        replacements: std::collections::BTreeMap::from([(successor.id.clone(), source.id.clone())]),
    };
    let candidates = store
        .unfinished_checkpoints(&conversation, &revision.plan)
        .unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0]["source_task"], source.id);
    assert_eq!(
        store
            .revision_checkpoints(&conversation, &revision)
            .unwrap()[0]["target"],
        successor.id
    );
    let other = store.create_conversation("other").unwrap();
    assert!(
        store
            .unfinished_checkpoints(&other.id, &revision.plan)
            .unwrap()
            .is_empty()
    );
    assert!(store.revision_checkpoints(&other.id, &revision).is_err());
    let specs = revision.merge(&baseline).unwrap();
    apply(
        &store,
        &conversation,
        &revision,
        &baseline,
        &specs,
        &BTreeSet::new(),
    );
    let recovered = store
        .task_in_plan_by_id(&revision.plan, &successor.id)
        .unwrap()
        .unwrap();
    assert_eq!(recovered.status, "queued");
    assert_eq!(recovered.base, source.base);
    assert_eq!(recovered.result, source.result);
    assert_eq!(recovered.report, source.report);
    assert_eq!(recovered.feedback.unwrap()["resume_role"], "reviewer");
    assert_eq!(
        store.task_by_id(&source.id).unwrap().unwrap().status,
        "coordinating"
    );
    assert_eq!(store.plan_revision(&revision.plan).unwrap(), 1);
    assert!(
        store
            .unfinished_checkpoints(&conversation, &revision.plan)
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .revision_checkpoints(&conversation, &revision)
            .unwrap_err()
            .to_string()
            .contains("already has an active")
    );
    let mut read_only = revision.clone();
    read_only.tasks[0].writes = false;
    assert!(read_only.merge(&baseline).is_err());
    for state in ["approved", "working"] {
        let mut unavailable = source.clone();
        unavailable.status = state.into();
        store.update_task(&unavailable).unwrap();
        assert!(
            store
                .revision_checkpoints(&conversation, &revision)
                .is_err(),
            "{state}"
        );
    }
}

#[test]
fn a_checkpoint_that_changed_during_review_cannot_partially_apply_a_revision() {
    let (_dir, store, conversation, run, mut old) = fixture();
    old[1].spec["writes"] = true.into();
    old[1].spec["change_kind"] = "code".into();
    old[1].status = "coordinating".into();
    old[1].base = Some("base".into());
    old[1].result = Some("checkpoint".into());
    store.update_task(&old[1]).unwrap();
    store
        .db()
        .execute(
            "UPDATE tasks SET spec=?2 WHERE id=?1",
            rusqlite::params![old[1].id, old[1].spec.to_string()],
        )
        .unwrap();
    let baseline = store
        .add_plan(&conversation, &[old[0].spec.clone()])
        .unwrap();
    store
        .link_plan_run(&conversation, &baseline[0].plan_id, &run)
        .unwrap();
    store
        .bind_task_ledger(&conversation, &run, &baseline[0].plan_id)
        .unwrap();
    let mut successor: TaskContract = serde_json::from_value(old[1].spec.clone()).unwrap();
    successor.id = "recover".into();
    let revision = PlanRevision {
        plan: baseline[0].plan_id.clone(),
        base_revision: 0,
        reason: "Recover existing code".into(),
        tasks: vec![successor],
        retire: vec![],
        redo: vec![],
        replacements: std::collections::BTreeMap::from([("recover".into(), old[1].id.clone())]),
    };
    let specs = revision.merge(&baseline).unwrap();
    store
        .revision_checkpoints(&conversation, &revision)
        .unwrap();
    old[1].status = "approved".into();
    store.update_task(&old[1]).unwrap();
    let (request, _) = store
        .reserve_revision_job(&conversation, &crate::id(), &revision)
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
    assert!(error.to_string().contains("must be unfinished"), "{error}");
    assert_eq!(store.plan_revision(&revision.plan).unwrap(), 0);
    assert!(
        store
            .task_in_plan_by_id(&revision.plan, "recover")
            .unwrap()
            .is_none()
    );
}

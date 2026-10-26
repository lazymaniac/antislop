use super::*;

fn spec(id: &str) -> TaskSpec {
    serde_json::from_value(json!({
        "id":id,"title":id,"goal":"Provide verified source evidence",
        "requirement_ids":["req"],"criterion_ids":["criterion"],
        "scope":["src"],"kind":"analysis","writes":false,
        "verifier":{"focus":"Check evidence independently","risks":["fabricated history"]}
    }))
    .unwrap()
}

fn blocked() -> (tempfile::TempDir, Store, String, Task) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("core.db")).unwrap();
    let conversation = store
        .create_conversation(dir.path().to_str().unwrap())
        .unwrap()
        .id;
    let mut task = store
        .add_plan(
            &conversation,
            &[serde_json::to_value(spec("input")).unwrap()],
        )
        .unwrap()
        .remove(0);
    task.status = status::NEEDS_USER.into();
    task.round = 11;
    task.report = Some("```blocker\n{\"kind\":\"missing_input\",\"needs\":\"historical source identity\",\"why\":\"never recorded\",\"suggest\":\"use verified current inputs for a NEW assessment\"}\n```".into());
    task.verdict = Some(json!({"approved":false}));
    task.feedback = Some(json!({"round":11}));
    let request = "Review source evidence independently.";
    let message = store.add_message(&conversation, "user", request).unwrap();
    let run = store.create_run(&conversation, request).unwrap();
    let source = store
        .register_message_source(&conversation, &run.id, message.seq)
        .unwrap();
    let change = crate::lean::ledger::LedgerChange {
        conversation: conversation.clone(),
        run_id: run.id.clone(),
        expected_spec_revision: 0,
        operation_id: crate::id(),
        reason: "test requirement".into(),
        entries: vec![serde_json::from_value(json!({"id":"req",
            "source":{"source_id":source.id,"start":0,"end":request.len()},
            "interpretation":request,"criteria":[{"id":"criterion","text":request,"mandatory":true}]
        })).unwrap()],
    };
    store
        .apply_ledger_change(&change, dir.path(), true)
        .unwrap();
    store
        .link_plan_run(&conversation, &task.plan_id, &run.id)
        .unwrap();
    store
        .bind_task_ledger(&conversation, &run.id, &task.plan_id)
        .unwrap();
    store.update_task(&task).unwrap();
    (dir, store, conversation, task)
}

fn clarification(task: &Task) -> TaskSpec {
    let mut spec: TaskSpec = serde_json::from_value(task.spec.clone()).unwrap();
    spec.goal =
        "Record missing history; supply independently verified current inputs for a new assessment"
            .into();
    spec.verifier.focus = "Verify current evidence and preserve historical limitations; downstream assessment remains required".into();
    spec
}

fn verdict(approved: bool) -> Verdict {
    serde_json::from_value(json!({"approved":approved,"findings":[]})).unwrap()
}

#[test]
fn exhausted_additions_offer_clarification_instead_of_another_discovery_task() {
    let (_dir, _store, _conversation, mut task) = blocked();
    let mut input = spec("input");
    input.resolves = Some("architect".into());
    task.spec = serde_json::to_value(&input).unwrap();
    let mut baseline = spec("baseline");
    baseline.resolves = Some("input".into());
    let mut revalidation = spec("revalidation");
    revalidation.resolves = Some("input".into());
    let plan = vec![
        (baseline, status::APPROVED.into()),
        (revalidation, status::APPROVED.into()),
        (input, status::NEEDS_USER.into()),
        (spec("architect"), status::QUEUED.into()),
    ];
    let limits = RecoveryLimits {
        retries: 3,
        clarifications: 0,
        ..RecoveryLimits::default()
    };
    let text = options_text(&recovery_options(&task, &plan, true, &limits));
    assert!(
        text.contains("add_task: unavailable — the blocker was already tried twice"),
        "{text}"
    );
    assert!(text.contains("split_task: unavailable"));
    assert!(text.contains("clarify_task: available"));
    assert!(text.contains("retry_with_guidance: unavailable"));
    let limits = HashMap::from([(task.id.clone(), limits)]);
    let prompt = super::super::core_prompt_helpers::blocked_report(&[task], true, &limits);
    assert!(!prompt.contains("The plan needs a structural repair"));
    assert!(prompt.contains("use verified current inputs for a NEW assessment"));
}

#[test]
fn clarification_cannot_change_obligations_access_or_dependencies() {
    let original = spec("input");
    let mut revised = original.clone();
    revised.goal = "Revalidate against current evidence".into();
    blocker::check_clarification(&original, &revised).unwrap();
    for (field, value) in [
        ("requirement_ids", json!([])),
        ("criterion_ids", json!([])),
        ("depends_on", json!(["downstream"])),
        ("scope", json!(["."])),
        ("writes", json!(true)),
        ("resolves", json!("other")),
        ("part_of", json!("other")),
    ] {
        let mut changed = serde_json::to_value(&revised).unwrap();
        changed[field] = value;
        assert!(
            blocker::check_clarification(&original, &serde_json::from_value(changed).unwrap())
                .is_err(),
            "{field}"
        );
    }
    revised.verifier.risks.clear();
    assert!(blocker::check_clarification(&original, &revised).is_err());
}

#[test]
fn clarification_is_reviewed_audited_and_invalidates_old_results_without_approval() {
    let (dir, store, conversation, task) = blocked();
    let revised = clarification(&task);
    assert!(
        store
            .clarify_task(&conversation, &task, &revised, &verdict(false), "rejected")
            .is_err()
    );
    assert_eq!(store.clarification_count(&task.id).unwrap(), 0);
    store
        .clarify_task(
            &conversation,
            &task,
            &revised,
            &verdict(true),
            "Verified replacement inputs exist",
        )
        .unwrap();
    let current = store.plan(&task.plan_id).unwrap().remove(0);
    assert_eq!(current.status, status::QUEUED);
    assert_eq!(current.round, 11);
    assert_eq!(current.input_generation, task.input_generation + 1);
    assert_eq!(current.report, task.report);
    assert!(current.verdict.is_none() && current.feedback.is_none());
    assert!(
        store
            .update_task(&task)
            .unwrap_err()
            .to_string()
            .contains("stale task generation")
    );
    let events = store.events_since(&conversation, 0, 100).unwrap();
    let audit = events
        .iter()
        .find(|event| event.kind == "task_clarified")
        .unwrap();
    assert_eq!(audit.payload["before"], task.spec);
    assert_eq!(
        audit.payload["after"],
        serde_json::to_value(&revised).unwrap()
    );
    assert_eq!(audit.payload["review"]["approved"], true);
    drop(store);
    let reopened = Store::open(&dir.path().join("core.db")).unwrap();
    assert_eq!(reopened.clarification_count(&task.id).unwrap(), 1);
}

#[test]
fn changed_task_rejects_stale_clarification_without_partial_events() {
    let (_dir, store, conversation, mut task) = blocked();
    let revised = clarification(&task);
    let before = task.clone();
    task.round += 1;
    store.update_task(&task).unwrap();
    assert!(
        store
            .clarify_task(
                &conversation,
                &before,
                &revised,
                &verdict(true),
                "stale review"
            )
            .is_err()
    );
    assert_eq!(store.clarification_count(&task.id).unwrap(), 0);
    let current = store.plan(&task.plan_id).unwrap().remove(0);
    assert_eq!(current.round, task.round);
    assert_eq!(current.status, status::NEEDS_USER);
    assert_eq!(current.spec, before.spec);
}

#[test]
fn permission_blockers_and_spent_clarification_budget_are_not_offered() {
    let (_dir, store, conversation, mut task) = blocked();
    for round in 0..2 {
        let mut revised = clarification(&task);
        revised.goal.push_str(&format!(" clarification {round}"));
        store
            .clarify_task(
                &conversation,
                &task,
                &revised,
                &verdict(true),
                "current evidence",
            )
            .unwrap();
        task = store.plan(&task.plan_id).unwrap().remove(0);
        task.status = status::NEEDS_USER.into();
        store.update_task(&task).unwrap();
    }
    assert!(
        store
            .clarify_task(
                &conversation,
                &task,
                &clarification(&task),
                &verdict(true),
                "third"
            )
            .unwrap_err()
            .to_string()
            .contains("twice")
    );
    let limits = RecoveryLimits {
        retries: 0,
        clarifications: 2,
        ..RecoveryLimits::default()
    };
    let text = options_text(&recovery_options(&task, &[], true, &limits));
    assert!(text.contains("clarify_task: unavailable"));
    task.report = Some(
        "```blocker\n{\"kind\":\"permission\",\"needs\":\"access\",\"why\":\"denied\"}\n```".into(),
    );
    let text = options_text(&recovery_options(
        &task,
        &[],
        true,
        &RecoveryLimits::default(),
    ));
    for action in ["add_task", "split_task", "clarify_task"] {
        assert!(text.contains(&format!("{action}: unavailable — only the user")));
    }
}

#[test]
fn independent_reviewer_controls_clarification_and_no_result_is_force_approved() {
    use std::os::unix::fs::PermissionsExt;
    for approved in [false, true] {
        let (dir, store, conversation, task) = blocked();
        drop(store);
        let script = dir.path().join("review-cli");
        std::fs::write(
            &script,
            format!(
                r#"#!/usr/bin/env python3
import json,sys
prompt=sys.argv[-1]
assert 'Review a proposed clarification' in prompt
assert 'Preserve every substantive accepted requirement' in prompt
answer='```verdict\n'+json.dumps({{'approved':{},'findings':[]}})+'\n```'
print(json.dumps({{'type':'item.completed','item':{{'type':'agent_message','text':answer}}}}))
"#,
                if approved { "True" } else { "False" }
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        crate::lean::native::fixture::interactive(&script);
        let config = include_str!("../../../examples/antislop.toml")
            .replace(
                "binary = \"codex\"",
                &format!("binary = {:?}", script.to_str().unwrap()),
            )
            .replace("provider = \"agy\"", "provider = \"codex\"");
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let core = Core::open(
            dir.path(),
            crate::config::Config::from_toml(&config).unwrap(),
        )
        .unwrap();
        // Keep the plan paused here so the test observes the recovery boundary.
        core.plans
            .lock()
            .unwrap()
            .insert(conversation.clone(), Arc::new(AtomicBool::new(false)));
        let revised = clarification(&task);
        let step: tasks::Unblocking = serde_json::from_value(json!({
            "task":"input","action":"clarify_task","spec":revised,
            "message":"Verified current inputs replace unauthenticated historical claims; independent assessment remains required"
        })).unwrap();
        let result =
            core.clarify_blocked_task(&conversation, &task.plan_id, &step, &AtomicBool::new(false));
        let current = core.store.plan(&task.plan_id).unwrap().remove(0);
        assert_eq!(result.is_ok(), approved, "{result:?}");
        assert_eq!(
            core.store.clarification_count(&task.id).unwrap(),
            usize::from(approved)
        );
        if approved {
            assert_eq!(current.status, status::QUEUED);
            assert!(current.verdict.is_none());
            assert_eq!(current.spec, serde_json::to_value(revised).unwrap());
        } else {
            assert_eq!(current.status, status::NEEDS_USER);
            assert_eq!(current.spec, task.spec);
        }
        let turn = core
            .store
            .last_turn(&conversation, PLAN_VERIFIER)
            .unwrap()
            .unwrap();
        assert_eq!(turn.status, store::TurnStatus::Done);
    }
}

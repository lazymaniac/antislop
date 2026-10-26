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
fn completed_additions_do_not_disable_new_recovery_proposals() {
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
    let state = RecoveryState::default();
    let text = options_text(&recovery_options(&task, &plan, &state));
    assert!(text.contains("add_task: available"));
    assert!(text.contains("split_task: unavailable"));
    assert!(text.contains("clarify_task: available"));
    assert!(text.contains("retry_with_guidance: available"));
    let states = HashMap::from([(task.id.clone(), state)]);
    let prompt = super::super::core_prompt_helpers::blocked_report(&[task], true, &states);
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
            .clarify_task(
                &conversation,
                &task,
                &revised,
                &verdict(false),
                "rejected",
                None
            )
            .is_err()
    );
    assert!(
        store
            .recovery_history(&task.id, "clarify_task")
            .unwrap()
            .is_empty()
    );
    store
        .clarify_task(
            &conversation,
            &task,
            &revised,
            &verdict(true),
            "Verified replacement inputs exist",
            None,
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
    assert_eq!(
        reopened.plan(&task.plan_id).unwrap()[0].spec,
        serde_json::to_value(revised).unwrap()
    );
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
                "stale review",
                None,
            )
            .is_err()
    );
    assert!(
        store
            .recovery_history(&task.id, "clarify_task")
            .unwrap()
            .is_empty()
    );
    let current = store.plan(&task.plan_id).unwrap().remove(0);
    assert_eq!(current.round, task.round);
    assert_eq!(current.status, status::NEEDS_USER);
    assert_eq!(current.spec, before.spec);
}

#[test]
fn permission_blockers_cannot_be_repaired_automatically() {
    let (_dir, _store, _conversation, mut task) = blocked();
    task.report = Some(
        "```blocker\n{\"kind\":\"permission\",\"needs\":\"access\",\"why\":\"denied\"}\n```".into(),
    );
    let text = options_text(&recovery_options(&task, &[], &RecoveryState::default()));
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
approved={}
findings=[] if approved else [{{'severity':'medium','issue':'Provide source evidence for the accepted criterion'}}]
answer='```verdict\n'+json.dumps({{'approved':approved,'findings':findings}})+'\n```'
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
            core.store
                .recovery_history(&task.id, "clarify_task")
                .unwrap()
                .len(),
            1
        );
        if approved {
            assert_eq!(current.status, status::QUEUED);
            assert!(current.verdict.is_none());
            assert_eq!(current.spec, serde_json::to_value(revised).unwrap());
        } else {
            assert_eq!(current.status, status::NEEDS_USER);
            assert_eq!(current.spec, task.spec);
            let feedback = core.store.messages_by(&conversation, RUNTIME, 0).unwrap();
            assert_eq!(feedback.len(), 1);
            assert!(
                feedback[0]
                    .text
                    .contains("Clarification review rejected for input")
            );
            assert!(feedback[0].text.contains("no attempt quota"));
            assert_eq!(
                core.unanswered(&conversation).unwrap(),
                Some(feedback[0].seq)
            );
            assert!(
                core.clarify_blocked_task(
                    &conversation,
                    &task.plan_id,
                    &step,
                    &AtomicBool::new(false)
                )
                .unwrap_err()
                .to_string()
                .contains("no progress")
            );
            assert_eq!(
                core.store
                    .messages_by(&conversation, RUNTIME, 0)
                    .unwrap()
                    .len(),
                1
            );
            let reopened = Store::open(&dir.path().join("core.db")).unwrap();
            let observation = core.recovery_observation(&conversation, &task).unwrap();
            assert!(
                reopened
                    .reserve_clarification(
                        &conversation,
                        &task,
                        step.spec.as_ref().unwrap(),
                        &observation,
                        step.message.as_deref().unwrap()
                    )
                    .unwrap_err()
                    .to_string()
                    .contains("no progress")
            );
        }
        let turn = core
            .store
            .last_turn(&conversation, PLAN_VERIFIER)
            .unwrap()
            .unwrap();
        assert_eq!(turn.status, store::TurnStatus::Done);
    }
}

#[test]
fn interrupted_identical_proposal_is_blocked_after_restart_but_new_evidence_is_allowed() {
    let (dir, store, conversation, task) = blocked();
    let next = clarification(&task);
    let attempt = store
        .reserve_clarification(&conversation, &task, &next, "state", "evidence")
        .unwrap();
    drop(store);
    let store = Store::open(&dir.path().join("core.db")).unwrap();
    assert!(
        store
            .reserve_clarification(&conversation, &task, &next, "state", "evidence")
            .unwrap_err()
            .to_string()
            .contains("no progress")
    );
    store
        .reserve_clarification(&conversation, &task, &next, "changed evidence", "evidence")
        .unwrap();
    let mut mismatched = next.clone();
    mismatched.goal.push_str(" not reviewed");
    assert!(
        store
            .clarify_task(
                &conversation,
                &task,
                &mismatched,
                &verdict(true),
                "evidence",
                Some(&attempt)
            )
            .is_err()
    );
    store
        .clarify_task(
            &conversation,
            &task,
            &next,
            &verdict(true),
            "evidence",
            Some(&attempt),
        )
        .unwrap();
    assert!(
        store
            .clarify_task(
                &conversation,
                &task,
                &next,
                &verdict(true),
                "evidence",
                Some(&attempt)
            )
            .is_err()
    );
}

#[test]
fn renamed_prerequisites_and_round_changes_are_not_new_progress() {
    let (dir, store, conversation, mut task) = blocked();
    let mut input = serde_json::to_value(spec("first")).unwrap();
    let proposal = recovery_progress::proposal("add_task", Some(&input), None, &[]);
    store
        .reserve_recovery_attempt(&conversation, &task, "add_task", "same evidence", &proposal)
        .unwrap();
    input["id"] = json!("renamed");
    input["title"] = json!("Another label");
    input["goal"] = json!("Provide   verified\nsource evidence");
    let repeated = recovery_progress::proposal("add_task", Some(&input), None, &[]);
    assert!(
        store
            .reserve_recovery_attempt(&conversation, &task, "add_task", "same evidence", &repeated)
            .unwrap_err()
            .to_string()
            .contains("no progress")
    );
    drop(store);
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let config =
        crate::config::Config::from_toml(include_str!("../../../examples/antislop.toml")).unwrap();
    let core = Core::open(dir.path(), config).unwrap();
    task.report = Some("agent did not finish turn 11111111-1111-4111-8111-111111111111".into());
    let first = core.recovery_observation(&conversation, &task).unwrap();
    task.round += 1;
    task.report = Some("agent did not finish turn 22222222-2222-4222-8222-222222222222".into());
    assert_eq!(
        first,
        core.recovery_observation(&conversation, &task).unwrap()
    );
    task.input_generation += 1;
    assert_ne!(
        first,
        core.recovery_observation(&conversation, &task).unwrap()
    );
}

#[test]
fn retry_aliases_cannot_repeat_unchanged_guidance_after_restart() {
    let (dir, store, conversation, task) = blocked();
    let proposal = recovery_progress::proposal(
        "extend_rounds",
        None,
        Some("inspect dependency evidence"),
        &[],
    );
    store
        .reserve_recovery_attempt(&conversation, &task, "extend_rounds", "failure", &proposal)
        .unwrap();
    drop(store);
    let store = Store::open(&dir.path().join("core.db")).unwrap();
    let same = recovery_progress::proposal(
        "retry_with_guidance",
        None,
        Some("inspect  dependency\nevidence"),
        &[],
    );
    assert!(
        store
            .reserve_recovery_attempt(
                &conversation,
                &task,
                "retry_with_guidance",
                "failure",
                &same
            )
            .is_err()
    );
    for index in 0..5 {
        let proposal = recovery_progress::proposal(
            "retry_with_guidance",
            None,
            Some(&format!("correct evidence item {index}")),
            &[],
        );
        store
            .reserve_recovery_attempt(
                &conversation,
                &task,
                "retry_with_guidance",
                "failure",
                &proposal,
            )
            .unwrap();
    }
    assert_eq!(
        store
            .recovery_history(&task.id, "extend_rounds")
            .unwrap()
            .len(),
        6
    );
}

#[test]
fn changed_retry_guidance_requires_independent_progress_review() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, store, conversation, task) = blocked();
    drop(store);
    let script = dir.path().join("progress-cli");
    std::fs::write(&script, r#"#!/usr/bin/env python3
import json,sys
prompt=sys.argv[-1]
assert prompt.startswith('Assess progress of a proposed recovery')
assert 'Accepted ledger:' in prompt
assert 'Previous attempts and reviews:' in prompt
proposal=json.loads(prompt.split('\nProposal: ',1)[1].split('\n',1)[0])
approved=proposal['guidance'].startswith('inspect missing source boundary')
findings=[] if approved else [{'severity':'medium','issue':'Rewording the same failed approach does not address the missing input'}]
answer='```verdict\n'+json.dumps({'approved':approved,'findings':findings})+'\n```'
print(json.dumps({'type':'item.completed','item':{'type':'agent_message','text':answer}}))
"#).unwrap();
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
    let step = |guidance: &str| {
        serde_json::from_value::<tasks::Unblocking>(json!({
            "task":"input","action":"retry_with_guidance","message":guidance
        }))
        .unwrap()
    };
    let cancel = AtomicBool::new(false);
    core.check_recovery_progress(&conversation, &task, &step("retry the task"), &cancel)
        .unwrap();
    assert!(
        core.check_recovery_progress(&conversation, &task, &step("try the task again"), &cancel)
            .unwrap_err()
            .to_string()
            .contains("no progress")
    );
    for index in 0..5 {
        core.check_recovery_progress(
            &conversation,
            &task,
            &step(&format!("inspect missing source boundary {index}")),
            &cancel,
        )
        .unwrap();
    }
    assert_eq!(
        core.store
            .recovery_history(&task.id, "retry_with_guidance")
            .unwrap()
            .len(),
        7
    );
    let current = core.store.plan(&task.plan_id).unwrap().remove(0);
    assert_eq!(current.status, status::NEEDS_USER);
    assert_eq!(current.verdict, task.verdict);
}

#[test]
fn orchestrator_keeps_correcting_beyond_old_quota_but_stops_semantic_rewordings() {
    use std::os::unix::fs::PermissionsExt;
    for accept_correction in [true, false] {
        let (dir, store, conversation, task) = blocked();
        if accept_correction {
            std::fs::write(dir.path().join("allow-correction"), "").unwrap();
        }
        let mut proposed = clarification(&task);
        proposed
            .goal
            .push_str("; allocation instrumentation is mandatory");
        let mut corrected = proposed.clone();
        corrected.goal =
        "Use current evidence; instrumentation is conditional on an accepted performance criterion"
            .into();
        for (name, spec) in [
            ("initial.json", proposed),
            ("corrected.json", corrected.clone()),
        ] {
            std::fs::write(dir.path().join(name), serde_json::to_string(&json!({"actions":[{
            "task":"input","action":"clarify_task","spec":spec,"message":"Preserve accepted criteria and require independent review"
        }]})).unwrap()).unwrap();
        }
        drop(store);
        let script = dir.path().join("recovery-cli");
        std::fs::write(&script, r#"#!/usr/bin/env python3
import json,sys,pathlib
prompt=sys.argv[-1]
root=pathlib.Path(__file__).parent
if 'Review a proposed clarification' in prompt:
    spec=json.loads(prompt.split('\nProposed:\n',1)[1].split('\nRationale:\n',1)[0])
    calls=root/'review-count'
    count=int(calls.read_text())+1 if calls.exists() else 1
    calls.write_text(str(count))
    improving=(root/'allow-correction').exists()
    approved=improving and count>=5
    findings=[] if approved else [{'severity':'medium','issue':('Supply corrected evidence item '+str(count)) if improving else 'Make allocation instrumentation conditional on an accepted performance criterion','where':'historical note'}]
    if not improving and count>1:
        findings[0]['code']='recovery_no_progress'
        findings[0]['severity']='low'
        approved=True
    answer='```verdict\n'+json.dumps({'approved':approved,'findings':findings})+'\n```'
else:
    rejected='Clarification review rejected for input' in prompt
    proposal=root/('corrected.json' if rejected else 'initial.json')
    action=json.loads(proposal.read_text())
    if rejected:
        assert 'Independent reviewer findings' in prompt
        count=int((root/'review-count').read_text())
        action['actions'][0]['spec']['goal']+='; evidence correction '+str(count)
    answer='Correcting the evidence contract.\n```recover\n'+json.dumps(action)+'\n```'
print(json.dumps({'type':'item.completed','item':{'type':'agent_message','text':answer}}))
"#).unwrap();
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
        core.plans
            .lock()
            .unwrap()
            .insert(conversation.clone(), Arc::new(AtomicBool::new(false)));
        core.orchestrate(&conversation, &AtomicBool::new(false));
        let current = core.store.plan(&task.plan_id).unwrap().remove(0);
        if accept_correction {
            assert_eq!(current.status, status::QUEUED);
            let mut expected = corrected;
            expected.goal.push_str("; evidence correction 4");
            assert_eq!(current.spec, serde_json::to_value(expected).unwrap());
            assert!(current.verdict.is_none());
        } else {
            assert_eq!(current.status, status::NEEDS_USER);
            assert_eq!(current.spec, task.spec);
            assert_eq!(current.verdict, task.verdict);
        }
        assert_eq!(
            core.store
                .recovery_history(&task.id, "clarify_task")
                .unwrap()
                .len(),
            if accept_correction { 5 } else { 2 }
        );
        assert!(core.unanswered(&conversation).unwrap().is_none());
        assert_eq!(
            core.store
                .messages_by(&conversation, RUNTIME, 0)
                .unwrap()
                .len(),
            if accept_correction { 4 } else { 1 }
        );
    }
}

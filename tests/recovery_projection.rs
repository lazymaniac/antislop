//! Read-only compatibility probe against an explicitly supplied owned fixture.
#[test]
#[ignore = "requires ANTISLOP_FAILED_WAVE_UI_INPUT; projects actual owned failed sessions without provider work"]
fn retained_failed_review_wave_projects_each_latest_task_session() {
    let path =
        std::env::var_os("ANTISLOP_FAILED_WAVE_UI_INPUT").expect("explicit owned fixture required");
    let bytes = std::fs::read(path).unwrap();
    let run: architect::domain::Run = serde_json::from_slice(&bytes).unwrap();
    run.session_ledger.validate_for_run(&run.id).unwrap();
    let document = serde_json::to_value(&run).unwrap();
    let before = serde_json::to_vec(&document).unwrap();
    let snapshot =
        architect::tui::model::Snapshot::from_document(&document, &serde_json::json!({}));
    let mut checked = 0;
    for task in &run.team.as_ref().unwrap().tasks {
        if task.stage != architect::team::TaskStage::Discovery {
            continue;
        }
        let latest = run
            .session_ledger
            .turn_order
            .iter()
            .rev()
            .filter_map(|id| run.session_ledger.turns.get(id))
            .find(|t| {
                let s = &run.session_ledger.sessions[&t.session_id];
                s.scope.task_id.as_ref() == Some(&task.id)
                    && s.scope.role == architect::config::RoleKind::Verifier
            })
            .unwrap();
        assert_eq!(latest.state, architect::sessions::TurnState::Failed);
        let agent = snapshot
            .agents
            .iter()
            .find(|a| a.id == task.verifier_id)
            .unwrap();
        let details: serde_json::Value = serde_json::from_str(&agent.details["Session"]).unwrap();
        assert_eq!(details["current_session"], latest.session_id);
        checked += 1;
    }
    assert_eq!(checked, 3);
    assert_eq!(serde_json::to_vec(&document).unwrap(), before);
    println!(
        "Three actual failed verifier sessions are selected from their owning task reservation order; older bindings cannot hide them; no canonical write or provider work."
    );
}
#[test]
#[ignore = "requires ANTISLOP_CONTEXT_CITATIONS_INPUT; validates an owned completed native result without accepting it"]
fn completed_native_context_uses_only_exact_captured_members_and_authenticated_checks() {
    let path = std::env::var_os("ANTISLOP_CONTEXT_CITATIONS_INPUT")
        .expect("explicit owned fixture required");
    let run: architect::domain::Run =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let before = serde_json::to_vec(&run).unwrap();
    run.session_ledger.validate_for_run(&run.id).unwrap();
    let team = run.team.as_ref().unwrap();
    let task = team
        .tasks
        .iter()
        .find(|t| t.id == "task-contextanalyst")
        .unwrap();
    let owner = team.agents.iter().find(|a| a.id == task.owner_id).unwrap();
    let session = &run.session_ledger.sessions[owner.session_id.as_ref().unwrap()];
    let turn = &run.session_ledger.turns[session.current_turn_id.as_ref().unwrap()];
    assert!(turn.provider_completed && turn.state == architect::sessions::TurnState::Completed);
    let output: architect::team::models::Output =
        architect::transport::decode(turn.output.as_ref().unwrap().as_bytes()).unwrap();
    architect::team::models::validate_output(&run, task, owner.role, &output).unwrap();
    let mut missing = run.clone();
    missing.task_checks.clear();
    assert!(architect::team::models::validate_output(&missing, task, owner.role, &output).is_err());
    let mut missing = run.clone();
    missing.input_snapshot.clear();
    assert!(architect::team::models::validate_output(&missing, task, owner.role, &output).is_err());
    let mut changed = output.clone();
    changed.context.as_mut().unwrap().model.facts[0].source_hash = "0".repeat(64);
    assert!(architect::team::models::validate_output(&run, task, owner.role, &changed).is_err());
    assert_eq!(serde_json::to_vec(&run).unwrap(), before);
    if std::env::var_os("ANTISLOP_NATIVE_READONLY_PROPOSAL").is_some() {
        let mut preview = run.clone();
        preview.reconciliation_required = false;
        let scope = preview.task_result_restore_scope(&task.id).unwrap();
        assert!(architect::sessions::digest(&scope));
        assert_eq!(serde_json::to_vec(&run).unwrap(), before);
        println!(
            "Derived an eligible read-only restoration scope from the original native prompt in a hypothetical reconciled clone; no approval or canonical state change."
        );
    }
    println!(
        "Validated {} source-bound native context facts and all sections; missing receipts/captures and changed hashes refused; no artifact acceptance or provider work.",
        output.context.as_ref().unwrap().model.facts.len()
    );
}
#[test]
#[ignore = "requires ANTISLOP_TASK_RECOVERY_PROJECTION_INPUT; native fixture copied into a disposable store; no provider work"]
fn unsupported_native_resume_cannot_record_approval_or_start_an_attempt() {
    use architect::{
        runtime::{Command, Runtime},
        storage::Store,
    };
    let path = std::env::var_os("ANTISLOP_TASK_RECOVERY_PROJECTION_INPUT")
        .expect("explicit owned fixture required");
    let run: architect::domain::Run =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let task_id = "task-contextanalyst";
    assert_eq!(run.status, architect::domain::Status::Blocked);
    assert!(!run.reconciliation_required);
    assert!(
        architect::runtime::team::resume::for_task(&run, task_id)
            .unwrap_err()
            .to_string()
            .contains("does not implement native exact-ID continuation")
    );
    let replacement = architect::runtime::team::replacement::for_task(&run, task_id).unwrap();
    replacement.validate_for_run(&run).unwrap();
    assert_eq!(
        replacement.handoff.version,
        architect::runtime::team::replacement::HANDOFF_VERSION
    );
    assert!(replacement.handoff.prior_turns.is_empty());
    assert!(!replacement.handoff.prior_turn_refs.is_empty());
    let mut sources = std::collections::BTreeMap::new();
    for reference in &replacement.handoff.prior_turn_refs {
        architect::context::history::extend(
            &run.session_ledger.turns[&reference.turn_id],
            reference,
            &mut sources,
        )
        .unwrap();
    }
    let scope = run.session_ledger.turns[&replacement.handoff.prior_turn_refs[0].turn_id]
        .context
        .as_ref()
        .expect("owned native fixture uses the pinned reader")
        .bundle
        .scope
        .clone();
    let bundle = architect::context::Bundle::new(scope, sources).unwrap();
    for reference in &replacement.handoff.prior_turn_refs {
        let restored = architect::context::history::restore(reference, &bundle.documents).unwrap();
        assert_eq!(
            serde_json::to_vec(&restored).unwrap(),
            serde_json::to_vec(&run.session_ledger.turns[&reference.turn_id]).unwrap()
        );
    }
    println!(
        "Reconstructed {} original native task turns byte-for-byte from the flat candidate archive; handoff={} UTF-8 bytes; no provider launch.",
        replacement.handoff.prior_turn_refs.len(),
        serde_json::to_vec(&replacement.handoff).unwrap().len()
    );
    let root = tempfile::tempdir().unwrap();
    let mut store = Store::open(&root.path().join("state.db")).unwrap();
    store.save(&run, "owned_read_only_copy", None).unwrap();
    let runtime = std::sync::Arc::new(Runtime::new(store).unwrap());
    let before = serde_json::to_vec(&runtime.store.lock().unwrap().get(&run.id).unwrap()).unwrap();
    let sequence = runtime.store.lock().unwrap().sequence().unwrap();
    // Even the former exact scope cannot authorize an unavailable transport.
    let scope_hash =
        architect::hash(serde_json::to_vec(&("native_task_resume_v1", task_id, &run)).unwrap());
    let result = runtime.dispatch(
        &architect::id(),
        Command::ResumeTaskSession {
            run_id: run.id.clone(),
            task_id: task_id.into(),
            scope_hash,
        },
    );
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("does not implement native exact-ID continuation")
    );
    let store = runtime.store.lock().unwrap();
    assert_eq!(store.sequence().unwrap(), sequence);
    assert_eq!(
        serde_json::to_vec(&store.get(&run.id).unwrap()).unwrap(),
        before
    );
    println!(
        "Owned native fixture retained both artifacts, all prior sessions and the scoped replacement handoff; unsupported exact-ID resume admitted no approval, attempt or native worker."
    );
}

#[test]
#[ignore = "requires ANTISLOP_MANAGEMENT_PROFILE_STATE; read-only daemon calls"]
fn explicit_backend_profile_reuses_authenticated_management_inputs() {
    use architect::tui::backend::{Backend, IpcBackend};
    use std::time::{Duration, Instant};
    let state = std::env::var_os("ANTISLOP_MANAGEMENT_PROFILE_STATE")
        .expect("explicit owned profile state required");
    let mut backend = IpcBackend::new(std::path::Path::new(&state));
    let runs = backend.list_runs().unwrap();
    assert_eq!(
        runs.len(),
        1,
        "owned profiling fixture must contain exactly one run"
    );
    let deadline = Instant::now();
    loop {
        let start = Instant::now();
        let snapshot = backend.snapshot(&runs[0].id).unwrap();
        println!(
            "Backend snapshot: {:.4}s; preparing={}; management_packages={}; task_replacements={}",
            start.elapsed().as_secs_f64(),
            snapshot.orchestration_recovery_preparing || snapshot.task_recovery_preparing,
            snapshot.orchestration_recoveries.len(),
            snapshot.task_session_replacements.len()
        );
        if !snapshot.orchestration_recovery_preparing && !snapshot.task_recovery_preparing {
            assert!(
                !snapshot.orchestration_recoveries.is_empty()
                    || !snapshot.task_session_replacements.is_empty()
            );
            break;
        }
        assert!(
            deadline.elapsed() < Duration::from_secs(120),
            "still checking; no mutation requested"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    for _ in 0..2 {
        let start = Instant::now();
        let snapshot = backend.snapshot(&runs[0].id).unwrap();
        assert!(
            !snapshot.orchestration_recoveries.is_empty()
                || !snapshot.task_session_replacements.is_empty()
        );
        println!(
            "Repeated authenticated backend snapshot: {:.4}s",
            start.elapsed().as_secs_f64()
        );
    }
    let start = Instant::now();
    let control = architect::ipc::call(
        std::path::Path::new(&state),
        &architect::ipc::Request {
            version: architect::ipc::PROTOCOL_VERSION,
            request_id: architect::id(),
            idempotency_key: None,
            operation: architect::ipc::Operation::Snapshot {
                run_id: runs[0].id.clone(),
            },
        },
    )
    .unwrap();
    assert!(control.error.is_none());
    println!(
        "Read-only control snapshot: {:.4}s",
        start.elapsed().as_secs_f64()
    );
    if control.data.unwrap()["task_session_replacement_count"]
        .as_u64()
        .is_some_and(|n| n > 0)
    {
        let start = Instant::now();
        let text = architect::ipc::read_document(
            std::path::Path::new(&state),
            &runs[0].id,
            "task-recovery",
        )
        .unwrap();
        println!(
            "Read-only complete task-recovery document: {:.4}s / {} bytes",
            start.elapsed().as_secs_f64(),
            text.len()
        );
    }
}

#[test]
#[ignore = "requires ANTISLOP_RECOVERY_PROJECTION_INPUT; no provider or mutation"]
fn retained_native_review_prefix_authenticates_with_its_original_prepared_inputs() {
    let path = std::env::var_os("ANTISLOP_RECOVERY_PROJECTION_INPUT")
        .expect("explicit retained fixture required");
    let bytes = std::fs::read(path).unwrap();
    let run: architect::domain::Run = serde_json::from_slice(&bytes).unwrap();
    let before = serde_json::to_vec(&run).unwrap();
    let mut legacy_handoffs = 0;
    if let Some(team) = &run.team {
        team.validate(&run.spec).unwrap();
        for replacement in team.tasks.iter().flat_map(|t| &t.native_replacements) {
            if replacement.handoff.version
                == architect::runtime::team::replacement::LEGACY_HANDOFF_VERSION
            {
                assert!(replacement.handoff.prior_turn_refs.is_empty());
                assert_eq!(
                    replacement.handoff.hash().unwrap(),
                    replacement.handoff_hash
                );
                legacy_handoffs += 1;
            }
        }
    }
    let round = run.orchestration_rounds.last().unwrap();
    let response = architect::transport::decode(round.output.as_ref().unwrap().as_bytes()).unwrap();
    let completed = round
        .review_parts
        .iter()
        .take_while(|p| p.output.is_some())
        .count();
    assert!(completed > 0, "fixture must contain real completed prefix");
    if completed == round.review_parts.len() {
        architect::orchestration::planning::authenticate(&run, round).unwrap();
    } else {
        let pending =
            architect::orchestration::planning::parts::recovery_part(&run, round, &response)
                .unwrap()
                .unwrap();
        assert_eq!(pending.id, round.review_parts[completed].id);
    }
    assert_eq!(serde_json::to_vec(&run).unwrap(), before);
    if let Some(context) = run
        .session_ledger
        .turn_order
        .last()
        .and_then(|id| run.session_ledger.turns.get(id))
        .and_then(|turn| turn.context.as_ref())
    {
        context.validate().unwrap();
        let (observed, total) = context.progress().unwrap();
        println!(
            "Latest retained transport {}: {observed}/{total} native-observed required UTF-8 bytes; complete={}; this is delivery evidence, not task acceptance.",
            context.version,
            context.complete().is_ok()
        );
    }
    println!(
        "Authenticated {completed} retained native reports against exact original prepared inputs; no provider launch or state write."
    );
    println!(
        "Retained {legacy_handoffs} exact legacy task handoff hashes without rewriting native inputs."
    );
}

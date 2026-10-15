use architect::{
    Result,
    config::RoleKind,
    domain::{Run, RunSpec},
    providers::{self, ObservedInvocation, ProviderEvent, Support},
    sessions::*,
};
use std::sync::{Arc, atomic::AtomicBool};

fn fixture() -> (tempfile::TempDir, Run, SessionRecord, TurnRecord) {
    let root = tempfile::tempdir().unwrap();
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().to_string_lossy().into();
    let run = Run::new(architect::id(), spec).unwrap();
    let provider = providers::build("fake").unwrap();
    let invocation = ObservedInvocation {
        turn_id: "turn-1",
        run: &run,
        resume: None,
        context_packet: None,
    };
    let request = provider.prepare_execute(&invocation).unwrap();
    let session = SessionRecord::new(
        "logical-1".into(),
        SessionScope::for_run(
            &run,
            "executor".into(),
            None,
            RoleKind::Executor,
            "fake".into(),
            "1".into(),
        )
        .unwrap(),
    )
    .unwrap();
    let turn = TurnRecord::new(
        "turn-1".into(),
        session.id.clone(),
        request.prompt_hash(),
        request.schema_hash().unwrap(),
        None,
    )
    .unwrap();
    (root, run, session, turn)
}
fn ok(_: &SessionLedger, _: &str) -> Result<()> {
    Ok(())
}

#[test]
fn incomplete_context_keeps_authenticated_rejected_output_without_completing_or_replaying_the_turn()
{
    let (_root, run, mut session, mut turn) = fixture();
    session.scope.provider = "agy".into();
    session.scope.binary_version = "1.3.5".into();
    session.scope.task_id = Some("owned-task".into());
    let schema = serde_json::json!({"type":"object"});
    let prompt = "Original prepared source requiring complete delivery";
    turn.prompt_hash = architect::hash(prompt);
    turn.schema_hash = architect::hash(serde_json::to_vec(&schema).unwrap());
    let context = architect::context::transport::Transport::new(&providers::TurnRequest {
        turn_id: &turn.id,
        run: &run,
        prompt,
        schema: &schema,
        resume: None,
    })
    .unwrap();
    let mut ledger = SessionLedger::default();
    ledger
        .reserve(session.clone(), turn.clone(), &mut ok)
        .unwrap();
    ledger
        .record_provider_event(
            &turn.id,
            &ProviderEvent::ContextPrepared {
                turn_id: turn.id.clone(),
                transport: Box::new(context),
            },
            &mut ok,
        )
        .unwrap();
    ledger
        .record_provider_event(
            &turn.id,
            &ProviderEvent::DeliveryPendingUnknown {
                turn_id: turn.id.clone(),
            },
            &mut ok,
        )
        .unwrap();
    ledger
        .record_provider_event(
            &turn.id,
            &ProviderEvent::DeliverySent {
                turn_id: turn.id.clone(),
            },
            &mut ok,
        )
        .unwrap();
    ledger
        .record_provider_event(
            &turn.id,
            &ProviderEvent::SessionObserved {
                session_id: architect::id(),
            },
            &mut ok,
        )
        .unwrap();
    let output = r#"{"findings":["Missing pinned source pages"],"agent_messages":[{"text":"PRIVATE_UNDELIVERED_OUTBOX"}]}"#;
    ledger
        .record_provider_event(
            &turn.id,
            &ProviderEvent::OutputRejected {
                turn_id: turn.id.clone(),
                output: output.into(),
            },
            &mut ok,
        )
        .unwrap();
    let before = serde_json::to_value(&ledger).unwrap();
    assert!(
        ledger
            .record_provider_event(
                &turn.id,
                &ProviderEvent::Completed {
                    turn_id: turn.id.clone()
                },
                &mut ok
            )
            .is_err()
    );
    assert_eq!(serde_json::to_value(&ledger).unwrap(), before);
    ledger
        .fail(&turn.id, providers::FailureKind::Protocol, false, &mut ok)
        .unwrap();
    let restored: SessionLedger =
        serde_json::from_value(serde_json::to_value(&ledger).unwrap()).unwrap();
    restored.validate_for_run(&run.id).unwrap();
    let rejected = restored.turns[&turn.id].rejected_output.as_ref().unwrap();
    assert_eq!(rejected.output, output);
    assert_eq!(rejected.hash, architect::hash(output));
    assert!(restored.turns[&turn.id].output.is_none());
    let mut report_run = run.clone();
    report_run.session_ledger = restored.clone();
    let reports =
        architect::context::transport::rejected_reports(&report_run, "orchestrator", None);
    assert_eq!(reports.len(), 1);
    assert_eq!(
        reports[0]["report_projection"]["findings"],
        serde_json::json!(["Missing pinned source pages"])
    );
    assert!(
        !serde_json::to_string(&reports)
            .unwrap()
            .contains("PRIVATE_UNDELIVERED_OUTBOX")
    );
    assert_eq!(reports[0]["original_output_hash"], architect::hash(output));
    assert_eq!(
        architect::context::transport::rejected_reports(
            &report_run,
            "task-reviewer",
            Some("owned-task")
        )
        .len(),
        1
    );
    assert!(
        architect::context::transport::rejected_reports(
            &report_run,
            "unrelated",
            Some("another-task")
        )
        .is_empty()
    );
    let mut changed = restored.clone();
    changed
        .turns
        .get_mut(&turn.id)
        .unwrap()
        .rejected_output
        .as_mut()
        .unwrap()
        .output
        .push('!');
    assert!(changed.validate().is_err());
    let mut next = session;
    next.id = "distinct-replacement".into();
    let next_turn = TurnRecord::new(
        "turn-2".into(),
        next.id.clone(),
        turn.prompt_hash,
        turn.schema_hash,
        None,
    )
    .unwrap();
    ledger.reserve(next, next_turn, &mut ok).unwrap();
    assert_eq!(ledger.turns[&turn.id], restored.turns[&turn.id]);
}

#[test]
fn observed_fake_output_is_durable_before_completion_is_visible() {
    let (_root, run, session, turn) = fixture();
    let mut ledger = SessionLedger::default();
    ledger
        .reserve(session.clone(), turn.clone(), &mut ok)
        .unwrap();
    ledger
        .reserve(session.clone(), turn.clone(), &mut ok)
        .unwrap();
    let mut changed = turn;
    changed.prompt_hash = architect::hash("changed");
    assert!(ledger.reserve(session, changed, &mut ok).is_err());
    let provider = providers::build("fake").unwrap();
    let receipt = provider
        .execute_observed(
            &ObservedInvocation {
                turn_id: "turn-1",
                run: &run,
                resume: None,
                context_packet: None,
            },
            Arc::new(AtomicBool::new(false)),
            &mut |event| ledger.record_provider_event("turn-1", event, &mut ok),
        )
        .unwrap();
    assert_eq!(ledger.turns["turn-1"].state, TurnState::Sent);
    assert!(ledger.turns["turn-1"].provider_completed);
    let reference = OutputRef {
        document: "turn:turn-1".into(),
        hash: architect::hash(&receipt.output),
    };
    let prior = ledger.clone();
    let fail =
        &mut |_: &SessionLedger, _: &str| Err(architect::Error::Invalid("commit failed".into()));
    assert!(
        ledger
            .complete("turn-1", &receipt, reference.clone(), fail)
            .is_err()
    );
    assert_eq!(ledger, prior);
    ledger
        .complete("turn-1", &receipt, reference, &mut ok)
        .unwrap();
    assert_eq!(ledger.turns["turn-1"].state, TurnState::Completed);
    let restored: SessionLedger =
        serde_json::from_slice(&serde_json::to_vec(&ledger).unwrap()).unwrap();
    restored.validate().unwrap();
    assert_eq!(
        restored.turns["turn-1"].output.as_deref(),
        Some(receipt.output.as_str())
    );
}

#[test]
fn callback_commit_failure_prevents_provider_progress() {
    let (_root, run, session, turn) = fixture();
    let mut ledger = SessionLedger::default();
    ledger.reserve(session, turn, &mut ok).unwrap();
    let before = ledger.clone();
    let mut calls = 0;
    let result = providers::build("fake").unwrap().execute_observed(
        &ObservedInvocation {
            turn_id: "turn-1",
            run: &run,
            resume: None,
            context_packet: None,
        },
        Arc::new(AtomicBool::new(false)),
        &mut |event| {
            calls += 1;
            ledger.record_provider_event("turn-1", event, &mut |_, _| {
                Err(architect::Error::Invalid("database unavailable".into()))
            })
        },
    );
    assert!(result.is_err());
    assert_eq!(calls, 1);
    assert_eq!(ledger, before);
}

#[test]
fn reservation_order_is_durable_idempotent_and_does_not_invent_legacy_chronology() {
    let (_root, _run, mut session, mut turn) = fixture();
    let mut ledger = SessionLedger::default();
    for (logical, turn_id) in [("z-old", "z-turn"), ("a-new", "a-turn")] {
        session.id = logical.into();
        turn.id = turn_id.into();
        turn.session_id = logical.into();
        ledger
            .reserve(session.clone(), turn.clone(), &mut ok)
            .unwrap();
    }
    assert_eq!(ledger.turn_order, ["z-turn", "a-turn"]);
    ledger
        .reserve(session.clone(), turn.clone(), &mut ok)
        .unwrap();
    assert_eq!(ledger.turn_order, ["z-turn", "a-turn"]);
    let mut encoded = serde_json::to_value(&ledger).unwrap();
    encoded.as_object_mut().unwrap().remove("turn_order");
    let mut legacy: SessionLedger = serde_json::from_value(encoded).unwrap();
    legacy.validate().unwrap();
    assert!(legacy.turn_order.is_empty());
    session.id = "replacement".into();
    turn.id = "replacement-turn".into();
    turn.session_id = session.id.clone();
    let before = legacy.clone();
    assert!(
        legacy
            .reserve(session.clone(), turn.clone(), &mut |_, _| Err(
                architect::Error::Invalid("commit failed".into())
            ))
            .is_err()
    );
    assert_eq!(legacy, before);
    legacy.reserve(session, turn, &mut ok).unwrap();
    assert_eq!(legacy.turn_order, ["replacement-turn"]);
    let restored: SessionLedger =
        serde_json::from_str(&serde_json::to_string(&legacy).unwrap()).unwrap();
    assert_eq!(restored, legacy);
    for order in [
        vec!["missing".into()],
        vec!["replacement-turn".into(), "replacement-turn".into()],
    ] {
        let mut corrupt = restored.clone();
        corrupt.turn_order = order;
        assert!(corrupt.validate().is_err());
    }
}

#[test]
fn recovery_requires_observed_native_id_reconciliation_scope_and_authorization() {
    let (_root, _run, session, turn) = fixture();
    let mut ledger = SessionLedger::default();
    ledger.reserve(session, turn, &mut ok).unwrap();
    ledger
        .record_provider_event(
            "turn-1",
            &ProviderEvent::DeliveryPendingUnknown {
                turn_id: "turn-1".into(),
            },
            &mut ok,
        )
        .unwrap();
    ledger.interrupted(&mut ok).unwrap();
    let session = &ledger.sessions["logical-1"];
    let turn = &ledger.turns["turn-1"];
    assert!(matches!(
        recovery_decision(
            session,
            turn,
            &session.scope,
            Support::Supported,
            true,
            true
        ),
        RecoveryDecision::ReconcileUnknown { .. }
    ));
    let mut observed = session.clone();
    observed.native_session_id = Some("00000000-0000-4000-8000-000000000001".into());
    observed.native_observed = true;
    assert!(matches!(
        recovery_decision(
            &observed,
            turn,
            &observed.scope,
            Support::Supported,
            false,
            true
        ),
        RecoveryDecision::ReconcileUnknown { .. }
    ));
    assert_eq!(
        recovery_decision(
            &observed,
            turn,
            &observed.scope,
            Support::Supported,
            true,
            false
        ),
        RecoveryDecision::AwaitAuthorization
    );
    assert!(matches!(
        recovery_decision(
            &observed,
            turn,
            &observed.scope,
            Support::Unverified,
            true,
            true
        ),
        RecoveryDecision::ReplacementRequired { .. }
    ));
    let mut changed = observed.scope.clone();
    changed.policy_revision += 1;
    assert!(matches!(
        recovery_decision(&observed, turn, &changed, Support::Supported, true, true),
        RecoveryDecision::ReplacementRequired { .. }
    ));
    assert_eq!(
        recovery_decision(
            &observed,
            turn,
            &observed.scope,
            Support::Supported,
            true,
            true
        ),
        RecoveryDecision::ResumeExact {
            native_session_id: observed.native_session_id.unwrap()
        }
    );
}

#[test]
fn exact_resume_reserves_a_new_turn_without_rewriting_unknown_history_and_commit_failure_is_atomic()
{
    let (_root, _run, session, turn) = fixture();
    let mut ledger = SessionLedger::default();
    ledger.reserve(session, turn, &mut ok).unwrap();
    ledger
        .record_provider_event(
            "turn-1",
            &ProviderEvent::SessionObserved {
                session_id: "10000000-0000-4000-8000-000000000001".into(),
            },
            &mut ok,
        )
        .unwrap();
    ledger.interrupted(&mut ok).unwrap();
    let before = ledger.clone();
    let session = ledger.sessions["logical-1"].clone();
    let next = TurnRecord::new(
        "continued-turn".into(),
        session.id.clone(),
        architect::hash("current source-bound checkpoint"),
        architect::hash("same task schema"),
        None,
    )
    .unwrap();
    assert!(
        ledger
            .reserve(session.clone(), next.clone(), &mut ok)
            .is_err(),
        "ordinary reservation must not clear unknown delivery"
    );
    assert!(
        ledger
            .reserve_resumed(session.clone(), next.clone(), "wrong-checkpoint", &mut ok)
            .is_err()
    );
    assert!(
        ledger
            .reserve_resumed(
                session.clone(),
                ledger.turns["turn-1"].clone(),
                "turn-1",
                &mut ok
            )
            .is_err(),
        "cannot overwrite the old turn"
    );
    let fail = &mut |_: &SessionLedger, _: &str| {
        Err(architect::Error::Invalid("resume commit failed".into()))
    };
    assert!(
        ledger
            .reserve_resumed(session.clone(), next.clone(), "turn-1", fail)
            .is_err()
    );
    assert_eq!(ledger, before);
    ledger
        .reserve_resumed(session, next, "turn-1", &mut ok)
        .unwrap();
    assert_eq!(ledger.turns["turn-1"], before.turns["turn-1"]);
    assert_eq!(ledger.turns["turn-1"].state, TurnState::DeliveryUnknown);
    assert_eq!(ledger.sessions.len(), 1);
    assert_eq!(
        ledger.sessions["logical-1"].current_turn_id.as_deref(),
        Some("continued-turn")
    );
    assert_eq!(ledger.turn_order, vec!["turn-1", "continued-turn"]);
}

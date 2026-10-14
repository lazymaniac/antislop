use architect::{Result, communication::*, providers::Support, sessions::OutputRef};
fn ok(_: &CommunicationLedger, _: &str) -> Result<()> {
    Ok(())
}
fn message(id: &str, body: &str) -> MessageRecord {
    MessageRecord::new(
        id.into(),
        Sender::User,
        "orchestrator".into(),
        None,
        MessageKind::UserInstruction,
        OutputRef {
            document: format!("message:{id}"),
            hash: architect::hash(body),
        },
    )
    .unwrap()
}

#[test]
fn accepted_history_is_immutable_and_unsupported_steering_queues_next_turn() {
    let mut ledger = CommunicationLedger::default();
    let first = message("m1", "keep scope");
    ledger.accept(first.clone(), &mut ok).unwrap();
    ledger.accept(first, &mut ok).unwrap();
    assert!(
        ledger
            .accept(message("m1", "change scope"), &mut ok)
            .is_err()
    );
    assert_eq!(
        ledger
            .plan("m1", DeliveryMode::MidTurn, Support::Unsupported, &mut ok)
            .unwrap(),
        DeliveryPlan::QueuedNextTurn
    );
    assert_eq!(
        ledger
            .plan(
                "m1",
                DeliveryMode::InterruptAndResume,
                Support::Supported,
                &mut ok
            )
            .unwrap(),
        DeliveryPlan::RequiresExplicitInterrupt
    );
    let packet = ledger
        .context_packet(
            "orchestrator",
            None,
            "turn-1",
            &architect::hash("scope"),
            &mut |_| Ok("keep scope".into()),
        )
        .unwrap();
    let prompt = packet.append_to("original immutable prompt").unwrap();
    assert!(prompt.starts_with("original immutable prompt"));
    assert!(prompt.contains("does not grant permissions"));
    ledger
        .sent("m1", "logical-session", "turn-1", &mut ok)
        .unwrap();
    ledger.interrupted(&mut ok).unwrap();
    assert_eq!(ledger.messages["m1"].state, DeliveryState::Unknown);
    assert!(
        ledger
            .sent("m1", "logical-session", "turn-2", &mut ok)
            .is_err()
    );
    assert_eq!(ledger.history.len(), 1);
    ledger.validate().unwrap();
}

#[test]
fn failed_commit_and_bad_content_proof_do_not_advance_delivery() {
    let mut ledger = CommunicationLedger::default();
    ledger.accept(message("m1", "body"), &mut ok).unwrap();
    let before = ledger.clone();
    assert!(
        ledger
            .sent("m1", "session", "turn", &mut |_, _| Err(
                architect::Error::Invalid("commit failed".into())
            ))
            .is_err()
    );
    assert_eq!(ledger, before);
    assert!(
        ledger
            .context_packet(
                "orchestrator",
                None,
                "turn",
                &architect::hash("scope"),
                &mut |_| Ok("different body".into())
            )
            .is_err()
    );
    ledger.sent("m1", "session", "turn", &mut ok).unwrap();
    assert!(
        ledger
            .delivered("m1", "wrong-session", "turn", &mut ok)
            .is_err()
    );
    ledger.delivered("m1", "session", "turn", &mut ok).unwrap();
    ledger
        .acknowledge(
            "m1",
            "session",
            "turn",
            OutputRef {
                document: "ack:1".into(),
                hash: architect::hash("acknowledged"),
            },
            &mut ok,
        )
        .unwrap();
    assert_eq!(ledger.messages["m1"].state, DeliveryState::Acknowledged);
    assert!(ledger.cancel("m1", &mut ok).is_err());
}

#[test]
fn delivery_intent_needs_an_exact_send_receipt_and_uncertainty_never_replays() {
    let mut ledger = CommunicationLedger::default();
    ledger.accept(message("m1", "body"), &mut ok).unwrap();
    ledger
        .reserve_delivery("m1", "session", "turn", &mut ok)
        .unwrap();
    assert_eq!(
        ledger.messages["m1"].state,
        DeliveryState::DeliveryPendingUnknown
    );
    assert!(ledger.sent("m1", "other-session", "turn", &mut ok).is_err());
    assert!(ledger.sent("m1", "session", "other-turn", &mut ok).is_err());
    assert!(ledger.delivered("m1", "session", "turn", &mut ok).is_err());
    assert!(
        ledger
            .reserve_delivery("m1", "session", "replacement", &mut ok)
            .is_err()
    );
    ledger.sent("m1", "session", "turn", &mut ok).unwrap();
    ledger
        .delivery_unknown("m1", "session", "turn", &mut ok)
        .unwrap();
    assert!(ledger.sent("m1", "session", "turn", &mut ok).is_err());
    let packet = ledger
        .context_packet(
            "orchestrator",
            None,
            "replacement",
            &architect::hash("scope"),
            &mut |_| Ok("body".into()),
        )
        .unwrap();
    assert!(packet.messages.is_empty());
    ledger.validate().unwrap();
}

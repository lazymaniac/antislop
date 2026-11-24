use super::*;

pub(super) fn finish(
    core: &Core,
    outcome: Result<agent::Outcome>,
    events: TurnEvents<'_>,
    cancel: &AtomicBool,
) -> Result<Option<String>> {
    match outcome {
        Ok(outcome) => finish_success(core, outcome, events, cancel),
        Err(error) => finish_failure(core, error, events, cancel),
    }
}

fn finish_success(
    core: &Core,
    outcome: agent::Outcome,
    events: TurnEvents<'_>,
    cancel: &AtomicBool,
) -> Result<Option<String>> {
    let turn = &events.turn;
    if let Some(session) = &outcome.session
        && let Err(error) = core.store.set_session(&turn.id, session)
    {
        // A rejected provider binding is an execution failure, not a turn
        // still running after its worker has already left.
        return finish_failure(core, error, events, cancel);
    }
    events.announce("agent_final", json!({"text": outcome.text}));
    if let Some(error) = events.persistence_error.borrow_mut().take() {
        core.store
            .finish_turn(&turn.id, TurnStatus::Failed, Some(&error.to_string()))?;
        return Err(error);
    }
    core.store.finish_turn(&turn.id, TurnStatus::Done, None)?;
    let delivered = events.inbox.iter().map(|mail| mail.id).collect::<Vec<_>>();
    core.store
        .mark_delivered(&events.conversation, &delivered, &turn.id)?;
    events.announce(
        "turn_finished",
        json!({
            "role": turn.role,
            "status": "done",
            "prompt_bytes": events.prompt_bytes.get(),
            "answer_bytes": outcome.text.len(),
        }),
    );
    Ok(Some(outcome.text))
}

fn finish_failure(
    core: &Core,
    error: Error,
    events: TurnEvents<'_>,
    cancel: &AtomicBool,
) -> Result<Option<String>> {
    let status = if cancel.load(Ordering::SeqCst) {
        TurnStatus::Cancelled
    } else {
        TurnStatus::Failed
    };
    let error = agent::persisted_error(&error.to_string());
    core.store
        .finish_turn(&events.turn.id, status, Some(&error))?;
    record_failed_mail(&events, &error);
    let steered = core.store.requeue_steered(&events.turn.id)?;
    if !steered.is_empty() {
        events.announce(
            "mail_delivery_failed",
            json!({
                "recipient": events.turn.role,
                "ids": steered,
                "reason": "typed into a turn that ended without a result; it goes again with the next turn",
                "state": "queued",
            }),
        );
    }
    events.announce(
        "turn_finished",
        json!({
            "role": events.turn.role,
            "status": status,
            "error": error,
            "prompt_bytes": events.prompt_bytes.get(),
        }),
    );
    Ok(None)
}

fn record_failed_mail(events: &TurnEvents<'_>, error: &str) {
    let ids = events
        .inbox
        .iter()
        .filter(|mail| mail.kind == "agent_work")
        .map(|mail| mail.id)
        .collect::<Vec<_>>();
    if !ids.is_empty() {
        events.announce(
            "mail_delivery_failed",
            json!({"recipient":events.turn.role,"ids":ids,"reason":error,"state":"queued"}),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conflicting_pi_result_identity_fails_the_turn_without_rebinding_history() {
        let (directory, core, conversation) = crate::lean::agent_tools::tests::core();
        let turn = core
            .store
            .start_turn(&conversation, PLAN_REVIEWER, "pi", 0, None)
            .unwrap();
        core.store
            .record_run_context(&store::RunContext {
                turn_id: turn.id.clone(),
                conversation: conversation.clone(),
                role: PLAN_REVIEWER.into(),
                provider: "pi".into(),
                profile: "low".into(),
                cwd: directory.path().into(),
                add_dirs: vec![],
                provider_thread_id: None,
                provider_session_id: Some("original.jsonl".into()),
            })
            .unwrap();
        core.store.set_session(&turn.id, "original.jsonl").unwrap();
        let events = TurnEvents {
            store: &core.store,
            conversation: conversation.clone(),
            turn: turn.clone(),
            inbox: vec![],
            persistence_error: std::cell::RefCell::new(None),
            prompt_bytes: std::cell::Cell::new(None),
        };
        assert!(
            finish(
                &core,
                Ok(agent::Outcome {
                    session: Some("different.jsonl".into()),
                    text: "not accepted".into()
                }),
                events,
                &AtomicBool::new(false)
            )
            .unwrap()
            .is_none()
        );
        let saved = core
            .store
            .last_turn(&conversation, PLAN_REVIEWER)
            .unwrap()
            .unwrap();
        assert_eq!(saved.status, TurnStatus::Failed);
        assert_eq!(
            saved.error.as_deref(),
            Some("run is already bound to another provider session")
        );
        assert_eq!(saved.session_id.as_deref(), Some("original.jsonl"));
        assert_eq!(
            core.store
                .run_context(&conversation, &turn.id)
                .unwrap()
                .unwrap()
                .provider_session_id
                .as_deref(),
            Some("original.jsonl")
        );
        let events = core.store.events_since(&conversation, 0, 100).unwrap();
        assert!(
            events
                .iter()
                .any(|event| event.kind == "turn_finished" && event.payload["status"] == "failed")
        );
        assert!(!events.iter().any(|event| event.kind == "agent_final"));
    }
}

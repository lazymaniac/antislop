use super::*;

pub(super) fn finish(
    core: &Core,
    outcome: Result<agent::Outcome>,
    events: TurnEvents<'_>,
    cancel: &AtomicBool,
) -> Result<Option<String>> {
    match outcome {
        Ok(outcome) => finish_success(core, outcome, events),
        Err(error) => finish_failure(core, error, events, cancel),
    }
}

fn finish_success(
    core: &Core,
    outcome: agent::Outcome,
    events: TurnEvents<'_>,
) -> Result<Option<String>> {
    let turn = &events.turn;
    if let Some(session) = &outcome.session {
        core.store.set_session(&turn.id, session)?;
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
        json!({"role": turn.role, "status": "done"}),
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
    events.announce(
        "turn_finished",
        json!({"role": events.turn.role, "status": status, "error": error}),
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

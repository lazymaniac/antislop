//! Bind the existing communication ledger to actual runtime turns. Queue and
//! delivery changes share the same SQLite transaction as their run/turn event.
use super::Runtime;
use crate::{
    Result,
    communication::{
        ContextPacket, DeliveryMode, DeliveryState, MessageKind, MessageRecord, Sender,
    },
    domain::{Run, Stage, Status},
    hash, id,
    providers::Support,
    require,
    sessions::{OutputRef, TurnState},
};

pub(super) fn agents(run: &Run) -> std::collections::BTreeSet<String> {
    let reviewers = run
        .diff_reviews
        .iter()
        .map(|review| format!("diff-review:{}", review.id));
    if let Some(team) = &run.team {
        return team
            .agents
            .iter()
            .map(|agent| agent.id.clone())
            .chain(reviewers)
            .collect();
    }
    let mut agents: std::collections::BTreeSet<_> = [
        Stage::Discovery,
        Stage::Planning,
        Stage::Execution,
        Stage::Validation,
        Stage::Integration,
        Stage::Delivery,
    ]
    .into_iter()
    .flat_map(|stage| {
        ["executor", "verifier"]
            .into_iter()
            .map(move |role| format!("{stage:?}:{role}"))
    })
    .collect();
    if run.spec.config.as_ref().is_some_and(|pinned| {
        pinned
            .config
            .validate_work_kinds(&std::collections::BTreeSet::from([
                crate::config::WorkKind::Orchestration,
            ]))
            .is_ok()
    }) {
        agents.extend(["orchestrator".into(), "orchestrator:verifier".into()]);
    }
    agents.extend(reviewers);
    agents
}
fn validate_text(content: &str) -> Result<()> {
    require(
        !content.trim().is_empty()
            && !content.contains('\0')
            && content.len() < crate::transport::MAX_FRAME,
        "invalid message text",
    )
}
fn queue(
    run: &mut Run,
    message_id: String,
    sender: Sender,
    recipient: String,
    kind: MessageKind,
    content: &str,
) -> Result<()> {
    validate_text(content)?;
    let message = MessageRecord::new(
        message_id.clone(),
        sender,
        recipient,
        None,
        kind,
        OutputRef {
            document: format!("message:{message_id}"),
            hash: hash(content),
        },
    )?;
    run.communication_ledger
        .accept(message, &mut |_, _| Ok(()))?;
    run.communication_ledger.plan(
        &message_id,
        DeliveryMode::NextTurn,
        Support::Unsupported,
        &mut |_, _| Ok(()),
    )?;
    run.message_bodies.insert(message_id, content.into());
    Ok(())
}
pub(super) fn accept(run: &mut Run, recipient: &str, content: &str) -> Result<()> {
    validate_text(content)?;
    let agents = agents(run);
    require(
        recipient == "*" || agents.contains(recipient),
        "unknown message recipient",
    )?;
    let recipients: Vec<_> = if recipient == "*" {
        agents.into_iter().collect()
    } else {
        vec![recipient.to_owned()]
    };
    for recipient in recipients {
        queue(
            run,
            id(),
            Sender::User,
            recipient,
            MessageKind::UserInstruction,
            content,
        )?;
    }
    Ok(())
}
pub(super) fn runtime_status(run: &mut Run, recipient: &str, content: &str) -> Result<()> {
    require(
        agents(run).contains(recipient),
        "unknown runtime status recipient",
    )?;
    queue(
        run,
        id(),
        Sender::Runtime,
        recipient.into(),
        MessageKind::Status,
        content,
    )
}

pub(super) fn retire_recipients(run: &mut Run) -> Result<()> {
    let known = agents(run);
    let retired: Vec<_> = run
        .communication_ledger
        .messages
        .values()
        .filter(|message| {
            !known.contains(&message.recipient_agent_id)
                && matches!(
                    message.state,
                    DeliveryState::AcceptedRuntime | DeliveryState::QueuedNextTurn
                )
        })
        .map(|message| message.id.clone())
        .collect();
    for id in retired {
        run.communication_ledger.cancel(&id, &mut |_, _| Ok(()))?;
    }
    Ok(())
}

/// Runs in the SAME transaction as output completion. Invalid outboxes retain
/// the raw completed output and a rejection, without admitting a partial batch.
/// Identity is derived exclusively from the observed session/turn, not JSON.
pub(super) fn record_outbox(run: &mut Run, session_id: &str) -> Result<()> {
    use crate::communication::outbox::{Message, Receipt, Target};
    let session = run
        .session_ledger
        .sessions
        .get(session_id)
        .ok_or_else(|| crate::Error::Invalid("outbox author session missing".into()))?;
    let turn = session
        .current_turn_id
        .as_ref()
        .and_then(|id| run.session_ledger.turns.get(id))
        .ok_or_else(|| crate::Error::Invalid("outbox author turn missing".into()))?;
    let Some(raw) = &turn.output else {
        return Ok(());
    };
    // Non-JSON/malformed reports are still preserved for the work contract's
    // normal rejection path. They cannot issue messages through text matching.
    let Ok(value) = crate::transport::decode::<serde_json::Value>(raw.as_bytes()) else {
        return Ok(());
    };
    let Some(outbox) = value.get("agent_messages") else {
        return Ok(());
    };
    let output_hash = hash(raw);
    let turn_id = turn.id.clone();
    if let Some(saved) = run.agent_outboxes.get(&turn_id) {
        return require(saved.output_hash == output_hash, "outbox output changed");
    }
    let sender = session.scope.agent_id.clone();
    let mut candidate = run.clone();
    let result = (|| -> Result<Vec<String>> {
        require(
            run.status != Status::Cancelled,
            "run cancelled before outbox admission",
        )?;
        require(
            session.scope.spec_revision == run.spec_revision
                && session.native_observed
                && session.state == TurnState::Completed
                && turn.state == TurnState::Completed
                && turn.provider_completed
                && turn.output_ref.as_ref().is_some_and(|reference| {
                    reference.hash == output_hash && reference.document == format!("turn:{turn_id}")
                }),
            "outbox has no authenticated completed author",
        )?;
        let agents = agents(run);
        require(
            agents.contains(&sender),
            "outbox author is not a current run agent",
        )?;
        let messages: Vec<Message> = serde_json::from_value(outbox.clone())?;
        let mut ids = vec![];
        for (index, message) in messages.into_iter().enumerate() {
            require(
                message.kind != MessageKind::UserInstruction,
                "agent cannot impersonate a user instruction",
            )?;
            let recipients: Vec<_> = match message.target {
                Target::Agent { agent_id } => {
                    require(
                        agents.contains(&agent_id),
                        "unknown agent message recipient",
                    )?;
                    vec![agent_id]
                }
                Target::Broadcast => agents
                    .iter()
                    .filter(|agent| **agent != sender)
                    .cloned()
                    .collect(),
            };
            for recipient in recipients {
                let message_id = format!(
                    "agent:{}",
                    hash(serde_json::to_vec(&(&turn_id, index, &recipient))?)
                );
                queue(
                    &mut candidate,
                    message_id.clone(),
                    Sender::Agent {
                        agent_id: sender.clone(),
                    },
                    recipient,
                    message.kind,
                    &message.text,
                )?;
                ids.push(message_id);
            }
        }
        Ok(ids)
    })();
    let receipt = match result {
        Ok(message_ids) => {
            run.communication_ledger = candidate.communication_ledger;
            run.message_bodies = candidate.message_bodies;
            Receipt {
                output_hash,
                message_ids,
                rejection: None,
            }
        }
        Err(error) => Receipt {
            output_hash,
            message_ids: vec![],
            rejection: Some(error.to_string()),
        },
    };
    run.agent_outboxes.insert(turn_id, receipt);
    Ok(())
}

impl Runtime {
    pub(super) fn message_packet(
        &self,
        run: &Run,
        view: &Run,
        recipient: &str,
        turn: &str,
    ) -> Result<Option<ContextPacket>> {
        let store = self.store.lock().expect("store mutex");
        let current = store.get(&run.id)?;
        require(
            current.spec_revision == run.spec_revision
                && current.attempt == run.attempt
                && current.status != Status::Cancelled,
            "message scope is no longer active",
        )?;
        let packet = current.communication_ledger.context_packet(
            recipient,
            None,
            turn,
            &view.scope_hash("message-context"),
            &mut |reference| {
                let message_id = reference.document.strip_prefix("message:").ok_or_else(|| {
                    crate::Error::Invalid("invalid message body reference".into())
                })?;
                current
                    .message_bodies
                    .get(message_id)
                    .cloned()
                    .ok_or_else(|| crate::Error::Invalid("message body is missing".into()))
            },
        )?;
        Ok((!packet.messages.is_empty()).then_some(packet))
    }
}

pub(super) fn checkpoint(
    run: &mut Run,
    session: &str,
    packet: &ContextPacket,
    kind: &str,
) -> Result<()> {
    packet.validate()?;
    let turn = run
        .session_ledger
        .turns
        .get(&packet.turn_id)
        .ok_or_else(|| crate::Error::Invalid("message target turn is missing".into()))?;
    let scope = &run
        .session_ledger
        .sessions
        .get(session)
        .ok_or_else(|| crate::Error::Invalid("message target session is missing".into()))?
        .scope;
    require(
        turn.session_id == session
            && turn.context_packet_hash.as_ref() == Some(&packet.hash()?)
            && scope.agent_id == packet.recipient_agent_id
            && (packet.task_id.is_none() || scope.task_id == packet.task_id),
        "message packet does not match the reserved turn/session",
    )?;
    let state = turn.state;
    if kind == "turn_reserved" {
        require(
            !run.message_packets.contains_key(&packet.turn_id),
            "message packet already reserved",
        )?;
        for entry in &packet.messages {
            let message = run
                .communication_ledger
                .messages
                .get(&entry.message_id)
                .ok_or_else(|| crate::Error::Invalid("queued message is missing".into()))?;
            require(
                message.recipient_agent_id == packet.recipient_agent_id
                    && message.task_id == packet.task_id
                    && message.sender == entry.sender
                    && message.sequence == entry.sequence
                    && message.content_ref == entry.content_ref
                    && run.message_bodies.get(&entry.message_id) == Some(&entry.content),
                "queued message changed before delivery reservation",
            )?;
            run.communication_ledger.reserve_delivery(
                &entry.message_id,
                session,
                &packet.turn_id,
                &mut |_, _| Ok(()),
            )?;
        }
        run.message_packets
            .insert(packet.turn_id.clone(), packet.clone());
    } else {
        require(
            run.message_packets.get(&packet.turn_id) == Some(packet),
            "reserved message packet changed",
        )?;
        for entry in &packet.messages {
            let message = run
                .communication_ledger
                .messages
                .get(&entry.message_id)
                .ok_or_else(|| crate::Error::Invalid("reserved message is missing".into()))?;
            match (state, message.state) {
                (TurnState::Sent, DeliveryState::DeliveryPendingUnknown) => {
                    run.communication_ledger.sent(
                        &entry.message_id,
                        session,
                        &packet.turn_id,
                        &mut |_, _| Ok(()),
                    )?;
                }
                (TurnState::Completed, DeliveryState::Sent) => {
                    run.communication_ledger.delivered(
                        &entry.message_id,
                        session,
                        &packet.turn_id,
                        &mut |_, _| Ok(()),
                    )?;
                }
                (
                    TurnState::Failed | TurnState::Stopped | TurnState::DeliveryUnknown,
                    DeliveryState::DeliveryPendingUnknown | DeliveryState::Sent,
                ) => {
                    run.communication_ledger.delivery_unknown(
                        &entry.message_id,
                        session,
                        &packet.turn_id,
                        &mut |_, _| Ok(()),
                    )?;
                }
                _ => (),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::RoleKind,
        runtime::Command,
        sessions::{SessionRecord, SessionScope, TurnRecord},
        storage::Store,
    };
    use std::sync::Arc;

    #[test]
    fn reservation_is_atomic_and_crash_uncertainty_never_requeues_a_message() {
        let root = tempfile::tempdir().unwrap();
        let db = root.path().join("state.db");
        let runtime = Arc::new(Runtime::new(Store::open(&db).unwrap()).unwrap());
        let mut spec: crate::domain::RunSpec =
            serde_json::from_str(include_str!("../../examples/fake-spec.json")).unwrap();
        spec.workspace = root.path().to_string_lossy().into();
        let mut run = runtime.dispatch(&id(), Command::Create { spec }).unwrap();
        run = runtime
            .dispatch(
                &id(),
                Command::SendMessage {
                    run_id: run.id.clone(),
                    target_agent_id: "Discovery:executor".into(),
                    text: "first".into(),
                },
            )
            .unwrap();
        // Model the persisted boundary before an actual process is launched.
        run.status = Status::Running;
        runtime
            .store
            .lock()
            .unwrap()
            .save(&run, "test_boundary", None)
            .unwrap();
        let packet = runtime
            .message_packet(&run, &run, "Discovery:executor", "turn")
            .unwrap()
            .unwrap();
        let later = runtime
            .dispatch(
                &id(),
                Command::SendMessage {
                    run_id: run.id.clone(),
                    target_agent_id: "Discovery:executor".into(),
                    text: "after packet cutoff".into(),
                },
            )
            .unwrap();
        let scope = SessionScope::for_run(
            &run,
            "Discovery:executor".into(),
            None,
            RoleKind::Executor,
            "fake".into(),
            "1".into(),
        )
        .unwrap();
        let session = SessionRecord::new("session".into(), scope).unwrap();
        let turn = TurnRecord::new(
            "turn".into(),
            "session".into(),
            hash("prompt"),
            hash("schema"),
            Some(packet.hash().unwrap()),
        )
        .unwrap();
        let connection = rusqlite::Connection::open(&db).unwrap();
        connection.execute_batch("CREATE TRIGGER refuse_checkpoint BEFORE UPDATE ON runs BEGIN SELECT RAISE(ABORT, 'injected commit failure'); END;").unwrap();
        assert!(
            runtime
                .checkpoint_session(&run, "session", Some(&packet), true, |ledger, persist| {
                    ledger.reserve(session.clone(), turn.clone(), persist)
                })
                .is_err()
        );
        let unchanged = runtime.store.lock().unwrap().get(&run.id).unwrap();
        assert_eq!(unchanged.session_ledger, run.session_ledger);
        assert_eq!(unchanged.communication_ledger, later.communication_ledger);
        assert!(unchanged.message_packets.is_empty());
        connection
            .execute_batch("DROP TRIGGER refuse_checkpoint;")
            .unwrap();
        runtime
            .checkpoint_session(&run, "session", Some(&packet), true, |ledger, persist| {
                ledger.reserve(session, turn, persist)
            })
            .unwrap();
        let reserved = runtime.store.lock().unwrap().get(&run.id).unwrap();
        assert_eq!(reserved.message_packets["turn"], packet);
        let first = &reserved.communication_ledger.messages[&packet.messages[0].message_id];
        assert_eq!(first.state, DeliveryState::DeliveryPendingUnknown);
        assert!(
            reserved
                .communication_ledger
                .messages
                .values()
                .any(|m| m.state == DeliveryState::QueuedNextTurn)
        );
        drop(connection);
        drop(runtime);
        let recovered = Runtime::new(Store::open(&db).unwrap()).unwrap();
        let run = recovered.store.lock().unwrap().get(&run.id).unwrap();
        assert!(run.reconciliation_required);
        assert_eq!(
            run.communication_ledger.messages[&packet.messages[0].message_id].state,
            DeliveryState::Unknown
        );
        let next = recovered
            .message_packet(&run, &run, "Discovery:executor", "replacement")
            .unwrap()
            .unwrap();
        assert_eq!(next.messages.len(), 1);
        assert_eq!(next.messages[0].content, "after packet cutoff");
        assert_ne!(next.messages[0].message_id, packet.messages[0].message_id);
    }
}

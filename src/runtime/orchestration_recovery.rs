//! Read-only management recovery: exact resume, distinct replacement, or retained output.
use crate::{
    Result,
    config::{RoleKind, WorkKind},
    domain::*,
    hash, id,
    orchestration::planning::{self, Round, RoundStatus},
    require,
    sessions::{SessionRecord, SessionScope, TurnRecord, TurnState},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Resume,
    Replacement,
    Restore,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pending {
    pub round_id: String,
    pub kind: Kind,
    pub role: RoleKind,
    pub previous_session: Option<String>,
    pub previous_turn: Option<String>,
    pub native_session_id: Option<String>,
    pub retained_executor: Option<String>,
    pub retained_output: Option<String>,
    pub retained_review: Option<String>,
    pub handoff: serde_json::Value,
    pub handoff_hash: String,
    pub scope_hash: String,
}
impl Pending {
    pub fn validate_for_run(&self, run: &Run) -> Result<()> {
        let current = for_round(run, &self.round_id, self.kind)?;
        require(
            serde_json::to_value(self)? == serde_json::to_value(current)?,
            "management recovery package differs from captured scope",
        )
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recovery {
    pub id: String,
    pub kind: Kind,
    pub role: RoleKind,
    pub previous_session: Option<String>,
    pub previous_turn: Option<String>,
    pub native_session_id: Option<String>,
    pub session_id: String,
    pub handoff: serde_json::Value,
    pub handoff_hash: String,
    pub approval_scope: String,
}
fn agent(role: RoleKind) -> &'static str {
    if role == RoleKind::Orchestrator {
        "orchestrator"
    } else {
        "orchestrator:verifier"
    }
}
fn current<'a>(
    run: &'a Run,
    round: &Round,
    role: RoleKind,
) -> Option<(&'a SessionRecord, &'a TurnRecord)> {
    run.session_ledger
        .turn_order
        .iter()
        .rev()
        .filter_map(|id| run.session_ledger.turns.get(id))
        .find_map(|turn| {
            let session = &run.session_ledger.sessions[&turn.session_id];
            (session.scope.task_id.as_deref() == Some(round.id.as_str())
                && session.scope.agent_id == agent(role)
                && session.scope.role == role)
                .then_some((session, turn))
        })
}
fn scoped(
    run: &Run,
    round: &Round,
    s: &SessionRecord,
    t: &TurnRecord,
    role: RoleKind,
) -> Result<()> {
    let provider = run
        .spec
        .config
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("management configuration missing".into()))?
        .config
        .resolve(WorkKind::Orchestration, role)?
        .provider;
    require(
        s.current_turn_id.as_ref() == Some(&t.id)
            && s.state == t.state
            && serde_json::to_value(provider)?.as_str() == Some(s.scope.provider.as_str())
            && s.scope
                == SessionScope::for_run(
                    run,
                    agent(role).into(),
                    Some(round.id.clone()),
                    role,
                    s.scope.provider.clone(),
                    s.scope.binary_version.clone(),
                )?,
        "management native scope/identity changed; start a newly scoped conversation",
    )
}
fn completed(s: &SessionRecord, t: &TurnRecord) -> Result<String> {
    require(
        s.native_observed
            && s.native_session_id.is_some()
            && t.state == TurnState::Completed
            && t.provider_completed
            && t.output.as_ref().is_some_and(|o| {
                t.output_ref
                    .as_ref()
                    .is_some_and(|r| r.document == format!("turn:{}", t.id) && r.hash == hash(o))
            }),
        "retained management output has no completed native authorship",
    )?;
    Ok(t.output.clone().unwrap())
}
pub fn for_round(run: &Run, round_id: &str, kind: Kind) -> Result<Pending> {
    require(
        !run.reconciliation_required
            && run.pending_edits.is_empty()
            && !matches!(run.status, Status::Running | Status::Paused),
        "reconcile and stop management work before recovery",
    )?;
    run.session_ledger.validate()?;
    let round = run
        .orchestration_rounds
        .last()
        .filter(|r| r.id == round_id)
        .ok_or_else(|| crate::Error::Invalid("management recovery round is superseded".into()))?;
    require(
        round.contract_version == planning::REVIEW_CONTRACT_VERSION
            && round.checklist_version == crate::domain::checklist_version(),
        "management recovery contract/checklist changed",
    )?;
    require(
        matches!(round.status, RoundStatus::Failed | RoundStatus::Interrupted)
            && round.spec_revision == run.spec_revision
            && round.source_hashes == run.source_hashes()
            && round.input_hash == hash(serde_json::to_vec(&run.input_snapshot)?)
            && round.comments_hash == crate::comments::input_hash(run)
            && !planning::new_context_after_round(run, round),
        "management recovery is stale, rejected, or has new user obligations",
    )?;
    require(
        crate::workspace::read_inputs(
            std::path::Path::new(&run.spec.workspace),
            &run.spec.input_files,
        )? == run.input_snapshot,
        "management sources changed",
    )?;
    let context: serde_json::Value = crate::transport::decode(round.request_context.as_bytes())?;
    require(
        context["spec"] == serde_json::to_value(&run.spec)?,
        "management configuration/contract changed",
    )?;
    if round.intake_evidence_id.is_some() {
        let raw: planning::Response = crate::transport::decode(
            round
                .output
                .as_ref()
                .ok_or_else(|| crate::Error::Invalid("intake source response missing".into()))?
                .as_bytes(),
        )?;
        let proposal: crate::runtime::intake::Proposal = crate::transport::decode(
            raw.intake_proposal_json
                .as_ref()
                .ok_or_else(|| crate::Error::Invalid("intake source proposal missing".into()))?
                .as_bytes(),
        )?;
        require(
            crate::workspace::read_inputs(
                std::path::Path::new(&run.spec.workspace),
                &proposal.task.input_files,
            )? == round.proposed_sources,
            "proposed intake sources changed; start a fresh management round",
        )?;
    }
    let executor = current(run, round, RoleKind::Orchestrator);
    let mut retained_executor = None;
    let mut retained_output = None;
    let role = if let Some((session, turn)) = executor {
        scoped(run, round, session, turn, RoleKind::Orchestrator)?;
        if turn.state == TurnState::Completed {
            let output = completed(session, turn)?;
            require(
                round.output.as_ref().is_none_or(|o| o == &output),
                "retained orchestrator response changed",
            )?;
            let response: planning::Response = crate::transport::decode(output.as_bytes())?;
            planning::validate_response(run, &response)?;
            retained_executor = Some(session.id.clone());
            retained_output = Some(output);
            RoleKind::Verifier
        } else {
            RoleKind::Orchestrator
        }
    } else {
        RoleKind::Orchestrator
    };
    let prior = current(run, round, role);
    let mut retained_review = None;
    if let Some((session, turn)) = prior {
        scoped(run, round, session, turn, role)?;
        if turn.state == TurnState::Completed && role == RoleKind::Verifier {
            let review = completed(session, turn)?;
            require(
                round.review.as_ref().is_none_or(|r| r == &review),
                "retained management review changed",
            )?;
            let _: planning::ReviewResponse = crate::transport::decode(review.as_bytes())?;
            retained_review = Some(review);
            require(
                kind == Kind::Restore,
                "completed management review must be restored without native replay",
            )?;
        } else {
            require(
                matches!(
                    turn.state,
                    TurnState::Failed | TurnState::Stopped | TurnState::DeliveryUnknown
                ) && turn.output.is_none()
                    && turn.output_ref.is_none(),
                "management turn is not interrupted",
            )?;
        }
    }
    require(
        kind != Kind::Restore || retained_review.is_some(),
        "no completed management review to restore",
    )?;
    if kind == Kind::Resume {
        let (session, turn) = prior.ok_or_else(|| {
            crate::Error::Invalid(
                "management native identity absent; use explicit replacement".into(),
            )
        })?;
        require(
            session.native_observed
                && session.native_session_id.is_some()
                && !matches!(
                    turn.failure,
                    Some(
                        crate::providers::FailureKind::InvalidSession
                            | crate::providers::FailureKind::Unsupported
                    )
                ),
            "management native identity unavailable; use explicit replacement",
        )?;
    }
    let messages: Vec<_> = run
        .communication_ledger
        .messages
        .values()
        .filter(|m| {
            m.recipient_agent_id == "orchestrator"
                || m.recipient_agent_id == "orchestrator:verifier"
        })
        .map(|m| serde_json::json!({"record":m,"content":run.message_bodies.get(&m.id)}))
        .collect();
    for message in &messages {
        if let Some(body) = message["content"].as_str() {
            require(
                message["record"]["content_ref"]["hash"] == hash(body),
                "management history body changed",
            )?;
        }
    }
    let handoff = serde_json::json!({"version":"orchestration-handoff-v1","round_id":round_id,
        "request_context_hash":hash(&round.request_context),"spec":run.spec,"source_hashes":run.source_hashes(),
        "messages":messages,"retained_executor":retained_executor,"retained_output":retained_output,"retained_review":retained_review,
        "previous_session":prior.map(|(s,_)|s),"previous_turn":prior.map(|(_,t)|t),
        "instruction":"Preserve original interview obligations and immutable request scope. Historical queued/unknown states do not prove old delivery. Recovery authorizes read-only management only, never team installation or project effects. A replacement is distinctly new; original native history remains unchanged."});
    let handoff_hash = hash(serde_json::to_vec(&handoff)?);
    Ok(Pending {
        round_id: round_id.into(),
        kind,
        role,
        previous_session: prior.map(|(s, _)| s.id.clone()),
        previous_turn: prior.map(|(_, t)| t.id.clone()),
        native_session_id: prior.and_then(|(s, _)| s.native_session_id.clone()),
        retained_executor,
        retained_output,
        retained_review,
        handoff,
        handoff_hash: handoff_hash.clone(),
        scope_hash: hash(serde_json::to_vec(&(
            "orchestration_recovery_v1",
            round_id,
            kind,
            run,
            handoff_hash,
        ))?),
    })
}
pub fn pending(run: &Run) -> BTreeMap<String, Pending> {
    run.orchestration_rounds
        .last()
        .map(|r| {
            [Kind::Resume, Kind::Replacement, Kind::Restore]
                .into_iter()
                .filter_map(|kind| {
                    for_round(run, &r.id, kind)
                        .ok()
                        .map(|p| (format!("{}:{kind:?}", r.id), p))
                })
                .collect()
        })
        .unwrap_or_default()
}
pub(super) fn prepare(run: &mut Run, round_id: &str, kind: Kind, scope: &str) -> Result<Status> {
    let p = for_round(run, round_id, kind)?;
    require(p.scope_hash == scope, "management recovery scope mismatch")?;
    let index = run.orchestration_rounds.len() - 1;
    let session_id = if kind == Kind::Replacement {
        id()
    } else {
        p.previous_session.clone().unwrap()
    };
    let round = &mut run.orchestration_rounds[index];
    if p.retained_output.is_some() {
        round.executor_session = p.retained_executor;
        round.output = p.retained_output;
    }
    if p.retained_review.is_some() {
        round.review = p.retained_review;
        round.verifier_session = p.previous_session.clone();
    }
    round.recoveries.push(Recovery {
        id: id(),
        kind,
        role: p.role,
        previous_session: p.previous_session,
        previous_turn: p.previous_turn,
        native_session_id: p.native_session_id,
        session_id,
        handoff: p.handoff,
        handoff_hash: p.handoff_hash,
        approval_scope: scope.into(),
    });
    round.status = RoundStatus::Running;
    round.diagnostic = None;
    let previous = round.previous_status;
    run.approvals.push(Approval {
        action: format!("recover-orchestration:{round_id}"),
        scope_hash: scope.into(),
        spec_revision: run.spec_revision,
        actor: "local_user".into(),
    });
    run.status = Status::Running;
    run.reconciliation_required = true;
    Ok(previous)
}
pub(crate) fn admitted<'a>(run: &'a Run, round_id: &str, role: RoleKind) -> Option<&'a Recovery> {
    let round = run
        .orchestration_rounds
        .iter()
        .find(|r| r.id == round_id && r.status == RoundStatus::Running)?;
    round
        .recoveries
        .last()
        .filter(|r| r.role == role && r.kind != Kind::Restore)
}
pub fn decorate(request: &mut crate::providers::PreparedTurn, recovery: &Recovery) -> Result<()> {
    require(
        recovery.handoff_hash == hash(serde_json::to_vec(&recovery.handoff)?),
        "management handoff content changed",
    )?;
    request.prompt.push_str(&format!("\nImmutable orchestration recovery: {}\nRecovery kind: {:?}. Preserve this same scoped management round and its obligations. Do not repeat completed executor output or claim new permissions/delivery receipts. Replacement is a DISTINCT NEW SESSION; exact resume keeps the observed native ID. Handoff hash: {}",serde_json::to_string(&recovery.handoff)?,recovery.kind,recovery.handoff_hash));
    Ok(())
}

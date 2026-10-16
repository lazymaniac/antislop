//! Read-only management recovery: exact resume, distinct replacement, or retained output.
use crate::{
    Result,
    config::{RoleKind, WorkKind},
    domain::*,
    hash, id,
    orchestration::planning::{self, Round, RoundStatus},
    require,
    sessions::{SessionRecord, TurnRecord, TurnState},
};
use serde::{Deserialize, Serialize};
use std::{cell::OnceCell, collections::BTreeMap};

pub(crate) mod authenticated;
pub(crate) mod handoff;

// One immutable run view can offer several recovery kinds. Authenticate its
// completed review prefix once, never once for each lookup and candidate kind.
// The borrowed view prevents reuse after a ledger/round mutation. Live sources
// are observed once for this view, and again for every new snapshot/admission.
struct ReviewProgress<'a> {
    run: &'a Run,
    part: OnceCell<Option<&'a planning::parts::Part>>,
    ledger_checked: OnceCell<()>,
    sources_checked: OnceCell<()>,
    validated_output: OnceCell<String>,
}
impl<'a> ReviewProgress<'a> {
    fn new(run: &'a Run) -> Self {
        Self {
            run,
            part: OnceCell::new(),
            ledger_checked: OnceCell::new(),
            sources_checked: OnceCell::new(),
            validated_output: OnceCell::new(),
        }
    }
    fn checked(&self, cache: &OnceCell<()>, validate: impl FnOnce() -> Result<()>) -> Result<()> {
        if cache.get().is_none() {
            validate()?;
            let _ = cache.set(());
        }
        Ok(())
    }
    fn validate_output(&self, output: &str) -> Result<()> {
        if self.validated_output.get().is_some_and(|old| old == output) {
            return Ok(());
        }
        let response: planning::Response = crate::transport::decode(output.as_bytes())?;
        planning::validate_response(self.run, &response)?;
        let _ = self.validated_output.set(output.into());
        Ok(())
    }
    fn part(&self, round: &'a Round) -> Result<Option<&'a planning::parts::Part>> {
        if round.review_parts.is_empty() {
            return Ok(None);
        }
        require(
            self.run
                .orchestration_rounds
                .last()
                .is_some_and(|r| std::ptr::eq(r, round)),
            "review progress belongs to another round",
        )?;
        if let Some(part) = self.part.get() {
            return Ok(*part);
        }
        let response: planning::Response = crate::transport::decode(
            round
                .output
                .as_ref()
                .ok_or_else(|| {
                    crate::Error::Invalid("review progress lacks its executor output".into())
                })?
                .as_bytes(),
        )?;
        let part = planning::parts::recovery_part(self.run, round, &response)?;
        let _ = self.part.set(part);
        Ok(part)
    }
}

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_part: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_part: Option<String>,
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
    round: &'a Round,
    role: RoleKind,
    progress: &ReviewProgress<'a>,
) -> Option<(&'a SessionRecord, &'a TurnRecord)> {
    let target = if role == RoleKind::Verifier && !round.review_parts.is_empty() {
        progress.part(round).ok()?
    } else {
        None
    };
    run.session_ledger
        .turn_order
        .iter()
        .rev()
        .filter_map(|id| run.session_ledger.turns.get(id))
        .find_map(|turn| {
            let session = &run.session_ledger.sessions[&turn.session_id];
            (session.scope.task_id.as_deref() == Some(round.id.as_str())
                && session.scope.agent_id == agent(role)
                && session.scope.role == role
                && target.is_none_or(|part| {
                    part.session_id.as_ref().map_or_else(
                        || {
                            !round
                                .review_parts
                                .iter()
                                .any(|p| p.session_id.as_ref() == Some(&session.id))
                        },
                        |id| id == &session.id,
                    )
                }))
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
                .matches_run(run, agent(role).into(), Some(round.id.clone()), role)?,
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
fn incomplete_positive_review<'a>(
    run: &'a Run,
    round: &'a Round,
    progress: &ReviewProgress<'a>,
) -> bool {
    let Some(output) = round.output.as_ref() else {
        return false;
    };
    let Ok(response) = crate::transport::decode::<planning::Response>(output.as_bytes()) else {
        return false;
    };
    let part = if round.review_parts.is_empty() {
        None
    } else {
        let Ok(part) = progress.part(round) else {
            return false;
        };
        part
    };
    let raw = part
        .and_then(|p| p.output.as_ref())
        .or(round.review.as_ref());
    let Some(raw) = raw else {
        return false;
    };
    let Ok(review) = crate::transport::decode::<planning::ReviewResponse>(raw.as_bytes()) else {
        return false;
    };
    review.findings.is_empty()
        && !review.checks.is_empty()
        && review
            .checks
            .iter()
            .all(|c| matches!(c.status, CheckStatus::Pass | CheckStatus::NotApplicable))
        && part
            .map_or_else(
                || planning::validate_review(run, round, &response, &review),
                |p| planning::validate_review_checks(run, round, &response, &review, &p.check_ids),
            )
            .is_err()
}

pub(crate) fn sources_current(run: &Run) -> Result<()> {
    let round = run
        .orchestration_rounds
        .last()
        .ok_or_else(|| crate::Error::Invalid("management round missing".into()))?;
    require(
        crate::workspace::inputs_current(run)?,
        "management sources changed",
    )?;
    if round.intake_evidence_id.is_some() {
        round.validate_proposed_observation()?;
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
        let captured = crate::workspace::observation::capture(
            std::path::Path::new(&run.spec.workspace),
            &proposal.task.input_files,
        )?;
        require(
            captured.snapshot == round.proposed_sources
                && round
                    .proposed_input_observation
                    .as_ref()
                    .is_none_or(|o| o == &captured.observation),
            "proposed intake sources changed; start a fresh management round",
        )?;
    }
    Ok(())
}

pub fn for_round(run: &Run, round_id: &str, kind: Kind) -> Result<Pending> {
    for_round_with_progress(run, round_id, kind, &ReviewProgress::new(run))
}
fn for_round_with_progress<'a>(
    run: &'a Run,
    round_id: &str,
    kind: Kind,
    progress: &ReviewProgress<'a>,
) -> Result<Pending> {
    require(
        std::ptr::eq(run, progress.run),
        "recovery view belongs to another run",
    )?;
    require(
        !run.reconciliation_required
            && run.pending_edits.is_empty()
            && !matches!(run.status, Status::Running | Status::Paused),
        "reconcile and stop management work before recovery",
    )?;
    progress.checked(&progress.ledger_checked, || {
        run.session_ledger.validate_for_run(&run.id)
    })?;
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
        (matches!(round.status, RoundStatus::Failed | RoundStatus::Interrupted)
            || (round.status == RoundStatus::Rejected
                && kind == Kind::Replacement
                && incomplete_positive_review(run, round, progress)))
            && round.spec_revision == run.spec_revision
            && round.source_hashes == run.source_hashes()
            && round.input_hash == hash(serde_json::to_vec(&run.input_snapshot)?)
            && round.comments_hash == crate::comments::input_hash(run)
            && !planning::new_context_after_round(run, round),
        "management recovery is stale, rejected, or has new user obligations",
    )?;
    progress.checked(&progress.sources_checked, || sources_current(run))?;
    let context: serde_json::Value = crate::transport::decode(round.request_context.as_bytes())?;
    require(
        context["spec"] == serde_json::to_value(&run.spec)?,
        "management configuration/contract changed",
    )?;
    let executor = current(run, round, RoleKind::Orchestrator, progress);
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
            progress.validate_output(&output)?;
            retained_executor = Some(session.id.clone());
            retained_output = Some(output);
            RoleKind::Verifier
        } else {
            RoleKind::Orchestrator
        }
    } else {
        RoleKind::Orchestrator
    };
    let review_part = if role == RoleKind::Verifier {
        let response: planning::Response =
            crate::transport::decode(retained_output.as_ref().unwrap().as_bytes())?;
        if round.review_parts.is_empty() {
            if kind == Kind::Replacement {
                planning::parts::plan(run, round, &response)?
                    .first()
                    .map(|p| p.id.clone())
            } else {
                None
            }
        } else {
            progress.part(round)?.map(|p| p.id.clone())
        }
    } else {
        None
    };
    let prior = current(run, round, role, progress);
    let mut retained_review = None;
    if let Some((session, turn)) = prior {
        scoped(run, round, session, turn, role)?;
        if turn.state == TurnState::Completed && role == RoleKind::Verifier {
            let native_review = completed(session, turn)?;
            let review = if !round.review_parts.is_empty() && review_part.is_none() {
                let response =
                    crate::transport::decode(retained_output.as_ref().unwrap().as_bytes())?;
                serde_json::to_string(&planning::parts::combine(run, round, &response)?)?
            } else {
                native_review.clone()
            };
            require(
                round.review.as_ref().is_none_or(|r| r == &review)
                    && review_part
                        .as_ref()
                        .and_then(|id| round.review_parts.iter().find(|p| &p.id == id))
                        .and_then(|p| p.output.as_ref())
                        .is_none_or(|raw| raw == &native_review),
                "retained management review changed",
            )?;
            let _: planning::ReviewResponse = crate::transport::decode(review.as_bytes())?;
            if kind == Kind::Replacement && incomplete_positive_review(run, round, progress) {
                // The completed raw report stays in the ledger/handoff. A fresh
                // independent review must satisfy every original obligation.
            } else {
                retained_review = Some(review);
                require(
                    kind == Kind::Restore,
                    "completed management review must be restored without native replay",
                )?;
            }
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
    let profile = run
        .spec
        .config
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("management configuration missing".into()))?
        .config
        .resolve(WorkKind::Orchestration, role)?;
    let mut handoff = serde_json::json!({"version":"orchestration-handoff-v1","round_id":round_id,
        "request_context_hash":hash(&round.request_context),"spec":run.spec,"source_hashes":run.source_hashes(),
        "messages":messages,"retained_executor":retained_executor,"retained_output":retained_output,"retained_review":retained_review,
        "previous_session":prior.map(|(s,_)|s),"previous_turn":if profile.isolation == crate::config::IsolationProfile::PinnedContext { serde_json::Value::Null } else { serde_json::to_value(prior.map(|(_,t)|t))? },"incomplete_positive_review":incomplete_positive_review(run,round,progress),
        "review_known_agents":round.review_known_agents.clone().unwrap_or_else(||super::messaging_agents(run)),
        "review_part":review_part,"review_part_provenance":round.review_parts.iter().map(|p|serde_json::json!({"id":p.id,"check_ids":p.check_ids,"session_id":p.session_id,"output_hash":p.output.as_ref().map(hash)})).collect::<Vec<_>>(),
        "instruction":"Preserve original interview obligations and immutable request scope. Historical queued/unknown states do not prove old delivery. Recovery authorizes read-only management only, never team installation or project effects. A replacement is distinctly new; original native history remains unchanged."});
    handoff::prepare(profile.isolation, prior.map(|(_, turn)| turn), &mut handoff)?;
    let handoff_hash = hash(serde_json::to_vec(&handoff)?);
    Ok(Pending {
        review_part,
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
    let progress = ReviewProgress::new(run);
    run.orchestration_rounds
        .last()
        .map(|r| {
            [Kind::Resume, Kind::Replacement, Kind::Restore]
                .into_iter()
                .filter_map(|kind| {
                    for_round_with_progress(run, &r.id, kind, &progress)
                        .ok()
                        .map(|p| (format!("{}:{kind:?}", r.id), p))
                })
                .collect()
        })
        .unwrap_or_default()
}
pub fn validate_pending_for_run(run: &Run, packages: &BTreeMap<String, Pending>) -> Result<()> {
    authenticated::Set::build_for_packages(run, packages).map(|_| ())
}
pub(super) fn prepare(
    run: &mut Run,
    round_id: &str,
    kind: Kind,
    scope: &str,
    authenticated: &authenticated::Set,
) -> Result<Status> {
    let p = authenticated.select(run, round_id, kind, scope)?;
    let index = run.orchestration_rounds.len() - 1;
    if run.orchestration_rounds[index]
        .review_known_agents
        .is_none()
    {
        run.orchestration_rounds[index].review_known_agents = Some(serde_json::from_value(
            p.handoff["review_known_agents"].clone(),
        )?);
    }
    if p.role == RoleKind::Verifier
        && p.review_part.is_some()
        && run.orchestration_rounds[index].review_parts.is_empty()
    {
        let round = &run.orchestration_rounds[index];
        let response = crate::transport::decode(p.retained_output.as_ref().unwrap().as_bytes())?;
        run.orchestration_rounds[index].review_parts =
            planning::parts::plan(run, round, &response)?;
    }
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
    if kind == Kind::Replacement && p.role == RoleKind::Verifier {
        round.review = None;
        round.verifier_session = None;
        if let Some(part) = p
            .review_part
            .as_ref()
            .and_then(|id| round.review_parts.iter_mut().find(|part| &part.id == id))
        {
            part.session_id = None;
            part.output = None;
        }
    }
    if p.retained_review.is_some() {
        if let Some(part) = p
            .review_part
            .as_ref()
            .and_then(|id| round.review_parts.iter_mut().find(|part| &part.id == id))
        {
            part.output = p.retained_review;
            part.session_id = p.previous_session.clone();
        } else {
            round.review = p.retained_review;
        }
        round.verifier_session = p.previous_session.clone();
    }
    round.recoveries.push(Recovery {
        review_part: p.review_part,
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
        .filter(|r| {
            role != RoleKind::Verifier
                || r.review_part.as_deref()
                    == planning::parts::pending(round).map(|p| p.id.as_str())
        })
}
pub fn decorate(
    run: &Run,
    request: &mut crate::providers::PreparedTurn,
    recovery: &Recovery,
) -> Result<()> {
    handoff::original(run, recovery)?;
    require(
        recovery.handoff_hash == hash(serde_json::to_vec(&recovery.handoff)?),
        "management handoff content changed",
    )?;
    request.prompt.push_str(&format!("\nImmutable orchestration recovery: {}\nRecovery kind: {:?}. Preserve this same scoped management round and its obligations. Do not repeat completed executor output or claim new permissions/delivery receipts. Replacement is a DISTINCT NEW SESSION; exact resume keeps the observed native ID. Handoff hash: {}",serde_json::to_string(&recovery.handoff)?,recovery.kind,recovery.handoff_hash));
    Ok(())
}

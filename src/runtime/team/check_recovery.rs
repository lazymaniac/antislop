//! Restore an already captured actual runner result, never invent an outcome or rerun.
use super::*;
use crate::domain::Approval;
use crate::runtime::team::checks::{CheckState, Invocation};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub id: String,
    pub invocation_id: String,
    pub intent_hash: String,
    pub original_intent: Invocation,
    pub output: String,
    pub output_hash: String,
    pub process_success: bool,
    pub observed_after: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_barrier: Option<BaselineBarrier>,
}
/// Captured by the baseline coordinator before it can admit any native worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BaselineBarrier {
    pub team: crate::team::TeamPlan,
    pub native_ledger_hash: String,
    pub execution_ledger_hash: String,
    pub run_attempt: u64,
}
fn baseline_barrier_matches(run: &Run, o: &Outcome) -> Result<bool> {
    let Some(barrier) = &o.baseline_barrier else {
        return Ok(false);
    };
    if o.original_intent.plan.phase != crate::team::checks::CheckPhase::BeforeExecution {
        return Ok(false);
    }
    let Some(team) = &run.team else {
        return Ok(false);
    };
    if barrier.team.checkpoint_uncertain
        || !barrier.team.tasks.iter().any(|t| {
            t.id == o.original_intent.task_id
                && t.attempt == o.original_intent.task_attempt
                && t.state == TaskState::Running
                && t.artifact.is_none()
                && t.review.is_none()
        })
    {
        return Ok(false);
    }
    barrier.team.validate(&run.spec)?;
    let mut expected = barrier.team.clone();
    expected.recover_interrupted();
    expected.checkpoint_uncertain = team.checkpoint_uncertain;
    Ok(barrier.run_attempt == run.attempt
        && barrier.native_ledger_hash == hash(serde_json::to_vec(&run.session_ledger)?)
        && barrier.execution_ledger_hash == hash(serde_json::to_vec(&run.task_executions)?)
        && serde_json::to_value(expected)? == serde_json::to_value(team)?)
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Restoration {
    pub outcome_id: String,
    pub original_state: CheckState,
    pub actor: String,
    pub scope_hash: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pending {
    pub invocation_id: String,
    pub task_id: String,
    pub outcome: Outcome,
    pub scope_hash: String,
}
impl Pending {
    pub fn validate_for_run(&self, run: &Run) -> Result<()> {
        let current = for_invocation(run, &self.invocation_id)?;
        require(
            serde_json::to_value(self)? == serde_json::to_value(current)?,
            "captured task check recovery package changed",
        )
    }
}
pub(crate) fn intent_hash(i: &Invocation) -> Result<String> {
    Ok(hash(serde_json::to_vec(&(
        "task_check_intent_v1",
        &i.id,
        &i.task_id,
        i.task_attempt,
        &i.artifact_id,
        &i.input_snapshot,
        &i.artifact_hash,
        &i.plan,
        i.spec_revision,
        &i.configuration_hash,
        &i.source_hashes,
        &i.authorization_scope,
        &i.before_hash,
    ))?))
}
fn binding<'a>(run: &'a Run, i: &Invocation) -> Result<&'a Outcome> {
    let outcomes: Vec<_> = run
        .task_check_outcomes
        .iter()
        .filter(|o| o.invocation_id == i.id)
        .collect();
    require(
        outcomes.len() == 1,
        "task check has no unique captured actual outcome",
    )?;
    let o = outcomes[0];
    let inputs = if i.plan.phase == crate::team::checks::CheckPhase::BeforeExecution {
        i.input_snapshot.as_ref().ok_or_else(|| {
            crate::Error::Invalid("captured baseline input snapshot missing".into())
        })?
    } else {
        &run.input_snapshot
    };
    require(
        o.original_intent.id == i.id
            && o.original_intent.state == CheckState::Intent
            && o.original_intent.restoration.is_none()
            && o.intent_hash == intent_hash(i)?
            && o.intent_hash == intent_hash(&o.original_intent)?
            && o.output_hash == hash(&o.output)
            && o.observed_after.as_ref() == Some(inputs)
            && i.before_hash == hash(serde_json::to_vec(inputs)?)
            && i.source_hashes == run.source_hashes_for_inputs(inputs)
            && i.spec_revision == run.spec_revision
            && run.mode_configuration_matches(&i.configuration_hash),
        "captured check intent/result/source binding changed",
    )?;
    let output: serde_json::Value = crate::transport::decode(o.output.as_bytes())?;
    require(
        output["argv"] == serde_json::to_value(&i.plan.argv)?
            && output["truncated"].is_boolean()
            && (output["exit_code"].is_i64() || output["signal"].is_i64())
            && o.process_success
                == (output["exit_code"] == 0
                    && output["signal"].is_null()
                    && output["truncated"] == false),
        "captured check outcome is not a known actual process result",
    )?;
    require(
        run.approvals.iter().any(|a| {
            a.actor == "local_user"
                && a.spec_revision == i.spec_revision
                && a.scope_hash == i.authorization_scope
                && matches!(
                    a.action.as_str(),
                    "step_team"
                        | "refresh_task_sources"
                        | "retry_task_verification"
                        | "resume_task_session"
                        | "replace_task_session"
                )
        }),
        "captured check lacks its original scoped authorization",
    )?;
    Ok(o)
}
pub fn for_invocation(run: &Run, id: &str) -> Result<Pending> {
    require(
        !run.reconciliation_required
            && run.pending_edits.is_empty()
            && !matches!(
                run.status,
                Status::Running | Status::Paused | Status::Completed | Status::Cancelled
            ),
        "stop/reconcile before restoring a task check",
    )?;
    require(
        run.checklist_version == crate::domain::checklist_version(),
        "task check restoration checklist changed; explicit revision required",
    )?;
    run.session_ledger.validate_for_run(&run.id)?;
    require(
        crate::workspace::inputs_current(run)?,
        "check restoration workspace changed",
    )?;
    let i = run
        .task_checks
        .iter()
        .find(|i| i.id == id)
        .ok_or_else(|| crate::Error::Invalid("unknown task check invocation".into()))?;
    require(
        matches!(i.state, CheckState::Intent | CheckState::Unknown)
            && i.restoration.is_none()
            && i.evidence_id.is_none()
            && i.output_hash.is_none(),
        "check is already completed or has a conflicting recorded outcome",
    )?;
    let team = run
        .team
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("task check team missing".into()))?;
    team.validate(&run.spec)?;
    require(
        team.spec_revision == run.spec_revision,
        "task check team revision is stale",
    )?;
    let outcome = binding(run, i)?.clone();
    require(
        !team.checkpoint_uncertain || baseline_barrier_matches(run, &outcome)?,
        "scheduler checkpoint lacks a captured pre-admission baseline barrier",
    )?;
    let task = team
        .tasks
        .iter()
        .find(|t| t.id == i.task_id)
        .ok_or_else(|| crate::Error::Invalid("task check owner missing".into()))?;
    require(
        task.state == TaskState::Blocked
            && task.review.is_none()
            && task.attempt > 0
            && task.attempt == i.task_attempt
            && task.check_plan.iter().any(|p| p == &i.plan),
        "task check contract/attempt changed",
    )?;
    if i.plan.phase == crate::team::checks::CheckPhase::BeforeVerification {
        require(
            task.artifact.as_ref().is_some_and(|a| {
                Some(&a.id) == i.artifact_id.as_ref() && a.content_hash == i.artifact_hash
            }),
            "check executor artifact changed",
        )?;
        let reference = task.artifact.as_ref().unwrap();
        let artifact = run
            .artifacts
            .iter()
            .find(|a| a.id == reference.id)
            .ok_or_else(|| crate::Error::Invalid("check executor content missing".into()))?;
        require(
            artifact.hash == reference.content_hash
                && hash(&artifact.content) == artifact.hash
                && reference.source_hashes == run.source_hashes(),
            "check executor content/sources changed",
        )?;
        let session = run
            .session_ledger
            .sessions
            .get(&reference.session_id)
            .ok_or_else(|| crate::Error::Invalid("check executor session missing".into()))?;
        let turn = session
            .current_turn_id
            .as_ref()
            .and_then(|id| run.session_ledger.turns.get(id))
            .ok_or_else(|| crate::Error::Invalid("check executor turn missing".into()))?;
        require(
            session.native_observed
                && turn.state == crate::sessions::TurnState::Completed
                && turn.provider_completed
                && turn.output.as_ref() == Some(&artifact.content),
            "check executor lacks native completed authorship",
        )?;
        effects::binding(run, reference)?;
    }
    Ok(Pending {
        invocation_id: id.into(),
        task_id: i.task_id.clone(),
        outcome,
        scope_hash: hash(serde_json::to_vec(&("restore_task_check_v1", id, run))?),
    })
}
pub fn pending(run: &Run) -> BTreeMap<String, Pending> {
    run.task_checks
        .iter()
        .filter_map(|i| for_invocation(run, &i.id).ok().map(|p| (i.id.clone(), p)))
        .collect()
}
pub(crate) fn restore(run: &mut Run, invocation_id: &str, scope: &str) -> Result<()> {
    let p = for_invocation(run, invocation_id)?;
    require(
        p.scope_hash == scope,
        "task check restoration scope mismatch",
    )?;
    let baseline_barrier_settled = baseline_barrier_matches(run, &p.outcome)?;
    let i = run
        .task_checks
        .iter_mut()
        .find(|i| i.id == invocation_id)
        .unwrap();
    let evidence_id = id();
    let original_state = i.state.clone();
    i.state = CheckState::Completed;
    i.success = p.outcome.process_success;
    i.after_hash = Some(i.before_hash.clone());
    i.output_hash = Some(p.outcome.output_hash.clone());
    i.evidence_id = Some(evidence_id.clone());
    i.restoration = Some(Restoration {
        outcome_id: p.outcome.id.clone(),
        original_state,
        actor: "local_user".into(),
        scope_hash: scope.into(),
    });
    run.evidence.push(Evidence {
        id: evidence_id,
        artifact_hash: i.artifact_hash.clone(),
        spec_revision: run.spec_revision,
        source: format!(
            "runner:{}{}",
            if i.plan.phase == crate::team::checks::CheckPhase::BeforeExecution {
                "baseline:"
            } else {
                ""
            },
            serde_json::to_string(&i.plan.argv)?
        ),
        output_hash: p.outcome.output_hash,
        output: p.outcome.output,
        success: i.success,
        input_hash: Some(i.before_hash.clone()),
    });
    run.approvals.push(Approval {
        action: "restore_task_check_result".into(),
        scope_hash: scope.into(),
        spec_revision: run.spec_revision,
        actor: "local_user".into(),
    });
    authenticate(
        run,
        run.task_checks
            .iter()
            .find(|i| i.id == invocation_id)
            .unwrap(),
    )?;
    if baseline_barrier_settled
        && run
            .task_checks
            .iter()
            .all(|c| c.state == CheckState::Completed)
        && !run.session_ledger.turns.values().any(|t| {
            matches!(
                t.state,
                crate::sessions::TurnState::Pending
                    | crate::sessions::TurnState::PendingUnknown
                    | crate::sessions::TurnState::Sent
            )
        })
    {
        // Only the captured coordinator barrier is settled. Tasks remain blocked
        // until explicit continuation; no worker was admitted or is replayed.
        run.team.as_mut().unwrap().checkpoint_uncertain = false;
    }
    let restored = run
        .task_checks
        .iter()
        .find(|i| i.id == invocation_id)
        .unwrap();
    if restored.plan.phase == crate::team::checks::CheckPhase::BeforeVerification {
        let task_id = restored.task_id.clone();
        let task = run
            .team
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.id == task_id)
            .unwrap();
        let owner = &run.session_ledger.sessions[&task.artifact.as_ref().unwrap().session_id];
        let boundary = run
            .session_ledger
            .turn_order
            .iter()
            .position(|id| Some(id) == owner.current_turn_id.as_ref())
            .unwrap();
        let prior_verifier = run
            .session_ledger
            .turn_order
            .iter()
            .skip(boundary + 1)
            .any(|id| {
                let session =
                    &run.session_ledger.sessions[&run.session_ledger.turns[id].session_id];
                session.scope.task_id.as_ref() == Some(&task_id)
                    && session.scope.role == RoleKind::Verifier
            });
        let all_known = run
            .task_checks
            .iter()
            .filter(|c| c.task_id == task_id && c.task_attempt == task.attempt)
            .all(|c| c.state == CheckState::Completed);
        if !prior_verifier && all_known {
            let task = run
                .team
                .as_mut()
                .unwrap()
                .tasks
                .iter_mut()
                .find(|t| t.id == task_id)
                .unwrap();
            task.state = TaskState::AwaitingVerification;
            task.diagnostic = None;
            run.team.as_ref().unwrap().validate(&run.spec)?;
        }
    }
    run.diagnostic=Some("Actual captured check result restored without running the command; independent task review remains required".into());
    Ok(())
}
pub(crate) fn authenticate(run: &Run, i: &Invocation) -> Result<()> {
    let r = i
        .restoration
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("task check restoration missing".into()))?;
    let o = binding(run, i)?;
    require(
        r.actor == "local_user"
            && r.outcome_id == o.id
            && matches!(r.original_state, CheckState::Intent | CheckState::Unknown)
            && run.approvals.iter().any(|a| {
                a.action == "restore_task_check_result"
                    && a.actor == "local_user"
                    && a.spec_revision == run.spec_revision
                    && a.scope_hash == r.scope_hash
            })
            && i.output_hash.as_ref() == Some(&o.output_hash)
            && i.success == o.process_success,
        "task check restoration has no authenticated captured result/approval",
    )
}

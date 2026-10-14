//! Restore a completed, durably applied executor result without repeating a turn or effect.
use super::*;
use crate::sessions::{SessionScope, TurnState};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Restoration {
    pub id: String,
    pub task_id: String,
    pub task_attempt: u64,
    pub execution_id: String,
    pub session_id: String,
    pub turn_id: String,
    pub artifact_id: String,
    pub artifact_hash: String,
    pub scope_hash: String,
    pub actor: String,
}

fn candidate<'a>(run: &'a Run, task_id: &str) -> Result<&'a effects::ExecutionBinding> {
    require(
        !run.reconciliation_required && run.pending_edits.is_empty(),
        "reconcile before restoring an executor result",
    )?;
    require(
        matches!(
            run.status,
            Status::Blocked | Status::Ready | Status::AwaitingResult | Status::ChangesRequested
        ),
        "executor restoration requires stopped work",
    )?;
    require(
        run.checklist_version == crate::domain::checklist_version(),
        "executor result checklist changed; explicit revision required",
    )?;
    let team = run
        .team
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("no installed team".into()))?;
    team.validate(&run.spec)?;
    require(
        team.spec_revision == run.spec_revision,
        "executor result team revision is stale",
    )?;
    let task = team
        .tasks
        .iter()
        .find(|t| t.id == task_id)
        .ok_or_else(|| crate::Error::Invalid("unknown task".into()))?;
    require(
        task.state == TaskState::Blocked
            && task.artifact.is_none()
            && task.review.is_none()
            && task.attempt > 0,
        "only a missing interrupted executor result can be restored",
    )?;
    let executions = run
        .task_executions
        .iter()
        .filter(|e| e.task_id == task_id && e.attempt == task.attempt)
        .collect::<Vec<_>>();
    require(
        executions.len() == 1,
        "executor outcome missing or ambiguous; do not replay",
    )?;
    let execution = executions[0];
    require(
        matches!(
            execution.state,
            effects::ExecutionState::Applied | effects::ExecutionState::ObservedApplied
        ) && execution.dependency_hashes == team.dependency_bindings(task)?,
        "executor effects/dependencies are uncertain or changed",
    )?;
    require(
        execution.source_hashes == run.source_hashes_for_inputs(&execution.input_snapshot),
        "executor original input binding changed",
    )?;
    effects::authenticate(run, execution)?;
    effects::lineage(run, task)?;
    let owner = team.agents.iter().find(|a| a.id == task.owner_id).unwrap();
    require(
        owner.session_id.as_ref() == Some(&execution.session_id),
        "completed executor is not this task's current owner session",
    )?;
    let session = &run.session_ledger.sessions[&execution.session_id];
    let mut original = run.clone();
    original.stage = session.scope.stage;
    original.input_snapshot = execution.input_snapshot.clone();
    let stage = match task.stage {
        TaskStage::Discovery => Stage::Discovery,
        TaskStage::Planning => Stage::Planning,
        TaskStage::Execution => Stage::Execution,
        TaskStage::Validation => Stage::Validation,
        TaskStage::Integration => Stage::Integration,
        TaskStage::Delivery => Stage::Delivery,
    };
    let provider = run
        .spec
        .config
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("executor configuration missing".into()))?
        .config
        .resolve(owner.role.work_kind(), owner.kind)?
        .provider;
    require(
        session.scope.stage == stage
            && session.scope
                == SessionScope::for_run(
                    &original,
                    owner.id.clone(),
                    Some(task.id.clone()),
                    owner.kind,
                    session.scope.provider.clone(),
                    session.scope.binary_version.clone(),
                )?
            && serde_json::to_value(provider)?.as_str() == Some(session.scope.provider.as_str()),
        "executor result native scope/configuration changed",
    )?;
    let turn = &run.session_ledger.turns[session.current_turn_id.as_ref().unwrap()];
    let output: crate::team::models::Output =
        crate::transport::decode(turn.output.as_ref().unwrap().as_bytes())?;
    crate::team::models::validate_output(&original, task, owner.role, &output)?;
    for receipt in &execution.receipts {
        require(
            run.input_snapshot
                .get(&receipt.path)
                .is_some_and(|text| hash(text) == receipt.after_hash),
            "executor owned effect changed since completion",
        )?;
    }
    require(
        !run.artifacts
            .iter()
            .any(|a| a.session_id == execution.session_id && a.attempt == task.attempt),
        "executor output already retained; use verifier recovery",
    )?;
    require(
        !run.task_checks.iter().any(|c| {
            c.task_id == task_id
                && c.task_attempt == task.attempt
                && matches!(
                    c.state,
                    checks::CheckState::Intent | checks::CheckState::Unknown
                )
        }),
        "executor has an uncertain check outcome; adjudicate before restoration",
    )?;
    let mut preview = run.clone();
    attach_result(&mut preview, task_id, execution)?;
    Ok(execution)
}

fn attach_result(
    run: &mut Run,
    task_id: &str,
    execution: &effects::ExecutionBinding,
) -> Result<(String, String)> {
    let session = &run.session_ledger.sessions[&execution.session_id];
    let turn_id = session.current_turn_id.as_ref().unwrap().clone();
    let raw = run.session_ledger.turns[&turn_id]
        .output
        .as_ref()
        .unwrap()
        .clone();
    let stage = session.scope.stage;
    let artifact_id = id();
    let evidence_id = id();
    let reference = ArtifactRef {
        id: artifact_id.clone(),
        task_id: task_id.into(),
        created_by: execution.scope.agent_id.clone(),
        session_id: execution.session_id.clone(),
        attempt: execution.attempt,
        spec_revision: run.spec_revision,
        content_hash: execution.artifact_hash.clone(),
        source_hashes: execution.source_hashes.clone(),
        dependency_hashes: execution.dependency_hashes.clone(),
        execution_id: Some(execution.id.clone()),
        evidence_refs: BTreeSet::from([evidence_id.clone()]),
    };
    run.task_executions
        .iter_mut()
        .find(|e| e.id == execution.id)
        .unwrap()
        .state = effects::ExecutionState::Committed;
    run.team
        .as_mut()
        .unwrap()
        .restore_executor_result(&run.spec, task_id, reference)?;
    run.artifacts.push(Artifact {
        id: artifact_id.clone(),
        stage,
        attempt: execution.attempt,
        spec_revision: run.spec_revision,
        session_id: execution.session_id.clone(),
        content: raw.clone(),
        hash: execution.artifact_hash.clone(),
    });
    run.evidence.push(Evidence {
        id: evidence_id,
        artifact_hash: execution.artifact_hash.clone(),
        spec_revision: run.spec_revision,
        source: "runtime:artifact-read".into(),
        output_hash: hash(&raw),
        output: raw,
        success: true,
        input_hash: Some(hash(serde_json::to_vec(&execution.input_snapshot)?)),
    });
    let task = run
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == task_id)
        .unwrap();
    checks::retry_safe(run, task, run.artifacts.last().unwrap())?;
    Ok((artifact_id, turn_id))
}

fn scope_hash(run: &Run, task_id: &str) -> Result<String> {
    Ok(hash(serde_json::to_vec(&(
        "restore_task_result_v1",
        task_id,
        run,
    ))?))
}

pub(crate) fn scope(run: &Run, task_id: &str) -> Result<String> {
    candidate(run, task_id)?;
    scope_hash(run, task_id)
}

pub(crate) fn scopes(run: &Run) -> BTreeMap<String, String> {
    run.team
        .as_ref()
        .map(|team| {
            team.tasks
                .iter()
                .filter(|t| t.state == TaskState::Blocked && t.artifact.is_none())
                .filter_map(|t| scope(run, &t.id).ok().map(|s| (t.id.clone(), s)))
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn restore(run: &mut Run, task_id: &str, expected_scope: &str) -> Result<()> {
    let execution = candidate(run, task_id)?.clone();
    require(
        scope_hash(run, task_id)? == expected_scope,
        "executor result restoration scope changed",
    )?;
    let (artifact_id, turn_id) = attach_result(run, task_id, &execution)?;
    run.task_result_restorations.push(Restoration {
        id: id(),
        task_id: task_id.into(),
        task_attempt: execution.attempt,
        execution_id: execution.id,
        session_id: execution.session_id,
        turn_id,
        artifact_id,
        artifact_hash: execution.artifact_hash,
        scope_hash: expected_scope.into(),
        actor: "local_user".into(),
    });
    run.approvals.push(crate::domain::Approval {
        action: "restore_task_result".into(),
        scope_hash: expected_scope.into(),
        spec_revision: run.spec_revision,
        actor: "local_user".into(),
    });
    // Clear only scheduler uncertainty for which every interrupted executor has
    // retained authenticated effects/output. Unknown checks still hold the barrier.
    let settled = run
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .all(|t| match t.state {
            TaskState::Running | TaskState::Verifying => false,
            TaskState::Blocked => t
                .artifact
                .as_ref()
                .is_some_and(|a| effects::binding(run, a).is_ok()),
            _ => true,
        })
        && !run.task_checks.iter().any(|c| {
            matches!(
                c.state,
                checks::CheckState::Intent | checks::CheckState::Unknown
            )
        })
        && !run.session_ledger.turns.values().any(|t| {
            matches!(
                t.state,
                TurnState::PendingUnknown | TurnState::Pending | TurnState::Sent
            )
        });
    if settled {
        run.team.as_mut().unwrap().checkpoint_uncertain = false;
    }
    let context = gate_context(run);
    run.team.as_mut().unwrap().invalidate_sources(&context);
    let team = run.team.as_ref().unwrap();
    run.status = if team.checkpoint_uncertain
        || team
            .tasks
            .iter()
            .any(|t| matches!(t.state, TaskState::Blocked | TaskState::Stale))
    {
        Status::Blocked
    } else {
        Status::Ready
    };
    run.diagnostic = Some("Completed executor output/effects restored without replay; independent verification or read-only source refresh still required".into());
    Ok(())
}

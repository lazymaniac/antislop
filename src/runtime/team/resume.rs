//! Scope-bound exact native continuation. Original unknown turns remain history.
use super::*;
use crate::{
    sessions::{SessionScope, TurnState},
    team::{NativeResume, TaskRecord},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pending {
    pub task_id: String,
    pub operation: TaskOperation,
    pub session_id: String,
    pub previous_turn_id: String,
    pub native_session_id: String,
    pub scope_hash: String,
}
fn stage(task: &TaskRecord) -> Stage {
    match task.stage {
        TaskStage::Discovery => Stage::Discovery,
        TaskStage::Planning => Stage::Planning,
        TaskStage::Execution => Stage::Execution,
        TaskStage::Validation => Stage::Validation,
        TaskStage::Integration => Stage::Integration,
        TaskStage::Delivery => Stage::Delivery,
    }
}
pub(super) fn interrupted(run: &Run, task_id: &str) -> Result<Interrupted> {
    require(
        !run.reconciliation_required && run.pending_edits.is_empty(),
        "reconcile effects before native session continuation",
    )?;
    require(
        matches!(
            run.status,
            Status::Ready
                | Status::AwaitingResult
                | Status::AwaitingStart
                | Status::Blocked
                | Status::ChangesRequested
        ),
        "run is not stopped for native continuation",
    )?;
    require(
        run.checklist_version == crate::domain::checklist_version(),
        "task checklist changed",
    )?;
    run.session_ledger.validate()?;
    let team = run
        .team
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("no team installed".into()))?;
    team.validate(&run.spec)?;
    require(
        !team.checkpoint_uncertain,
        "scheduler checkpoint requires recovery before native continuation",
    )?;
    let task = team
        .tasks
        .iter()
        .find(|t| t.id == task_id)
        .ok_or_else(|| crate::Error::Invalid("unknown task".into()))?;
    require(
        task.state == TaskState::Blocked
            && task.review.is_none()
            && task.attempt == task.history.len() as u64 + 1,
        "only an interrupted current attempt can resume",
    )?;
    let operation = if task.artifact.is_some() {
        TaskOperation::Verify
    } else {
        TaskOperation::Execute
    };
    if operation == TaskOperation::Verify {
        // Preserve all executor authorship, prior substantive concerns and check gates.
        retry::scope(run, task_id)?;
    }
    require(
        crate::workspace::read_inputs(Path::new(&run.spec.workspace), &run.spec.input_files)?
            == run.input_snapshot,
        "workspace changed before native continuation",
    )?;
    require(
        run.task_checks
            .iter()
            .filter(|c| c.task_id == task_id && c.task_attempt == task.attempt)
            .all(|c| c.state == checks::CheckState::Completed),
        "unknown task checks cannot be replayed by native continuation",
    )?;
    require(
        operation != TaskOperation::Execute
            || !run
                .task_executions
                .iter()
                .any(|e| e.task_id == task_id && e.attempt == task.attempt),
        "executor effects/output exist; reconcile or restore them without native replay",
    )?;
    let role = if operation == TaskOperation::Execute {
        RoleKind::Executor
    } else {
        RoleKind::Verifier
    };
    let agent_id = if operation == TaskOperation::Execute {
        &task.owner_id
    } else {
        &task.verifier_id
    };
    let agent = team.agents.iter().find(|a| &a.id == agent_id).unwrap();
    let turn = run
        .session_ledger
        .turn_order
        .iter()
        .rev()
        .filter_map(|id| run.session_ledger.turns.get(id))
        .find(|t| {
            let s = &run.session_ledger.sessions[&t.session_id];
            s.scope.task_id.as_deref() == Some(task_id) && s.scope.role == role
        });
    let session = turn.map(|t| &run.session_ledger.sessions[&t.session_id]);
    if let (Some(turn), Some(session)) = (turn, session) {
        let boundary = if operation == TaskOperation::Verify {
            task.artifact.as_ref()
        } else {
            task.history.last().and_then(|h| h.artifact.as_ref())
        };
        if let Some(artifact) = boundary {
            let prior_turn = run
                .session_ledger
                .sessions
                .get(&artifact.session_id)
                .and_then(|s| s.current_turn_id.as_ref())
                .ok_or_else(|| {
                    crate::Error::Invalid("native continuation attempt boundary missing".into())
                })?;
            let position = |id: &str| run.session_ledger.turn_order.iter().position(|t| t == id);
            require(
                position(&turn.id)
                    .zip(position(prior_turn))
                    .is_some_and(|(current, prior)| current > prior),
                "native session belongs to an earlier attempt or predates the pinned executor",
            )?;
        }
        require(
            session.current_turn_id.as_ref() == Some(&turn.id)
                && session.state == turn.state
                && turn.output.is_none()
                && turn.output_ref.is_none()
                && matches!(
                    turn.state,
                    TurnState::Failed | TurnState::Stopped | TurnState::DeliveryUnknown
                ),
            "task recovery requires an incomplete current turn; restore completed output instead",
        )?;
    }
    let profile = run
        .spec
        .config
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("configuration missing".into()))?
        .config
        .resolve(agent.role.work_kind(), role)?;
    let mut view = run.clone();
    view.stage = stage(task);
    if let Some(session) = session {
        require(
            serde_json::to_value(profile.provider)?.as_str()
                == Some(session.scope.provider.as_str())
                && session.scope
                    == SessionScope::for_run(
                        &view,
                        agent_id.clone(),
                        Some(task.id.clone()),
                        role,
                        session.scope.provider.clone(),
                        session.scope.binary_version.clone(),
                    )?,
            "native session workspace/input/configuration/role changed; replacement requires a revised contract",
        )?;
    }
    Ok(Interrupted {
        operation,
        session: session.cloned(),
        turn: turn.cloned(),
    })
}

pub(super) struct Interrupted {
    pub operation: TaskOperation,
    pub session: Option<crate::sessions::SessionRecord>,
    pub turn: Option<crate::sessions::TurnRecord>,
}

pub fn for_task(run: &Run, task_id: &str) -> Result<Pending> {
    let checkpoint = interrupted(run, task_id)?;
    let operation = checkpoint.operation;
    let session = checkpoint
        .session
        .ok_or_else(|| crate::Error::Invalid("no ordered native task turn to resume".into()))?;
    let turn = checkpoint.turn.unwrap();
    require(
        !matches!(
            turn.failure,
            Some(providers::FailureKind::InvalidSession | providers::FailureKind::Unsupported)
        ),
        "provider rejected this native session; explicitly replace it with a handoff",
    )?;
    require(
        session.native_observed,
        "native session identity unobserved; explicit replacement required",
    )?;
    let native = session.native_session_id.clone().ok_or_else(|| {
        crate::Error::Invalid(
            "native session identity unknown; explicit replacement required".into(),
        )
    })?;
    let p = Pending {
        task_id: task_id.into(),
        operation,
        session_id: session.id.clone(),
        previous_turn_id: turn.id.clone(),
        native_session_id: native,
        scope_hash: hash(serde_json::to_vec(&(
            "native_task_resume_v1",
            task_id,
            run,
        ))?),
    };
    // Preview the same transition and normal gate/dependency admission used at launch.
    let mut staged = run.clone();
    install(&mut staged, &p)?;
    let ready = staged.team.as_ref().unwrap().ready_wave_for(
        &run.spec,
        &gate_context(run),
        Some(task_id),
    )?;
    require(
        ready.len() == 1 && ready[0].operation == operation,
        "selected continuation is not ready",
    )?;
    Ok(p)
}
pub fn pending(run: &Run) -> BTreeMap<String, Pending> {
    run.team
        .as_ref()
        .map(|team| {
            team.tasks
                .iter()
                .filter_map(|t| for_task(run, &t.id).ok().map(|p| (t.id.clone(), p)))
                .collect()
        })
        .unwrap_or_default()
}
pub(crate) fn prepare(run: &mut Run, task_id: &str, scope: &str) -> Result<TaskOperation> {
    let p = for_task(run, task_id)?;
    require(p.scope_hash == scope, "native continuation scope mismatch")?;
    install(run, &p)?;
    Ok(p.operation)
}
fn install(run: &mut Run, p: &Pending) -> Result<()> {
    let launch_attempt = run
        .attempt
        .checked_add(1)
        .ok_or_else(|| crate::Error::Invalid("run attempt counter exhausted".into()))?;
    let task = run
        .team
        .as_mut()
        .unwrap()
        .tasks
        .iter_mut()
        .find(|t| t.id == p.task_id)
        .unwrap();
    task.native_resumes.push(NativeResume {
        id: id(),
        attempt: task.attempt,
        launch_attempt,
        operation: p.operation,
        session_id: p.session_id.clone(),
        previous_turn_id: p.previous_turn_id.clone(),
        native_session_id: p.native_session_id.clone(),
        approval_scope: p.scope_hash.clone(),
    });
    task.state = if p.operation == TaskOperation::Execute {
        TaskState::Resuming
    } else {
        TaskState::AwaitingVerification
    };
    task.diagnostic = None;
    run.team.as_ref().unwrap().validate(&run.spec)?;
    Ok(())
}
pub(crate) fn admitted<'a>(
    view: &'a Run,
    task_id: &str,
    role: RoleKind,
) -> Option<&'a NativeResume> {
    let task = view.team.as_ref()?.tasks.iter().find(|t| t.id == task_id)?;
    task.native_resumes.last().filter(|r| {
        r.launch_attempt == view.attempt
            && r.attempt == task.attempt
            && ((r.operation == TaskOperation::Execute && role == RoleKind::Executor)
                || (r.operation == TaskOperation::Verify && role == RoleKind::Verifier))
    })
}

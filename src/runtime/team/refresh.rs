//! Explicit read-only source refresh, preserving prior task outputs and effects.
use super::*;
pub(crate) fn scope(run: &Run, task_id: &str) -> Result<String> {
    require(
        run.checklist_version == crate::domain::checklist_version(),
        "source refresh checklist changed; explicit revision required",
    )?;
    require(
        !run.reconciliation_required
            && matches!(
                run.status,
                Status::Ready | Status::AwaitingResult | Status::Blocked | Status::ChangesRequested
            ),
        "source refresh requires reconciled stopped work",
    )?;
    let team = run
        .team
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("no team installed".into()))?;
    require(
        !team.checkpoint_uncertain,
        "task checkpoint outcome is uncertain",
    )?;
    team.validate(&run.spec)?;
    let task = team
        .tasks
        .iter()
        .find(|t| t.id == task_id)
        .ok_or_else(|| crate::Error::Invalid("unknown task".into()))?;
    require(
        task.state == TaskState::Stale && task.review.as_ref().is_none_or(|r| r.accepted),
        "source refresh cannot discard a rejection",
    )?;
    let artifact = task
        .artifact
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("stale task artifact missing".into()))?;
    effects::binding(run, artifact)?;
    effects::lineage(run, task)?;
    let mut candidate = team.clone();
    candidate.prepare_source_refresh(
        task_id,
        crate::team::SourceRefresh {
            approval_scope: "0".repeat(64),
            target_sources: run.source_hashes(),
            target_dependencies: team.dependency_bindings(task)?,
        },
    )?;
    require(
        candidate
            .ready_wave(&run.spec, &gate_context(run))?
            .iter()
            .any(|i| i.task.id == task_id && i.operation == TaskOperation::Execute),
        "source refresh waits for current independently accepted prerequisites/gates",
    )?;
    Ok(hash(serde_json::to_vec(&(
        "refresh_task_sources_v1",
        task_id,
        run,
    ))?))
}
pub(crate) fn prepare(run: &mut Run, task_id: &str, expected_scope: &str) -> Result<()> {
    require(
        scope(run, task_id)? == expected_scope,
        "task source refresh scope mismatch",
    )?;
    let sources = run.source_hashes();
    let team = run.team.as_mut().unwrap();
    let task = team.tasks.iter().find(|t| t.id == task_id).unwrap();
    let dependencies = team.dependency_bindings(task)?;
    team.prepare_source_refresh(
        task_id,
        crate::team::SourceRefresh {
            approval_scope: expected_scope.to_string(),
            target_sources: sources,
            target_dependencies: dependencies,
        },
    )?;
    team.validate(&run.spec)
}

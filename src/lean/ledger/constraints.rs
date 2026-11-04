use super::*;
use crate::lean::tasks::{ChangeKind, DeliveryMode, TaskContract};

pub(crate) fn validate_runtime_constraints(
    tasks: &[TaskContract],
    entries: &[LedgerEntry],
) -> Result<()> {
    for entry in entries
        .iter()
        .filter(|e| e.spec.disposition == Disposition::Active)
    {
        for constraint in &entry.spec.constraints {
            for task in tasks {
                let valid = match constraint {
                    RuntimeConstraint::NoFileChanges => !task.writes,
                    RuntimeConstraint::NoExistingFileChanges => {
                        !task.writes
                            || task.change_kind() == ChangeKind::Artifact
                                && !task.outputs.is_empty()
                                && task.delivery == Some(DeliveryMode::Untracked)
                    }
                    RuntimeConstraint::ArtifactOnly => !task.changes_code(),
                    RuntimeConstraint::UntrackedDelivery => {
                        !task.writes
                            || task.change_kind() == ChangeKind::Artifact
                                && task.delivery == Some(DeliveryMode::Untracked)
                    }
                    RuntimeConstraint::NoNewExecutions => {
                        !task.changes_code()
                            && !task.quality_gate
                            && task.kind != crate::config::WorkKind::Context
                    }
                };
                require(
                    valid,
                    &format!(
                        "task {} conflicts with runtime constraint {:?} from requirement {} (source {}:{}–{})",
                        task.id,
                        constraint,
                        entry.spec.id,
                        entry.spec.source.source_id,
                        entry.spec.source.start,
                        entry.spec.source.end
                    ),
                )?;
            }
        }
    }
    Ok(())
}

/// A revision is not an execution of every unchanged historical contract.
/// Launch-time validation still checks every task before it can run.
pub(crate) fn validate_plan_runtime_constraints(
    specs: &[TaskContract],
    baseline: &[crate::lean::store::Task],
    redo: &[String],
    entries: &[LedgerEntry],
) -> Result<()> {
    let changed = specs
        .iter()
        .filter(|spec| {
            !spec.deferred
                && (redo.contains(&spec.id)
                    || !baseline.iter().any(|old| {
                        let Ok(mut previous) =
                            serde_json::from_value::<TaskContract>(old.spec.clone())
                        else {
                            return false;
                        };
                        if previous.quality_gate && spec.quality_gate {
                            // Finalization rewires existing gates and refreshes their
                            // coverage when an earlier writer becomes a read-only task.
                            // This retains a future obligation, not permission to run it.
                            previous.depends_on.clone_from(&spec.depends_on);
                            previous.requirement_ids.clone_from(&spec.requirement_ids);
                            previous.criterion_ids.clone_from(&spec.criterion_ids);
                            previous.covers.clone_from(&spec.covers);
                        }
                        previous == **spec
                    }))
        })
        .cloned()
        .collect::<Vec<_>>();
    validate_runtime_constraints(&changed, entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn protecting_existing_files_allows_only_new_untracked_documents() {
        let mut entry: LedgerEntry = serde_json::from_value(json!({
            "run_id":"run", "version":1, "spec_revision":1,
            "original":"Do not modify existing files; create a new report",
            "status":"identified", "spec":{"id":"R15",
                "source":{"source_id":"synthetic-message","start":0,"end":53},
                "interpretation":"Preserve existing files and deliver a new report",
                "constraints":["no_existing_file_changes"]}
        }))
        .unwrap();
        let mut task: TaskContract = serde_json::from_value(json!({
            "id":"report", "title":"Report", "goal":"Report findings", "kind":"analysis",
            "writes":true, "change_kind":"artifact",
            "outputs":[".antislop/analysis.md"], "delivery":"untracked"
        }))
        .unwrap();
        let check = |task: &TaskContract, entry: &LedgerEntry| {
            validate_runtime_constraints(std::slice::from_ref(task), std::slice::from_ref(entry))
        };
        assert!(check(&task, &entry).is_ok());
        let decoded: LedgerEntry =
            serde_json::from_value(serde_json::to_value(&entry).unwrap()).unwrap();
        assert_eq!(decoded.spec.constraints, entry.spec.constraints);
        task.delivery = None;
        assert!(check(&task, &entry).is_err());
        task.delivery = Some(DeliveryMode::Untracked);
        task.outputs.clear();
        assert!(check(&task, &entry).is_err());
        task.outputs.push(".antislop/analysis.md".into());
        task.change_kind = Some(ChangeKind::Code);
        assert!(check(&task, &entry).is_err());
        task.change_kind = Some(ChangeKind::Artifact);
        entry.spec.constraints = vec![RuntimeConstraint::NoFileChanges];
        assert!(
            check(&task, &entry).is_err(),
            "legacy no-write constraint remains strict"
        );
        task.writes = false;
        task.change_kind = Some(ChangeKind::None);
        assert!(check(&task, &entry).is_ok());
        entry.spec.constraints = vec![RuntimeConstraint::NoExistingFileChanges];
        assert!(check(&task, &entry).is_ok());
    }
    #[test]
    fn run_limits_bind_generated_work_and_name_the_requirement_source() {
        let entry: LedgerEntry=serde_json::from_value(json!({"run_id":"run","version":1,"spec_revision":1,"original":"Only report findings","status":"identified","spec":{"id":"R1","source":{"source_id":"user-message","start":0,"end":20},"interpretation":"Report only","constraints":["artifact_only","no_new_executions","untracked_delivery"]}})).unwrap();
        let task: TaskContract=serde_json::from_value(json!({"id":"report","title":"Report","goal":"Report findings","kind":"analysis","writes":true,"change_kind":"artifact","outputs":["report.md"],"delivery":"untracked"})).unwrap();
        let finalized =
            crate::lean::tasks::with_options(crate::lean::tasks::with_required_agents(vec![
                task.clone(),
            ]));
        assert_eq!(finalized.len(), 1);
        assert!(validate_runtime_constraints(&finalized, std::slice::from_ref(&entry)).is_ok());
        let mut code = task;
        code.change_kind = Some(ChangeKind::Code);
        let message =
            validate_runtime_constraints(std::slice::from_ref(&code), std::slice::from_ref(&entry))
                .unwrap_err()
                .to_string();
        for field in ["report", "R1", "user-message"] {
            assert!(message.contains(field), "{message}");
        }
        let baseline: crate::lean::store::Task = serde_json::from_value(json!({
            "id":"runtime-task", "plan_id":"plan", "spec":code,
            "status":"approved", "round":1
        }))
        .unwrap();
        let analysis: TaskContract = serde_json::from_value(json!({
            "id":"diagnose", "title":"Diagnose", "goal":"Inspect retained results only", "kind":"analysis", "writes":false
        })).unwrap();
        let proposed = vec![code.clone(), analysis];
        assert!(
            validate_plan_runtime_constraints(
                &proposed,
                std::slice::from_ref(&baseline),
                &[],
                std::slice::from_ref(&entry)
            )
            .is_ok()
        );
        assert!(
            validate_runtime_constraints(std::slice::from_ref(&code), std::slice::from_ref(&entry))
                .is_err(),
            "unchanged historical work still cannot execute under the new restriction"
        );
        assert!(
            validate_plan_runtime_constraints(
                &proposed,
                std::slice::from_ref(&baseline),
                std::slice::from_ref(&code.id),
                std::slice::from_ref(&entry)
            )
            .is_err(),
            "redo is a new execution"
        );
        let mut changed = proposed.clone();
        changed[0].goal = "Change the implementation".into();
        assert!(
            validate_plan_runtime_constraints(
                &changed,
                std::slice::from_ref(&baseline),
                &[],
                std::slice::from_ref(&entry)
            )
            .is_err()
        );
        let mut pending = baseline.clone();
        pending.status = "queued".into();
        let mut gated = proposed.clone();
        gated[0].depends_on.push("diagnose".into());
        gated[0].goal = "Future obligation; stop until separate authorization".into();
        assert!(
            validate_plan_runtime_constraints(
                &gated,
                std::slice::from_ref(&pending),
                &[],
                std::slice::from_ref(&entry)
            )
            .is_err(),
            "a textual condition is not a runtime hold"
        );
        gated[0].deferred = true;
        assert!(
            validate_plan_runtime_constraints(
                &gated,
                std::slice::from_ref(&pending),
                &[],
                std::slice::from_ref(&entry)
            )
            .is_ok()
        );
        assert!(gated[0].require_not_deferred().is_err());
        assert!(
            validate_runtime_constraints(
                std::slice::from_ref(&gated[0]),
                std::slice::from_ref(&entry)
            )
            .is_err()
        );
        let mut released = gated.clone();
        released[0].deferred = false;
        pending.spec = serde_json::to_value(&gated[0]).unwrap();
        assert!(
            validate_plan_runtime_constraints(
                &released,
                &[pending],
                &[],
                std::slice::from_ref(&entry)
            )
            .is_err(),
            "clearing the hold cannot override the active execution restriction"
        );
        // Reproduce the real revision pipeline: narrowing one writer rebuilds
        // existing gates, changing edges/coverage while preserving their behavior.
        let mut future = code.clone();
        future.id = "future-writer".into();
        let original = crate::lean::tasks::with_required_agents(vec![code.clone(), future.clone()]);
        let history = original.iter().map(|spec| serde_json::from_value::<crate::lean::store::Task>(json!({
            "id":format!("runtime-{}", spec.id),"plan_id":"plan","spec":spec,"status":"queued","round":0
        })).unwrap()).collect::<Vec<_>>();
        let mut readonly = proposed[1].clone();
        readonly.id = code.id.clone();
        let mut diagnosis = proposed[1].clone();
        diagnosis.requirement_ids = vec!["R26".into()];
        let later = crate::lean::tasks::with_required_agents(vec![readonly, future, diagnosis]);
        let gate = later
            .iter()
            .find(|task| task.id == "quality-refactoring")
            .unwrap();
        let old_gate = original.iter().find(|task| task.id == gate.id).unwrap();
        assert_ne!(gate.depends_on, old_gate.depends_on);
        assert!(
            validate_plan_runtime_constraints(&later, &history, &[], std::slice::from_ref(&entry))
                .is_ok()
        );
        assert!(
            validate_runtime_constraints(std::slice::from_ref(gate), std::slice::from_ref(&entry))
                .is_err()
        );
        assert!(
            validate_plan_runtime_constraints(
                &later,
                &history,
                std::slice::from_ref(&gate.id),
                std::slice::from_ref(&entry)
            )
            .is_err()
        );
        let mut changed_gate = later.clone();
        changed_gate
            .iter_mut()
            .find(|task| task.id == gate.id)
            .unwrap()
            .goal = "Execute a different gate".into();
        assert!(
            validate_plan_runtime_constraints(
                &changed_gate,
                &history,
                &[],
                std::slice::from_ref(&entry)
            )
            .is_err()
        );
        let mut added = proposed;
        let mut new_code = code.clone();
        new_code.id = "new-code".into();
        added.push(new_code);
        assert!(validate_plan_runtime_constraints(&added, &[baseline], &[], &[entry]).is_err());
    }
}

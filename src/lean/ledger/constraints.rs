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
        let message = validate_runtime_constraints(&[code], &[entry])
            .unwrap_err()
            .to_string();
        for field in ["report", "R1", "user-message"] {
            assert!(message.contains(field), "{message}");
        }
    }
}

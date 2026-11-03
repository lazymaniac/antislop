//! One graph transformation, preserving the approved requirements and immutable history.
use super::*;
use crate::lean::tasks::{TaskContract, order_graph};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanRevision {
    pub plan: String,
    pub base_revision: i64,
    pub reason: String,
    pub tasks: Vec<TaskContract>,
    pub retire: Vec<String>,
    /// Approved tasks whose work must be done again. Without it an approved
    /// task whose contract changes only in wording keeps its result.
    #[serde(default)]
    pub redo: Vec<String>,
    /// New task id -> retired local task id, or an unfinished historical task UUID.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub replacements: std::collections::BTreeMap<String, String>,
}

impl PlanRevision {
    pub fn merge(&self, baseline: &[Task]) -> Result<Vec<TaskContract>> {
        require_revision_reason(self)?;
        let mut specs = baseline_specs(baseline)?;
        self.validate_retire_and_redo(&specs)?;
        for (new, old) in &self.replacements {
            crate::require(
                (self.retire.contains(old)
                    || (uuid::Uuid::parse_str(old).is_ok()
                        && !baseline.iter().any(|t| t.spec["id"] == *old)
                        && self
                            .tasks
                            .iter()
                            .any(|t| &t.id == new && t.writes && t.changes_code())))
                    && self.tasks.iter().any(|t| &t.id == new)
                    && !baseline.iter().any(|t| t.spec["id"] == *new),
                "replacements: use a fresh task id and a retired local source, or a code writer and a historical checkpoint UUID",
            )?;
            crate::require(
                !baseline
                    .iter()
                    .any(|t| t.spec["id"] == *old && t.status == "approved"),
                "replacements: completed tasks cannot be replaced; add a dependent task",
            )?;
        }
        let unique = self
            .replacements
            .values()
            .collect::<std::collections::BTreeSet<_>>();
        crate::require(
            unique.len() == self.replacements.len(),
            "replacements: a task can have only one successor",
        )?;
        remove_retired(&mut specs, self);
        self.merge_updates(&mut specs, baseline)?;
        for spec in &mut specs {
            for dependency in &mut spec.depends_on {
                if let Some((successor, _)) =
                    self.replacements.iter().find(|(_, old)| *old == dependency)
                {
                    *dependency = successor.clone();
                }
            }
        }
        order_graph(crate::lean::tasks::refresh_existing_options(specs))
    }

    fn validate_retire_and_redo(&self, specs: &[TaskContract]) -> Result<()> {
        for id in &self.retire {
            crate::require(
                specs.iter().any(|s| &s.id == id),
                "retired task does not exist",
            )?;
        }
        for id in &self.redo {
            crate::require(
                specs.iter().any(|s| &s.id == id) && !self.retire.contains(id),
                "redo names a task that does not exist or is retired",
            )?;
        }
        Ok(())
    }

    fn merge_updates(&self, specs: &mut Vec<TaskContract>, baseline: &[Task]) -> Result<()> {
        let mut seen = std::collections::BTreeSet::new();
        for task in &self.tasks {
            self.validate_update(task, baseline, &mut seen)?;
            if let Some(detail) = crate::lean::feasibility::forbids_reading(task) {
                return Err(Error::Invalid(format!("task {}: {detail}", task.id)));
            }
            replace_or_append(specs, task);
        }
        Ok(())
    }

    fn validate_update(
        &self,
        task: &TaskContract,
        baseline: &[Task],
        seen: &mut std::collections::BTreeSet<String>,
    ) -> Result<()> {
        crate::require(
            !task.quality_gate
                && task.options_for.is_none()
                && !baseline.iter().any(|old| {
                    old.spec["id"] == task.id
                        && (old.spec["quality_gate"] == true
                            || old.spec["options_for"].as_str().is_some())
                }),
            "generated gates and local options are maintained by the runtime: revise the task an option belongs to, and answer a gate's findings by revising the tasks they concern or adding a task that fixes them; options and gates follow the plan",
        )?;
        crate::require(
            seen.insert(task.id.clone()) && !self.retire.contains(&task.id),
            "revision repeats or retires an updated task",
        )
    }
}

fn require_revision_reason(revision: &PlanRevision) -> Result<()> {
    crate::require(
        !revision.reason.trim().is_empty(),
        "revision.reason is required",
    )
}

fn baseline_specs(baseline: &[Task]) -> Result<Vec<TaskContract>> {
    baseline
        .iter()
        .map(|task| serde_json::from_value(task.spec.clone()))
        .collect::<std::result::Result<_, _>>()
        .map_err(Error::from)
}

fn remove_retired(specs: &mut Vec<TaskContract>, revision: &PlanRevision) {
    specs.retain(|spec| {
        !revision.retire.contains(&spec.id)
            && !spec
                .options_for
                .as_ref()
                .is_some_and(|id| revision.retire.contains(id))
    });
}

fn replace_or_append(specs: &mut Vec<TaskContract>, task: &TaskContract) {
    if let Some(existing) = specs.iter_mut().find(|spec| spec.id == task.id) {
        *existing = task.clone();
    } else {
        specs.push(task.clone());
    }
}

fn same_contract(task: &Task, spec: &TaskContract) -> bool {
    serde_json::from_value::<TaskContract>(task.spec.clone())
        .ok()
        .as_ref()
        == Some(spec)
}

pub(in crate::lean) struct RevisionLease<'a> {
    pub requirement_revision: i64,
    pub request: &'a str,
}

#[path = "plan_revision/apply.rs"]
mod apply;

//! Resolve edits against the exact plan revision shown to the orchestrator.
use super::*;
use crate::lean::tasks::AcceptanceCriterion;
use std::collections::{BTreeMap, BTreeSet};
use store::{FindingAction, FindingDestination};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskPatch {
    task: String,
    goal: Option<String>,
    depends_on: Option<Vec<String>>,
    open_decisions: Option<Vec<String>>,
    #[serde(default)]
    criteria: BTreeMap<String, String>,
    #[serde(default)]
    add_criteria: Vec<AcceptanceCriterion>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestedFindingAction {
    task: String,
    finding: String,
    reason: String,
    destination: Option<FindingDestination>,
}

pub(super) fn preview_graph(
    revision: &store::PlanRevision,
    baseline: &[Task],
    candidate: &[TaskContract],
) -> Value {
    let old: BTreeMap<_, _> = baseline
        .iter()
        .map(|task| (task.spec["id"].as_str().unwrap_or_default(), task))
        .collect();
    let next: BTreeMap<_, _> = candidate
        .iter()
        .map(|task| (task.id.as_str(), task))
        .collect();
    let removed: Vec<_> = old.keys().filter(|id| !next.contains_key(*id)).collect();
    let added: Vec<_> = next.keys().filter(|id| !old.contains_key(*id)).collect();
    let changed: Vec<_> = next.iter().filter_map(|(id, task)| {
        let prior = old.get(id)?;
        let spec: TaskContract = serde_json::from_value(prior.spec.clone()).ok()?;
        (*task != &spec).then(|| json!({
            "id":id,"before_dependencies":spec.depends_on,"after_dependencies":task.depends_on,
            "criteria":task.acceptance_criteria.iter().filter(|item| !spec.acceptance_criteria.contains(item)).map(|item| &item.id).collect::<Vec<_>>()
        }))
    }).collect();
    let removed_options: Vec<_> = removed
        .iter()
        .filter(|id| old[**id].spec["options_for"].is_string())
        .collect();
    json!({"valid":true,"plan":revision.plan,"base_revision":revision.base_revision,
        "added":added,"removed":removed,"removed_options":removed_options,"changed":changed})
}

impl Core {
    pub(super) fn revision_finding_actions(
        &self,
        conversation: &str,
        plan: &str,
        args: &Value,
    ) -> Result<Vec<FindingAction>> {
        if args["finding_actions"].is_null() {
            return Ok(Vec::new());
        }
        let requested: Vec<RequestedFindingAction> =
            serde_json::from_value(args["finding_actions"].clone())?;
        let baseline = self.store.plan(plan)?;
        requested.into_iter().map(|action| {
            let task = baseline.iter().find(|task| task.spec["id"] == action.task)
                .ok_or_else(|| crate::invalid(format!("finding_actions: unknown source task {}", action.task)))?;
            require(!action.reason.trim().is_empty(), "finding_actions: explain the disposition")?;
            let (review_turn, _, verdict) = self.store.previous_task_review_record(conversation, task, "")?
                .ok_or_else(|| crate::invalid(format!("finding_actions: no completed review of {}", action.task)))?;
            require(verdict.findings.iter().any(|entry| entry["id"] == action.finding
                && entry["reproduction"].is_object() && progress::blocks(entry)),
                "finding_actions: name an open reproduced finding from the latest completed review")?;
            Ok(FindingAction {
                source_task: task.id.clone(),
                source_generation: task.input_generation,
                review_turn,
                finding: action.finding,
                reason: action.reason,
                destination: action.destination,
            })
        }).collect()
    }

    pub(super) fn revision_task_changes(
        &self,
        plan: &str,
        revision: i64,
        args: &Value,
    ) -> Result<Vec<TaskContract>> {
        let patches: Vec<TaskPatch> = args.get("patches").map_or_else(
            || Ok(Vec::new()),
            |value| serde_json::from_value(value.clone()),
        )?;
        if !patches.is_empty()
            || args.get("base_revision").is_some()
            || args["finding_actions"]
                .as_array()
                .is_some_and(|actions| !actions.is_empty())
        {
            require(
                args["base_revision"].as_i64() == Some(revision)
                    && self.store.plan_revision(plan)? == revision,
                "base_revision: use the current plan revision shown in task_states",
            )?;
        }
        let baseline = self.store.plan(plan)?;
        let prior: BTreeMap<_, _> = baseline
            .iter()
            .filter_map(|task| task.spec["id"].as_str().map(|id| (id, &task.spec)))
            .collect();
        let mut changed = BTreeSet::new();
        let mut result = Vec::new();
        if let Some(raw) = args.get("tasks") {
            let plans = planned_tasks(raw)?;
            for (task, authored) in plans
                .into_iter()
                .zip(raw.as_array().expect("validated array"))
            {
                let mut task = task.into_contract();
                if let Some(old) = prior.get(task.id.as_str()) {
                    if authored.get("open_decisions").is_none() {
                        task.open_decisions =
                            serde_json::from_value(old["open_decisions"].clone())?;
                    }
                    if authored.get("planning_inputs").is_none() {
                        task.planning_inputs = old.get("planning_inputs").map_or_else(
                            || Ok(Vec::new()),
                            |value| serde_json::from_value(value.clone()),
                        )?;
                    }
                }
                require(changed.insert(task.id.clone()), "revision repeats a task")?;
                result.push(task);
            }
        }
        for patch in patches {
            require(
                changed.insert(patch.task.clone()),
                "revision repeats a task",
            )?;
            let spec = prior
                .get(patch.task.as_str())
                .ok_or_else(|| crate::invalid(format!("patches: unknown task {}", patch.task)))?;
            let mut task: TaskContract = serde_json::from_value((*spec).clone())?;
            require(
                task.options_for.is_none() && !task.quality_gate,
                "patches: revise the authored owner of a generated task",
            )?;
            if let Some(goal) = patch.goal {
                task.goal = goal;
            }
            if let Some(dependencies) = patch.depends_on {
                task.depends_on = dependencies;
            }
            if let Some(decisions) = patch.open_decisions {
                task.open_decisions = Some(decisions);
            }
            for (id, text) in patch.criteria {
                let criterion = task
                    .acceptance_criteria
                    .iter_mut()
                    .find(|item| item.id == id)
                    .ok_or_else(|| {
                        crate::invalid(format!(
                            "task {}: criterion {id} does not exist; use add_criteria",
                            task.id
                        ))
                    })?;
                criterion.text = text;
            }
            task.acceptance_criteria.extend(patch.add_criteria);
            result.push(task);
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_task_patch_preserves_other_criteria_and_previews_removed_options() {
        let (_directory, core, conversation) = super::super::tests::core();
        let writer: TaskContract = serde_json::from_value(json!({
            "id":"writer","title":"Writer","kind":"implementation","change_kind":"code","goal":"Apply choice",
            "open_decisions":["Choose layout"],
            "acceptance_criteria":[{"id":"A","text":"Old evidence"},{"id":"B","text":"Retain this"}]
        })).unwrap();
        let tasks = tasks::with_options(vec![writer]);
        let specs = tasks.iter().map(|task| json!(task)).collect::<Vec<_>>();
        let baseline = core.store.add_plan(&conversation, &specs).unwrap();
        let plan = baseline[0].plan_id.clone();
        let arguments = json!({"base_revision":0,"patches":[{"task":"writer","criteria":{"A":"New evidence"},"open_decisions":[]}]});
        let changes = core.revision_task_changes(&plan, 0, &arguments).unwrap();
        assert_eq!(changes[0].acceptance_criteria[1].text, "Retain this");
        let revision: store::PlanRevision = serde_json::from_value(json!({
            "plan":plan,"base_revision":0,"reason":"Decide layout","tasks":changes,"retire":[]
        }))
        .unwrap();
        let candidate = revision.merge(&baseline).unwrap();
        assert_eq!(
            preview_graph(&revision, &baseline, &candidate)["removed_options"],
            json!(["writer-options"])
        );
        assert!(core.revision_task_changes(&plan, 1, &arguments).is_err());
    }
}

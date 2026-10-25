use super::*;

/// Splits an orchestrator answer into its text and an optional plan.
pub fn parse_plan(answer: &str) -> (String, Option<Result<Vec<TaskSpec>>>) {
    let Some((text, body)) = fenced(answer, "plan") else {
        return (answer.trim().to_owned(), None);
    };
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Plan {
        #[serde(default)]
        run_id: Option<String>,
        #[serde(default)]
        option_id: Option<String>,
        #[serde(default)]
        options_revision: Option<i64>,
        tasks: Vec<TaskSpec>,
    }
    let plan = serde_json::from_str::<Plan>(body)
        .map_err(Error::from)
        .and_then(|plan| {
            let _ = (plan.run_id, plan.option_id, plan.options_revision);
            validate(&plan.tasks)?;
            Ok(plan.tasks)
        });
    (text, Some(plan))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Edit {
    base_plan: String,
    tasks: Vec<TaskSpec>,
}

/// An additive graph revision. Omitted nodes retain their definitions and IDs;
/// changed nodes carry complete specs, so dependency rewiring is explicit.
/// The runtime checks that rewired nodes have not started before applying it.
pub fn parse_graph_edit(
    answer: &str,
    base_plan: &str,
    current: &[TaskSpec],
) -> (String, Option<Result<Vec<TaskSpec>>>) {
    let Some((text, body)) = fenced(answer, "graph") else {
        return (answer.trim().to_owned(), None);
    };
    let edited = serde_json::from_str::<Edit>(body)
        .map_err(Error::from)
        .and_then(|edit| merge_graph_edit(base_plan, current, edit));
    (text, Some(edited))
}

fn merge_graph_edit(base_plan: &str, current: &[TaskSpec], edit: Edit) -> Result<Vec<TaskSpec>> {
    crate::require(
        edit.base_plan == base_plan,
        "graph revision is stale; read the latest plan and retry",
    )?;
    crate::require(!edit.tasks.is_empty(), "graph revision has no changes")?;
    let mut seen = BTreeSet::new();
    let mut merged = current.to_vec();
    for task in edit.tasks {
        merge_graph_task(&mut merged, &mut seen, task)?;
    }
    order_graph(merged)
}

fn merge_graph_task(
    merged: &mut Vec<TaskSpec>,
    seen: &mut BTreeSet<String>,
    task: TaskSpec,
) -> Result<()> {
    crate::require(
        seen.insert(task.id.clone()),
        "duplicate task in graph revision",
    )?;
    if let Some(existing) = merged.iter_mut().find(|existing| existing.id == task.id) {
        *existing = task;
    } else {
        merged.push(task);
    }
    Ok(())
}

/// Stable topological order for edits that insert nodes before existing gates.
pub(crate) fn order_graph(mut remaining: Vec<TaskSpec>) -> Result<Vec<TaskSpec>> {
    let mut ordered = Vec::with_capacity(remaining.len());
    let mut done = BTreeSet::new();
    while !remaining.is_empty() {
        let position = remaining.iter().position(|task| {
            task.depends_on
                .iter()
                .all(|dependency| done.contains(dependency))
        });
        let Some(position) = position else {
            return Err(Error::Invalid(
                "graph contains a cycle or an unknown dependency".into(),
            ));
        };
        let task = remaining.remove(position);
        crate::require(done.insert(task.id.clone()), "duplicate task in graph")?;
        ordered.push(task);
    }
    validate(&ordered)?;
    Ok(ordered)
}

/// Explicit run binding in a planner proposal. Runtime checks ownership.
pub fn proposed_run(answer: &str) -> Option<String> {
    fenced(answer, "plan")
        .and_then(|(_, body)| serde_json::from_str::<Value>(body).ok())
        .and_then(|v| v["run_id"].as_str().map(str::to_owned))
}

/// One step of a ```recover block: how the orchestrator unblocks a task.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Unblocking {
    pub task: String,
    pub action: String,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub spec: Option<TaskSpec>,
    #[serde(default)]
    pub parts: Vec<TaskSpec>,
}

/// Splits the ```recover block off an orchestrator answer. Returns the answer
/// without it and the steps, when the block was there and valid.
pub fn parse_recoveries(answer: &str) -> (String, Vec<Unblocking>) {
    #[derive(Deserialize)]
    struct Block {
        actions: Vec<Unblocking>,
    }
    let Some((text, body)) = fenced(answer, "recover") else {
        return (answer.trim().to_owned(), Vec::new());
    };
    let steps = serde_json::from_str::<Block>(body)
        .map(|block| block.actions)
        .unwrap_or_default();
    (text, steps)
}

pub(crate) fn validate(tasks: &[TaskSpec]) -> Result<()> {
    crate::require(!tasks.is_empty(), "the plan has no tasks")?;
    let mut seen = std::collections::BTreeSet::new();
    for task in tasks {
        validate_task_id(&task.id)?;
        validate_dependencies(task, &seen)?;
        crate::require(
            seen.insert(task.id.as_str()),
            &format!("task id {} repeats", task.id),
        )?;
    }
    Ok(())
}

pub(crate) fn validate_task_id(id: &str) -> Result<()> {
    crate::require(
        !id.is_empty()
            && id.len() <= 256
            && id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
            && id.split('.').all(|fragment| !fragment.is_empty()),
        &format!(
            "invalid task id {id:?}: use 1–256 ASCII letters, digits, hyphens or underscores, optionally separated by single dots (no empty fragments)"
        ),
    )
}

fn validate_dependencies(task: &TaskSpec, seen: &std::collections::BTreeSet<&str>) -> Result<()> {
    for dependency in &task.depends_on {
        crate::require(
            seen.contains(dependency.as_str()),
            &format!(
                "task {} depends on {dependency}, which is not an earlier task",
                task.id
            ),
        )?;
    }
    Ok(())
}

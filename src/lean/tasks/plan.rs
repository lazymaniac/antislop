use super::*;

/// Planning cannot complete with prose or a requirements analyst's ledger block.
pub(crate) fn parse_required_plan(answer: &str) -> (String, Result<Vec<TaskContract>>) {
    let (text, plan) = parse_plan(answer);
    (text, plan.unwrap_or_else(|| Err(Error::Invalid(
        "planning response has no complete fenced plan block; return the complete task plan, not prose or a ledger block. Requirements extraction and ledger corrections belong to the runtime's requirements analyst".into(),
    ))))
}

/// Splits an orchestrator answer into its text and an optional plan.
pub fn parse_plan(answer: &str) -> (String, Option<Result<Vec<TaskContract>>>) {
    let Some((text, body)) = fenced(answer, "plan") else {
        return (answer.trim().to_owned(), None);
    };
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Plan {
        #[serde(default)]
        run_id: Option<String>,
        tasks: Vec<TaskContract>,
    }
    let plan = serde_json::from_str::<Plan>(body)
        .map_err(Error::from)
        .and_then(|mut plan| {
            let _ = plan.run_id;
            drop_unknown_risks(&mut plan.tasks);
            crate::require(
                plan.tasks
                    .iter()
                    .all(|t| !t.quality_gate && t.options_for.is_none()),
                "quality_gate and options_for are assigned by the runtime",
            )?;
            // Dependencies written out of order are the planner's slip, not a
            // reason to lose the plan: order them, and still reject a cycle.
            order_graph(plan.tasks)
        });
    (text, Some(plan))
}

/// A risk ID from another domain selects nothing; the plan review still checks
/// the contract's own risks, so the ID is dropped instead of failing the plan.
fn drop_unknown_risks(tasks: &mut [TaskContract]) {
    for task in tasks {
        let available: BTreeSet<_> = crate::lean::risks::for_kind(task.kind)
            .map(|(id, _)| id)
            .collect();
        task.verifier.risk_ids.retain(|id| available.contains(id));
    }
}

/// Stable topological order for edits that insert nodes before existing gates.
pub(crate) fn order_graph(mut remaining: Vec<TaskContract>) -> Result<Vec<TaskContract>> {
    let mut ordered = Vec::with_capacity(remaining.len());
    let mut done = BTreeSet::new();
    while !remaining.is_empty() {
        let position = remaining.iter().position(|task| {
            task.depends_on
                .iter()
                .all(|dependency| done.contains(dependency))
        });
        let Some(position) = position else {
            let unknown = remaining.iter().find_map(|task| {
                task.depends_on
                    .iter()
                    .find(|dependency| {
                        !done.contains(*dependency)
                            && !remaining.iter().any(|other| &other.id == *dependency)
                    })
                    .map(|dependency| {
                        format!("task {} depends on unknown task {dependency}", task.id)
                    })
            });
            return Err(Error::Invalid(
                unknown.unwrap_or_else(|| "graph contains a dependency cycle".into()),
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

pub(crate) fn validate(tasks: &[TaskContract]) -> Result<()> {
    crate::require(!tasks.is_empty(), "the plan has no tasks")?;
    let mut seen = std::collections::BTreeSet::new();
    for task in tasks {
        validate_task_id(&task.id)?;
        validate_dependencies(task, &seen)?;
        validate_local_contract(task, tasks)?;
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

fn validate_dependencies(
    task: &TaskContract,
    seen: &std::collections::BTreeSet<&str>,
) -> Result<()> {
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

fn validate_local_contract(task: &TaskContract, tasks: &[TaskContract]) -> Result<()> {
    let available: BTreeSet<_> = crate::lean::risks::for_kind(task.kind)
        .map(|(id, _)| id)
        .collect();
    for id in &task.verifier.risk_ids {
        crate::require(
            available.contains(id),
            &format!("task {}: risk {id} is not in this domain", task.id),
        )?;
    }
    let mut names = BTreeSet::new();
    for input in &task.inputs {
        crate::require(
            names.insert(&input.name) && !input.name.trim().is_empty(),
            "input names must be unique and nonempty",
        )?;
        let source = tasks
            .iter()
            .find(|source| source.id == input.task)
            .ok_or_else(|| Error::Invalid(format!("missing input task {}", input.task)))?;
        crate::require(
            task.depends_on.contains(&input.task) && source.outputs.contains(&input.output),
            "inputs must bind declared outputs of direct dependencies",
        )?;
    }
    crate::require(
        task.outputs.iter().all(|name| !name.trim().is_empty())
            && task.outputs.iter().collect::<BTreeSet<_>>().len() == task.outputs.len(),
        "output names must be unique and nonempty",
    )
}

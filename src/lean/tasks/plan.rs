use super::*;

/// A task as a planner writes it through `submit_plan` or `revise_plan`: what
/// to do and where. Everything else is the runtime's: whether it writes
/// follows from its kind unless stated, its verifier checks the goal, the
/// assigned criteria and the kind's risk catalog, and generated options and
/// quality gates are added around it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlannedTask {
    pub id: String,
    pub title: String,
    pub goal: String,
    pub kind: WorkKind,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub scope: Vec<String>,
    #[serde(default)]
    pub writes: Option<bool>,
    #[serde(default)]
    pub covers: Vec<String>,
    #[serde(default)]
    pub section: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
}

impl PlannedTask {
    pub fn into_contract(self) -> TaskContract {
        let writes = self.writes.unwrap_or_else(|| self.kind.writes_by_default());
        TaskContract {
            quality_gate: false,
            options_for: None,
            requirement_ids: Vec::new(),
            criterion_ids: Vec::new(),
            verifier: VerifierSpec {
                summary: Some(verifier_summary(writes).into()),
            },
            id: self.id,
            title: self.title,
            goal: self.goal,
            summary: self.summary,
            section: self.section,
            scope: self.scope,
            writes,
            depends_on: self.depends_on,
            kind: self.kind,
            covers: self.covers,
        }
    }
}

fn verifier_summary(writes: bool) -> &'static str {
    if writes {
        "Verify the change meets the goal, its criteria and the domain risks."
    } else {
        "Verify the result meets the goal, its criteria and the domain risks."
    }
}

/// Authored contracts in dependency order, with generated gate references
/// deferred until policy-aware finalization adds their targets.
pub fn planned_graph(tasks: Vec<PlannedTask>) -> Result<Vec<TaskContract>> {
    let tasks: Vec<_> = tasks.into_iter().map(PlannedTask::into_contract).collect();
    // Generated gates are resolved only after policy and coverage are known.
    // Sort and validate the authored graph without these deferred edges.
    let mut authored = tasks.clone();
    for task in &mut authored {
        task.depends_on.retain(|dependency| {
            tasks.iter().any(|other| &other.id == dependency)
                || !PREDEFINED_AGENT_POOL
                    .iter()
                    .any(|agent| GATE_KINDS.contains(&agent.kind) && agent.id == dependency)
        });
    }
    let mut ordered = order_graph(authored)?;
    for task in &mut ordered {
        task.depends_on = tasks
            .iter()
            .find(|original| original.id == task.id)
            .unwrap()
            .depends_on
            .clone();
    }
    Ok(ordered)
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
        crate::require(
            done.insert(task.id.clone()),
            &format!("task id {} repeats", task.id),
        )?;
        ordered.push(task);
    }
    validate(&ordered)?;
    Ok(ordered)
}

pub(crate) fn validate(tasks: &[TaskContract]) -> Result<()> {
    crate::require(!tasks.is_empty(), "the plan has no tasks")?;
    let mut seen = std::collections::BTreeSet::new();
    for task in tasks {
        validate_task_id(&task.id)?;
        validate_dependencies(task, &seen)?;
        crate::require(
            !task.title.trim().is_empty() && !task.goal.trim().is_empty(),
            &format!("task {}: title and goal must be nonempty", task.id),
        )?;
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

use super::*;

#[path = "artifact_outputs.rs"]
mod artifact_outputs;
pub(crate) use artifact_outputs::validate_artifact_output;

/// A planner-authored task with an explicit delivery purpose. The runtime
/// adds reviewer context, local options and quality gates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlannedTask {
    pub id: String,
    /// Optional immutable writer label: lowercase ASCII letters, digits or
    /// hyphens, 1–48 bytes. Omitted writer labels freeze a task-title slug.
    #[serde(default)]
    pub checkout_name: Option<String>,
    pub title: String,
    pub goal: String,
    pub kind: WorkKind,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub result_inputs: Vec<ResultInput>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub open_decisions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub planning_inputs: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deferred: bool,
    pub change_kind: ChangeKind,
    #[serde(default)]
    pub outputs: Vec<String>,
    #[serde(default)]
    pub delivery: Option<DeliveryMode>,
    #[serde(default)]
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
    #[serde(default)]
    pub section: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
}

impl PlannedTask {
    pub fn into_contract(self) -> TaskContract {
        let checkout_name = if self.change_kind.has_delivery() {
            Some(
                self.checkout_name
                    .unwrap_or_else(|| checkout_slug(&self.title)),
            )
        } else {
            None
        };
        TaskContract {
            quality_gate: false,
            integration_batch: false,
            options_for: None,
            open_decisions: Some(self.open_decisions),
            planning_inputs: self.planning_inputs,
            checkout_name,
            acceptance_criteria: self.acceptance_criteria,
            reviewer: ReviewerSpec {
                summary: Some(reviewer_summary(self.change_kind != ChangeKind::None).into()),
            },
            id: self.id,
            title: self.title,
            goal: self.goal,
            summary: self.summary,
            section: self.section,
            deferred: self.deferred,
            change_kind: self.change_kind,
            outputs: self.outputs,
            delivery: self.delivery,
            depends_on: self.depends_on,
            result_inputs: self.result_inputs,
            kind: self.kind,
        }
    }
}

pub(crate) fn checkout_slug(text: &str) -> String {
    let mut slug = String::new();
    let mut separator = false;
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() {
            let separator_bytes = usize::from(separator && !slug.is_empty());
            if slug.len() + separator_bytes + 1 > 48 {
                break;
            }
            if separator && !slug.is_empty() {
                slug.push('-');
            }
            separator = false;
            slug.push((byte as char).to_ascii_lowercase());
        } else {
            separator = true;
        }
    }
    if slug.is_empty() { "task".into() } else { slug }
}

fn reviewer_summary(delivers_files: bool) -> &'static str {
    if delivers_files {
        "Review the change meets the goal, its criteria and the domain risks."
    } else {
        "Review the result meets the goal, its criteria and the domain risks."
    }
}

/// Authored contracts in dependency order, with generated gate references
/// deferred until policy-aware finalization adds their targets.
pub fn planned_graph(tasks: Vec<PlannedTask>) -> Result<Vec<TaskContract>> {
    planned_graph_with(tasks, Vec::new())
}

/// The same, with the unchanged contracts of approved tasks the plan keeps.
pub fn planned_graph_with(
    tasks: Vec<PlannedTask>,
    kept: Vec<TaskContract>,
) -> Result<Vec<TaskContract>> {
    let mut tasks: Vec<_> = tasks.into_iter().map(PlannedTask::into_contract).collect();
    for spec in kept {
        crate::require(
            !tasks.iter().any(|task| task.id == spec.id),
            &format!(
                "task {} is both kept and planned; a kept task goes only in keep, a task in tasks runs again",
                spec.id
            ),
        )?;
        tasks.push(spec);
    }
    order_authored_graph(tasks)
}

fn order_authored_graph(tasks: Vec<TaskContract>) -> Result<Vec<TaskContract>> {
    let mut authored = tasks.clone();
    for task in &mut authored {
        task.depends_on
            .retain(|dependency| known_authored_dependency(&tasks, dependency));
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

fn known_authored_dependency(tasks: &[TaskContract], dependency: &str) -> bool {
    tasks.iter().any(|task| task.id == dependency)
        || !PREDEFINED_AGENT_POOL
            .iter()
            .any(|agent| GATE_KINDS.contains(&agent.kind) && agent.id == dependency)
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
            return Err(Error::Invalid(graph_failure(&remaining, &done)));
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

fn graph_failure(remaining: &[TaskContract], done: &BTreeSet<String>) -> String {
    remaining
        .iter()
        .find_map(|task| {
            task.depends_on
                .iter()
                .find(|dependency| {
                    !done.contains(*dependency)
                        && !remaining.iter().any(|other| &other.id == *dependency)
                })
                .map(|dependency| format!("task {} depends on unknown task {dependency}", task.id))
        })
        .unwrap_or_else(|| "graph contains a dependency cycle".into())
}

pub(crate) fn validate(tasks: &[TaskContract]) -> Result<()> {
    crate::require(!tasks.is_empty(), "the plan has no tasks")?;
    let mut seen = std::collections::BTreeSet::new();
    for task in tasks {
        validate_task_id(&task.id)?;
        if let Some(name) = &task.checkout_name {
            crate::require(
                task.change_kind.has_delivery() && super::valid_checkout_name(name),
                &format!(
                    "task {}: checkout_name must be a lowercase writer label of 1–48 ASCII letters, digits or hyphens",
                    task.id
                ),
            )?;
        }
        validate_change(task)?;
        if let Some(decisions) = &task.open_decisions {
            let unique: BTreeSet<_> = decisions.iter().map(|decision| decision.trim()).collect();
            crate::require(
                unique.len() == decisions.len() && !unique.contains(""),
                &format!(
                    "task {}: open_decisions must contain distinct nonempty choices",
                    task.id
                ),
            )?;
        }
        validate_acceptance_criteria(task)?;
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

fn validate_change(task: &TaskContract) -> Result<()> {
    crate::require(
        task.change_kind() != ChangeKind::Artifact || !task.outputs.is_empty(),
        &format!("task {}: artifact requires explicit outputs", task.id),
    )?;
    crate::require(
        task.delivery.is_none() || task.change_kind() == ChangeKind::Artifact,
        &format!("task {}: untracked delivery requires artifact", task.id),
    )?;
    if task.change_kind() != ChangeKind::Artifact {
        return Ok(());
    }
    for output in &task.outputs {
        validate_artifact_output(output)
            .map_err(|error| Error::Invalid(format!("task {}: {error}", task.id)))?;
        crate::require(
            !output
                .split('/')
                .next()
                .is_some_and(|part| part.eq_ignore_ascii_case(".antislop"))
                || task.delivery == Some(DeliveryMode::Untracked),
            &format!(
                "task {}: outputs under .antislop require delivery=untracked",
                task.id
            ),
        )?;
    }
    Ok(())
}

fn validate_acceptance_criteria(task: &TaskContract) -> Result<()> {
    crate::require(
        task.quality_gate || task.options_for.is_some() || !task.acceptance_criteria.is_empty(),
        &format!(
            "task {}: acceptance_criteria must contain at least one check",
            task.id
        ),
    )?;
    let mut ids = BTreeSet::new();
    for criterion in &task.acceptance_criteria {
        validate_task_id(&criterion.id)?;
        crate::require(
            criterion.id != GOAL_CHECK
                && !criterion.id.strip_prefix('R').is_some_and(|suffix| {
                    !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
                })
                && ids.insert(&criterion.id),
            &format!(
                "task {}: duplicate or reserved acceptance criterion {}",
                task.id, criterion.id
            ),
        )?;
        crate::require(
            !criterion.text.trim().is_empty(),
            &format!("task {}: criterion {} needs text", task.id, criterion.id),
        )?;
    }
    Ok(())
}

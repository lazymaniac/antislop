use super::*;

pub(super) struct GatePlan {
    prerequisites: Vec<String>,
    deferred: bool,
    prior_gates: Vec<String>,
    refactoring: Option<String>,

    ordered: Vec<(&'static PredefinedAgent, String)>,
}

impl GatePlan {
    pub(super) fn new(
        tasks: &mut [TaskContract],
        mut ordered: Vec<&'static PredefinedAgent>,
        team_gates: &[WorkKind],
    ) -> Self {
        ordered.sort_by_key(|agent| GATE_KINDS.iter().position(|kind| *kind == agent.kind));
        let mut ids = tasks.iter().map(|task| task.id.clone()).collect();
        let ordered: Vec<_> = ordered
            .into_iter()
            .map(|agent| (agent, reserve_task_id(agent.id, &mut ids)))
            .collect();
        retarget_checkpoints(tasks, team_gates, &ordered);
        let referenced: BTreeSet<_> = ordered.iter().map(|(_, id)| id.clone()).collect();
        let (placed, upstream) = upstream_tasks(tasks, referenced);
        Self {
            deferred: tasks
                .iter()
                .any(|task| task.changes_code() && task.deferred),
            // An explicitly referenced checkpoint follows all upstream work,
            // including read-only reviews, but never its own consumers.
            prerequisites: if placed {
                upstream.iter().map(|task| task.id.clone()).collect()
            } else {
                writer_ids(tasks)
            },
            prior_gates: Vec::new(),
            refactoring: None,

            ordered,
        }
    }

    pub(super) fn after_all_writers(mut self, tasks: &[TaskContract]) -> Self {
        self.prerequisites = writer_ids(tasks);
        self
    }

    pub(super) fn next(&mut self) -> Option<TaskContract> {
        if self.ordered.is_empty() {
            return None;
        }
        let (agent, id) = self.ordered.remove(0);
        let dependencies = self.dependencies(agent.kind, &id);
        let description = gate_description(agent);
        let mut task = gate_task(agent, id, dependencies, description);
        task.deferred = self.deferred;
        Some(task)
    }

    fn dependencies(&mut self, kind: WorkKind, id: &str) -> Vec<String> {
        let mut dependencies = self.prerequisites.clone();
        match kind {
            WorkKind::Integration => dependencies.extend(self.prior_gates.iter().cloned()),
            WorkKind::Refactoring => self.refactoring = Some(id.to_owned()),
            _ => dependencies.extend(self.refactoring.iter().cloned()),
        }
        self.prior_gates.push(id.to_owned());
        dependencies
    }
}

/// Resolve an obsolete final integration checkpoint to the effective checks, using their actual
/// reserved ids so authored name collisions cannot bypass a gate.
fn retarget_checkpoints(
    tasks: &mut [TaskContract],
    team_gates: &[WorkKind],
    generated: &[(&PredefinedAgent, String)],
) {
    let disabled: BTreeSet<_> = PREDEFINED_AGENT_POOL
        .iter()
        .filter(|agent| GATE_KINDS.contains(&agent.kind) && !team_gates.contains(&agent.kind))
        .map(|agent| agent.id)
        .collect();
    let authored: BTreeSet<_> = tasks.iter().map(|task| task.id.clone()).collect();
    let enabled: Vec<_> = tasks
        .iter()
        .filter(|task| team_gates.contains(&task.kind))
        .map(|task| task.id.clone())
        .chain(generated.iter().map(|(_, id)| id.clone()))
        .collect();
    if enabled.is_empty() || !tasks.iter().any(TaskContract::changes_code) {
        return;
    }
    for task in tasks {
        let mut dependencies = Vec::new();
        for id in &task.depends_on {
            let replacements = if disabled.contains(id.as_str()) && !authored.contains(id) {
                enabled.as_slice()
            } else {
                std::slice::from_ref(id)
            };
            for dependency in replacements {
                if !dependencies.contains(dependency) {
                    dependencies.push(dependency.clone());
                }
            }
        }
        task.depends_on = dependencies;
    }
}

fn upstream_tasks(
    tasks: &[TaskContract],
    referenced: BTreeSet<String>,
) -> (bool, Vec<TaskContract>) {
    let mut downstream = referenced;
    loop {
        let before = downstream.len();
        for task in tasks {
            if task.depends_on.iter().any(|id| downstream.contains(id)) {
                downstream.insert(task.id.clone());
            }
        }
        if downstream.len() == before {
            break;
        }
    }
    let placed = tasks
        .iter()
        .any(|task| task.options_for.is_none() && downstream.contains(&task.id));
    let upstream = tasks
        .iter()
        .filter(|task| task.options_for.is_none() && !downstream.contains(&task.id))
        .cloned()
        .collect();
    (placed, upstream)
}

fn writer_ids(tasks: &[TaskContract]) -> Vec<String> {
    tasks
        .iter()
        .filter(|task| task.changes_code())
        .map(|task| task.id.clone())
        .collect()
}

struct GateDescription {
    summary: &'static str,
    reviewer_summary: &'static str,
    audit: bool,
}

fn gate_description(agent: &PredefinedAgent) -> GateDescription {
    let (summary, reviewer_summary) = gate_summaries(agent);
    GateDescription {
        summary,
        reviewer_summary,
        audit: is_audit(agent.kind),
    }
}

fn gate_summaries(agent: &PredefinedAgent) -> (&'static str, &'static str) {
    match agent.kind {
        WorkKind::StaticAnalysis => (
            "Check formatting, compilation and lint warnings.",
            "Review that all static checks passed without warnings.",
        ),
        WorkKind::Testing => (
            "Run regression tests and exercise relevant edge cases.",
            "Review test results and coverage of the changed behavior.",
        ),
        WorkKind::Integration => (
            "Check the combined change against every requirement.",
            "Review integration results and delivery of all requirements.",
        ),
        WorkKind::Refactoring => (
            "Simplify the touched code without changing its behavior.",
            "Review behavior is preserved and the structure really improved.",
        ),
        WorkKind::Security => (
            "Audit the changed code paths for security problems.",
            "Review the audit examined the real code paths and backs each finding.",
        ),
        WorkKind::Performance => (
            "Audit the changed code paths for performance problems.",
            "Review the audit examined the real code paths and backs each finding.",
        ),
        _ => (agent.title, agent.focus),
    }
}

fn is_audit(kind: WorkKind) -> bool {
    matches!(kind, WorkKind::Security | WorkKind::Performance)
}

fn gate_task(
    agent: &PredefinedAgent,
    id: String,
    depends_on: Vec<String>,

    description: GateDescription,
) -> TaskContract {
    TaskContract {
        change_kind: if description.audit {
            ChangeKind::None
        } else {
            ChangeKind::Code
        },
        outputs: Vec::new(),
        delivery: None,
        deferred: false,
        quality_gate: true,
        integration_batch: false,
        acceptance_criteria: Vec::new(),
        options_for: None,
        result_inputs: Vec::new(),

        id,
        checkout_name: (!description.audit).then(|| crate::lean::tasks::checkout_slug(agent.title)),
        title: agent.title.to_owned(),
        summary: Some(description.summary.to_owned()),
        goal: gate_goal(agent.focus, description.audit),
        section: Some("Quality Gates".to_owned()),

        depends_on,
        kind: agent.kind,

        reviewer: ReviewerSpec {
            summary: Some(description.reviewer_summary.to_owned()),
        },
    }
}

fn gate_goal(focus: &str, audit: bool) -> String {
    if audit {
        format!(
            "Required audit of the whole change: {focus}. Read the changed code and \
             report every problem found with file, line and evidence, ranked by \
             severity; change nothing."
        )
    } else {
        format!(
            "Required quality gate for the whole change: {focus}. Report every problem \
             found with evidence; fix only what is needed to pass the gate."
        )
    }
}

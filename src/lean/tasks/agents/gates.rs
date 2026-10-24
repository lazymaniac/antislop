use super::*;

pub(super) struct GatePlan {
    writers: Vec<String>,
    scope: Vec<String>,
    prior_gates: Vec<String>,
    refactoring: Option<String>,
    ids: BTreeSet<String>,
    requirement_ids: Vec<String>,
    criterion_ids: Vec<String>,
    ordered: Vec<&'static PredefinedAgent>,
}

impl GatePlan {
    pub(super) fn new(tasks: &[TaskSpec], mut ordered: Vec<&'static PredefinedAgent>) -> Self {
        ordered.sort_by_key(|agent| GATE_KINDS.iter().position(|kind| *kind == agent.kind));
        Self {
            writers: writer_ids(tasks),
            scope: writer_scope(tasks),
            prior_gates: Vec::new(),
            refactoring: None,
            ids: tasks.iter().map(|task| task.id.clone()).collect(),
            requirement_ids: union_ids(tasks, |task| &task.requirement_ids),
            criterion_ids: union_ids(tasks, |task| &task.criterion_ids),
            ordered,
        }
    }

    pub(super) fn next(&mut self) -> Option<TaskSpec> {
        let agent = self.ordered.first().copied()?;
        self.ordered.remove(0);
        let id = reserve_task_id(agent.id, &mut self.ids);
        let dependencies = self.dependencies(agent.kind, &id);
        let description = gate_description(agent);
        Some(gate_task(
            agent,
            id,
            dependencies,
            &self.scope,
            &self.requirement_ids,
            &self.criterion_ids,
            description,
        ))
    }

    fn dependencies(&mut self, kind: WorkKind, id: &str) -> Vec<String> {
        let mut dependencies = self.writers.clone();
        match kind {
            WorkKind::Integration => dependencies.extend(self.prior_gates.iter().cloned()),
            WorkKind::Refactoring => self.refactoring = Some(id.to_owned()),
            _ => dependencies.extend(self.refactoring.iter().cloned()),
        }
        self.prior_gates.push(id.to_owned());
        dependencies
    }
}

fn writer_ids(tasks: &[TaskSpec]) -> Vec<String> {
    tasks
        .iter()
        .filter(|task| task.writes)
        .map(|task| task.id.clone())
        .collect()
}

fn writer_scope(tasks: &[TaskSpec]) -> Vec<String> {
    let mut scope = Vec::new();
    for path in tasks
        .iter()
        .filter(|task| task.writes)
        .flat_map(|task| &task.scope)
    {
        if !scope.contains(path) {
            scope.push(path.clone());
        }
    }
    scope
}

fn union_ids(tasks: &[TaskSpec], select: impl Fn(&TaskSpec) -> &Vec<String>) -> Vec<String> {
    tasks
        .iter()
        .flat_map(|task| select(task).iter().cloned())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

struct GateDescription {
    summary: &'static str,
    verifier_summary: &'static str,
    audit: bool,
}

fn gate_description(agent: &PredefinedAgent) -> GateDescription {
    let (summary, verifier_summary) = gate_summaries(agent);
    GateDescription {
        summary,
        verifier_summary,
        audit: is_audit(agent.kind),
    }
}

fn gate_summaries(agent: &PredefinedAgent) -> (&'static str, &'static str) {
    match agent.kind {
        WorkKind::StaticAnalysis => (
            "Check formatting, compilation and lint warnings.",
            "Verify that all static checks passed without warnings.",
        ),
        WorkKind::Testing => (
            "Run regression tests and exercise relevant edge cases.",
            "Verify test results and coverage of the changed behavior.",
        ),
        WorkKind::Integration => (
            "Check the combined change against every requirement.",
            "Verify integration results and delivery of all requirements.",
        ),
        WorkKind::Refactoring => (
            "Simplify the touched code without changing its behavior.",
            "Verify behavior is preserved and the structure really improved.",
        ),
        WorkKind::Security => (
            "Audit the changed code paths for security problems.",
            "Verify the audit examined the real code paths and backs each finding.",
        ),
        WorkKind::Performance => (
            "Audit the changed code paths for performance problems.",
            "Verify the audit examined the real code paths and backs each finding.",
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
    scope: &[String],
    requirement_ids: &[String],
    criterion_ids: &[String],
    description: GateDescription,
) -> TaskSpec {
    TaskSpec {
        requirement_ids: requirement_ids.to_vec(),
        criterion_ids: criterion_ids.to_vec(),
        id,
        title: agent.title.to_owned(),
        summary: Some(description.summary.to_owned()),
        goal: gate_goal(agent.focus, description.audit),
        section: Some("Quality Gates".to_owned()),
        scope: scope.to_vec(),
        writes: !description.audit,
        depends_on,
        kind: agent.kind,
        verifier: VerifierSpec {
            focus: agent.focus.to_owned(),
            summary: Some(description.verifier_summary.to_owned()),
            risks: Vec::new(),
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

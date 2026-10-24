use super::*;

/// Predefined quality agent role in the standard agent pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PredefinedAgent {
    pub id: &'static str,
    pub kind: WorkKind,
    pub title: &'static str,
    pub focus: &'static str,
    pub required_for_code_changes: bool,
}

/// Predefined pool of specialist and quality-gate agents ensuring that
/// every code change leaves the codebase cleaner and meets all requirements.
pub const PREDEFINED_AGENT_POOL: &[PredefinedAgent] = &[
    PredefinedAgent {
        id: "context-scout",
        kind: WorkKind::Context,
        title: "Context & Contracts Scout",
        focus: "Every claim backed by files read, existing contracts and invariants identified",
        required_for_code_changes: true,
    },
    PredefinedAgent {
        id: "clean-implementer",
        kind: WorkKind::Implementation,
        title: "Implementation & Boy-Scout Refactoring",
        focus: "All requirements met and touched code left cleaner and better structured than before",
        required_for_code_changes: true,
    },
    PredefinedAgent {
        id: "quality-refactoring",
        kind: WorkKind::Refactoring,
        title: "Code Quality & Structure Guardian",
        focus: "Behavior preserved with evidence while simplifying touched modules and removing technical debt",
        required_for_code_changes: true,
    },
    PredefinedAgent {
        id: "static-analyzer",
        kind: WorkKind::StaticAnalysis,
        title: "Static Analysis & Linter Gate",
        focus: "Zero compiler or linter warnings and strict adherence to project formatting and rules",
        required_for_code_changes: true,
    },
    PredefinedAgent {
        id: "verification-testing",
        kind: WorkKind::Testing,
        title: "Regression & Edge-Case Verification",
        focus: "Real code paths and edge cases exercised by automated tests with green runs observed",
        required_for_code_changes: true,
    },
    PredefinedAgent {
        id: "security-auditor",
        kind: WorkKind::Security,
        title: "Security Audit",
        focus: "Changed code paths examined for injection, unsafe input handling, secrets, privilege and trust-boundary problems",
        required_for_code_changes: true,
    },
    PredefinedAgent {
        id: "performance-auditor",
        kind: WorkKind::Performance,
        title: "Performance Audit",
        focus: "Changed code paths examined for needless work, quadratic behavior, blocking calls and memory growth",
        required_for_code_changes: true,
    },
    PredefinedAgent {
        id: "integration-gate",
        kind: WorkKind::Integration,
        title: "End-to-End Integration & Requirements Gate",
        focus: "Combined changes build cleanly, pass all checks, and satisfy every user requirement",
        required_for_code_changes: true,
    },
];

/// Kinds of required agents that check a finished change as a whole, and so
/// can be appended after a plan's writing tasks without reshaping the plan.
fn is_gate_kind(kind: WorkKind) -> bool {
    matches!(
        kind,
        WorkKind::Refactoring
            | WorkKind::StaticAnalysis
            | WorkKind::Testing
            | WorkKind::Security
            | WorkKind::Performance
            | WorkKind::Integration
    )
}

/// The gate kinds `with_required_agents` appends, in the order the runtime
/// works through them (see `gate_depends_on`).
pub const GATE_KINDS: [WorkKind; 6] = [
    WorkKind::Refactoring,
    WorkKind::StaticAnalysis,
    WorkKind::Testing,
    WorkKind::Security,
    WorkKind::Performance,
    WorkKind::Integration,
];

/// Returns the required quality gates from `PREDEFINED_AGENT_POOL` that a plan
/// modifying code (`writes == true`) lacks: no task of the gate's kind exists.
pub fn missing_quality_agents(tasks: &[TaskSpec]) -> Vec<&'static PredefinedAgent> {
    if !tasks.iter().any(|t| t.writes) {
        return Vec::new();
    }
    PREDEFINED_AGENT_POOL
        .iter()
        .filter(|agent| agent.required_for_code_changes && is_gate_kind(agent.kind))
        .filter(|agent| !tasks.iter().any(|t| t.kind == agent.kind))
        .collect()
}

/// Reserve a task ID, keeping the base when free and otherwise trying -2, -3, ….
fn reserve_task_id(base: &str, ids: &mut BTreeSet<String>) -> String {
    // Leave room for collision suffixes when a writer already uses the limit.
    // Normal IDs retain their familiar options/gate names unchanged.
    let bounded;
    let base = if base.len() > 240 {
        bounded = format!("task-{}", crate::hash(base.as_bytes()));
        bounded.as_str()
    } else {
        base
    };
    let mut id = base.to_owned();
    let mut suffix = 2;
    while !ids.insert(id.clone()) {
        id = format!("{base}-{suffix}");
        suffix += 1;
    }
    id
}

/// The plan with every missing required quality gate appended, so a plan that
/// changes code is always statically checked, tested and checked against the
/// user's requirements, and audited for structure, security and performance.
/// Refactoring runs first, after all writing tasks, since it changes code the
/// other gates must judge; static analysis, testing and the read-only audits
/// run in parallel after it; integration runs last, after them too.
pub fn with_required_agents(mut tasks: Vec<TaskSpec>) -> Vec<TaskSpec> {
    let missing = missing_quality_agents(&tasks);
    if missing.is_empty() {
        return tasks;
    }
    let writers: Vec<String> = tasks
        .iter()
        .filter(|t| t.writes)
        .map(|t| t.id.clone())
        .collect();
    let mut scope: Vec<String> = Vec::new();
    for task in tasks.iter().filter(|t| t.writes) {
        for path in &task.scope {
            if !scope.contains(path) {
                scope.push(path.clone());
            }
        }
    }
    let mut gates: Vec<String> = Vec::new();
    // Refactoring goes first and integration last, so the former is a
    // dependency of the other gates and the latter depends on them all.
    let mut ordered = missing;
    ordered.sort_by_key(|agent| GATE_KINDS.iter().position(|kind| *kind == agent.kind));
    let mut refactoring: Option<String> = None;
    let mut ids = tasks.iter().map(|task| task.id.clone()).collect();
    for agent in ordered {
        let id = reserve_task_id(agent.id, &mut ids);
        let mut depends_on = writers.clone();
        match agent.kind {
            WorkKind::Integration => depends_on.extend(gates.iter().cloned()),
            WorkKind::Refactoring => refactoring = Some(id.clone()),
            _ => depends_on.extend(refactoring.iter().cloned()),
        }
        gates.push(id.clone());
        // Audits report; the gates that run builds or restructure code write.
        let audits = matches!(agent.kind, WorkKind::Security | WorkKind::Performance);
        let (summary, verifier_summary) = match agent.kind {
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
        };
        tasks.push(TaskSpec {
            requirement_ids: tasks
                .iter()
                .flat_map(|t| t.requirement_ids.clone())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect(),
            criterion_ids: tasks
                .iter()
                .flat_map(|t| t.criterion_ids.clone())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect(),
            id,
            title: agent.title.to_owned(),
            summary: Some(summary.to_owned()),
            goal: if audits {
                format!(
                    "Required audit of the whole change: {}. Read the changed code and \
                     report every problem found with file, line and evidence, ranked by \
                     severity; change nothing.",
                    agent.focus
                )
            } else {
                format!(
                    "Required quality gate for the whole change: {}. Report every problem \
                     found with evidence; fix only what is needed to pass the gate.",
                    agent.focus
                )
            },
            section: Some("Quality Gates".to_owned()),
            scope: scope.clone(),
            // Gates run builds and checks, which write; a gate that changes
            // nothing leaves the project as it was.
            writes: !audits,
            depends_on,
            kind: agent.kind,
            verifier: VerifierSpec {
                focus: agent.focus.to_owned(),
                summary: Some(verifier_summary.to_owned()),
                risks: Vec::new(),
            },
        });
    }
    tasks
}

/// Add a direct, read-only options prerequisite to every writer lacking one.
/// Run after optional quality gates, including when required gates are disabled.
/// Input order is preserved; valid plans remain topologically ordered.
pub fn with_options(tasks: Vec<TaskSpec>) -> Vec<TaskSpec> {
    let options: BTreeSet<_> = tasks
        .iter()
        .filter(|task| task.kind == WorkKind::Options)
        .map(|task| task.id.clone())
        .collect();
    let mut ids: BTreeSet<_> = tasks.iter().map(|task| task.id.clone()).collect();
    let mut result = Vec::with_capacity(tasks.len());
    for mut task in tasks {
        if task.writes && !task.depends_on.iter().any(|id| options.contains(id)) {
            let id = reserve_task_id(&format!("{}-options", task.id), &mut ids);
            result.push(TaskSpec {
                id: id.clone(),
                requirement_ids: task.requirement_ids.clone(),
                criterion_ids: task.criterion_ids.clone(),
                title: format!("Options for {}", task.title),
                summary: Some("Compare feasible approaches and select the best supported variant.".into()),
                section: Some("Options".into()),
                goal: format!("Choose the best available approach for task {}: {}. Compare 2–4 genuinely different, realistic variants using the same explicit criteria. Exclude variants violating requirements before ranking. Support the selection with evidence and state the condition that would justify a departure. Do not implement anything, invent measurements, or copy repository contents into the report. Read files yourself; dependency reports do not guarantee that unmerged dependency changes are present in this read-only checkout. Name missing evidence and assumptions. End the complete report with exactly one line Selected: <id>, using the chosen variant's id.", task.id, task.goal),
                scope: task.scope.clone(),
                writes: false,
                depends_on: task.depends_on.clone(),
                kind: WorkKind::Options,
                verifier: VerifierSpec {
                    focus: "Check alternatives, common criteria, exclusions, evidence and selection.".into(),
                    summary: Some("Verify the selection follows from feasible alternatives and evidence.".into()),
                    risks: Vec::new(),
                },
            });
            task.depends_on.push(id);
        }
        result.push(task);
    }
    result
}

use super::*;
#[path = "agents/gates.rs"]
mod gates;
use gates::GatePlan;

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
    let mut gates = GatePlan::new(&tasks, missing);
    while let Some(task) = gates.next() {
        tasks.push(task);
    }
    tasks
}

/// Add a direct, read-only options prerequisite to every writer lacking one.
/// Run after optional quality gates, including when required gates are disabled.
/// Input order is preserved; valid plans remain topologically ordered.
pub fn with_options(tasks: Vec<TaskSpec>) -> Vec<TaskSpec> {
    let options = option_ids(&tasks);
    let mut ids: BTreeSet<_> = tasks.iter().map(|task| task.id.clone()).collect();
    let mut result = Vec::with_capacity(tasks.len());
    for mut task in tasks {
        if needs_options(&task, &options) {
            let prerequisite = options_task(&task, &mut ids);
            task.depends_on.push(prerequisite.id.clone());
            result.push(prerequisite);
        }
        result.push(task);
    }
    result
}

fn option_ids(tasks: &[TaskSpec]) -> BTreeSet<String> {
    tasks
        .iter()
        .filter(|task| task.kind == WorkKind::Options)
        .map(|task| task.id.clone())
        .collect()
}

fn needs_options(task: &TaskSpec, options: &BTreeSet<String>) -> bool {
    task.writes && !task.depends_on.iter().any(|id| options.contains(id))
}

fn options_task(task: &TaskSpec, ids: &mut BTreeSet<String>) -> TaskSpec {
    let id = reserve_task_id(&format!("{}-options", task.id), ids);
    TaskSpec {
        id,
        requirement_ids: task.requirement_ids.clone(),
        criterion_ids: task.criterion_ids.clone(),
        title: format!("Options for {}", task.title),
        summary: Some("Compare feasible approaches and select the best supported variant.".into()),
        goal: options_goal(task),
        section: Some("Options".into()),
        scope: task.scope.clone(),
        writes: false,
        depends_on: task.depends_on.clone(),
        kind: WorkKind::Options,
        resolves: None,
        part_of: None,
        verifier: VerifierSpec {
            focus: "Check alternatives, common criteria, exclusions, evidence and selection."
                .into(),
            summary: Some(
                "Verify the selection follows from feasible alternatives and evidence.".into(),
            ),
            risks: Vec::new(),
        },
    }
}

fn options_goal(task: &TaskSpec) -> String {
    format!(
        "Choose the best available approach for task {}: {}. Compare 2–4 genuinely different, realistic variants using the same explicit criteria. Exclude variants violating requirements before ranking. Support the selection with evidence and state the condition that would justify a departure. Do not implement anything, invent measurements, or copy repository contents into the report. Read files yourself; dependency reports do not guarantee that unmerged dependency changes are present in this read-only checkout. Name missing evidence and assumptions. End the complete report with exactly one line Selected: <id>, using the chosen variant's id.",
        task.id, task.goal
    )
}

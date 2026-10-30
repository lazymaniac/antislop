use super::*;

pub fn verifier_prompt(
    project: &str,
    task: &TaskContract,
    report: &str,
    changes: Option<&str>,
) -> String {
    let task_context = verifier_task_context(task);
    let blocker = "Ask the orchestrator about ambiguities. Report unrelated problems separately; do not expand this task.";
    let obligations = obligations_text(&obligations(task));
    let binding = verifier_binding(task);
    let risks = risks_note(task);
    format!(
        "You verify another agent's work. The project is your current working \
         directory: {project}. Check the worker's claims against the actual \
         files yourself with your tools. Work read-only. {REVIEW_RULES} {PROJECT_RULES}\n\n\
         {task_context}\n\n{binding}\n\n\
         Your obligations (the task's goal, its assigned acceptance criteria, then \
         the catalog risks of this kind of work). \
         Assess each from files and recorded evidence only; do not execute or edit:\n{obligations}\n{risks}\n\n\
         Command results are recorded evidence: judge them by the command and the output \
         the report quotes. Files the worker left under paths git ignores (such as \
         .antislop/ or target/) never reach your checkout; their absence is not a finding. \
         Raise one only when the report itself lacks the command or output a check needs.\n\n\
         The worker's report:\n<<<\n{report}\n>>>\n\n{}End with review: your report, checks with a status and note for every obligation id above, and findings, each naming the check it breaks, its severity, the issue and where. The runtime approves only when every check is ok or n/a and no finding is critical, high or medium. Suggestions outside this contract belong in the report, not in findings. {COVERAGE_FORMAT}\n\n{blocker}",
        change_note(changes),
    )
}

fn verifier_binding(task: &TaskContract) -> &'static str {
    if task.writes {
        OPTIONS_VERIFICATION
    } else {
        ""
    }
}

fn change_note(changes: Option<&str>) -> String {
    changes
        .map(|text| format!("{text}\n\n"))
        .unwrap_or_default()
}

fn verifier_task_context(task: &TaskContract) -> String {
    format!(
        "Task: {}\nGoal: {}\nScope: {}",
        task.title,
        task.goal,
        scope_text(&task.scope),
    )
}

/// Resumed verifiers need the same current dependency evidence as new sessions.
pub fn verifier_turn_prompt(
    project: &str,
    task: &TaskContract,
    report: &str,
    changes: Option<&str>,
    fresh: bool,
    upstream: &str,
) -> String {
    let prompt = if fresh {
        verifier_prompt(project, task, report, changes)
    } else {
        verifier_followup(task, report, changes)
    };
    prompt + upstream
}

pub fn verifier_followup(task: &TaskContract, report: &str, changes: Option<&str>) -> String {
    let blocker = "Ask the orchestrator about ambiguities. Report unrelated problems separately; do not expand this task.";
    let binding = verifier_binding(task);
    format!(
        "The worker's updated report:\n<<<\n{report}\n>>>\n\n{}Check it again against \
         the files. {binding} {REVIEW_RULES} {PROJECT_RULES} account again for your obligations (from files and recorded evidence only):\n{}\n{}\n\nEnd with review: your report, checks with a status and note for every obligation id above, and findings, each naming the check it breaks, its severity, the issue and where. The runtime approves only when every check is ok or n/a and no finding is critical, high or medium. Suggestions outside this contract belong in the report, not in findings. {COVERAGE_FORMAT}\n\n{blocker}",
        changes
            .map(|text| format!("{text}\n\n"))
            .unwrap_or_default(),
        obligations_text(&obligations(task)),
        risks_note(task),
    )
}

/// A code review the user asked for on a writer's change.
pub fn code_review_prompt(
    dir: &str,
    task: &TaskContract,
    base: &str,
    result: Option<&str>,
    comments: &[crate::lean::store::Comment],
) -> String {
    let range = review_range(base, result);
    let comments = review_comments(comments);
    format!(
        "You review a code change the user asked you to look at. The repository \
         is your current working directory: {dir}. Read the change with {range} \
         and the surrounding files yourself; work read-only. {PROJECT_RULES}\n\n\
         Task: {}\nGoal: {}\nScope: {}\nAlso check the risks of this kind of work:\n{}\n\n{comments}{}",
        task.title,
        task.goal,
        scope_text(&task.scope),
        obligations_text(&crate::lean::risks::for_kind(task.kind).collect::<Vec<_>>()),
        REVIEW_FORMAT
    )
}

fn review_range(base: &str, result: Option<&str>) -> String {
    match result {
        Some(result) => format!("`git diff {base} {result}`"),
        None => format!("`git diff {base}` (the working tree is the change)"),
    }
}

fn review_comments(comments: &[crate::lean::store::Comment]) -> String {
    if comments.is_empty() {
        return String::new();
    }
    format!(
        "The user's comments on this change:\n{}\n\n",
        comments
            .iter()
            .map(|comment| format!("- on {}: {}", comment.label, comment.text))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

pub fn plan_review_prompt(project: &str, request: &str, plan: &[TaskContract]) -> String {
    format!(
        "You review a plan before any of it runs or the user sees it. The \
         project is your current working directory: {project}. Read the files \
         you need yourself; work read-only. {OUTSIDE}\n\n\
         The user's request:\n<<<\n{}\n>>>\n\n\
         The orchestrator's plan:\n{}\n\n{}{}",
        request,
        plan_text(plan),
        PLAN_CHECKS,
        REVIEW_FORMAT
    )
}

pub fn plan_review_followup(request: &str, plan: &[TaskContract]) -> String {
    format!(
        "{}The orchestrator's new plan:\n{}\n\nCheck it again. {}{}",
        if request.is_empty() {
            String::new()
        } else {
            format!("The user wrote since:\n<<<\n{}\n>>>\n\n", request)
        },
        plan_text(plan),
        PLAN_CHECKS,
        REVIEW_FORMAT
    )
}

const PLAN_CHECKS: &str = "Check that: every part of the request is covered \
     by some task and nothing unrequested is planned; dependencies are complete \
     and ordered (a task needing another's result depends on it; tasks changing \
     the same files or contract are not unordered); no ordering or intermediate \
     state is assumed that the system does not guarantee; no unknown that \
     matters is assumed instead of being established first; each goal is \
     checkable; tasks and verifier criteria do not gate work on commit or tree hashes, \
     snapshot adoption, a clean tree or unchanged scoped paths: agents use their allocated \
     checkout as it is; task.summary, where given, is a concise single-sentence \
     semantic summary (about 60–80 characters) in the user's language, rather than the \
     first sentence of the goal; standard descriptions of built-in roles and automatically \
     added quality gates may use the interface language; scope names the places that matter, \
     including related projects or services; tasks that change files are marked `writes`; key quality and \
     verification agents from the predefined pool are included so touched code \
     is left in a better state; each goal can be verified by inspecting files and recorded \
     evidence only, never by execution or edits. Each verifier checks its task's goal, \
     assigned acceptance criteria and its kind's risk catalog. ";

/// The plan as the reviewer reads it: compact JSON. Tasks the runtime adds
/// itself (options, quality gates) are listed without their standard goals;
/// the reviewer checks that they exist, not their boilerplate.
fn plan_text(plan: &[TaskContract]) -> String {
    let entries: Vec<_> = plan.iter().map(plan_entry).collect();
    serde_json::to_string(&entries).unwrap_or_default()
}

fn plan_entry(task: &TaskContract) -> Value {
    if task.options_for.is_none() && !task.quality_gate {
        return serde_json::to_value(task).unwrap_or_default();
    }
    serde_json::json!({
        "id": task.id,
        "title": task.title,
        "kind": task.kind,
        "writes": task.writes,
        "depends_on": task.depends_on,
        "options_for": task.options_for,
        "quality_gate": task.quality_gate,
        "added_by_runtime": true,
    })
}

/// Review of a plan revision. A resumed reviewer has the plan already; either
/// way the earlier contracts are repeated only for tasks the revision changed
/// or removed, not for the whole plan.
/// What a revision does with approved work: the tasks it keeps and those it
/// sends back to work.
pub struct ApprovedWork<'a> {
    pub kept: &'a [String],
    pub redone: &'a [String],
}

pub fn plan_revision_review_prompt(
    project: &str,
    request: &str,
    plan: &[TaskContract],
    baseline: &[Value],
    approved: ApprovedWork<'_>,
    reason: &str,
    initial: bool,
) -> String {
    let current: Vec<Value> = plan
        .iter()
        .filter_map(|task| serde_json::to_value(task).ok())
        .collect();
    let previous: Vec<_> = baseline
        .iter()
        .filter(|spec| !current.contains(spec))
        .collect();
    let head = if initial {
        plan_review_prompt(project, request, plan)
    } else {
        plan_review_followup(request, plan)
    };
    format!(
        "{head}\nReview a revision within accepted requirements. Preserve every obligation and permission boundary. Validate dependencies, dedicated options for every writer and all quality gates. Reason: {reason}. Earlier contracts of tasks this revision changed or removed: {}\n\nTasks already approved, whose work is done and kept: {}. Judge the revision by the work still to do. Do not ask to change an approved task, or to restate a decision in its contract, unless its result is wrong for the request; a change to its title, goal or summary does not redo its work. When an approved result must be redone, the revision has to name the task in redo.\n\nApproved tasks this revision sends back to work, because their contract changes or it names them in redo; options tasks are generated from the writer they serve and follow its contract: {}.",
        serde_json::to_string(&previous).unwrap_or_default(),
        listed(approved.kept),
        listed(approved.redone)
    )
}

fn listed(ids: &[String]) -> String {
    if ids.is_empty() {
        "none".to_owned()
    } else {
        ids.join(", ")
    }
}

pub fn plan_rejected(findings: &[Value]) -> String {
    format!(
        "The plan verifier sent the plan back before the user saw it:\n{}\n\n\
         Propose a corrected plan, or explain to the user why a finding does \
         not apply.",
        findings_text(findings)
    )
}

pub fn audit_prompt(project: &str, request: &str, results: &str) -> String {
    format!(
        "You audit whether the user's request was fully delivered. The project \
         is your current working directory: {project}. Read the files yourself \
         to check what the results claim; work read-only. {OUTSIDE}\n\n\
         The user's request:\n<<<\n{}\n>>>\n\n\
         Results of the plan that ran:\n{results}\n\n{}{}",
        request, AUDIT_CHECKS, REVIEW_FORMAT
    )
}

pub fn audit_followup(request: &str, results: &str) -> String {
    format!(
        "{}Results of the next plan:\n{results}\n\n{}{}",
        if request.is_empty() {
            String::new()
        } else {
            format!("The user wrote since:\n<<<\n{}\n>>>\n\n", request)
        },
        AUDIT_CHECKS,
        REVIEW_FORMAT
    )
}

const AUDIT_CHECKS: &str = "List every obligation in the request, each \
     qualifier included, and decide for each whether the results and the files \
     show it done and whether touched code was left in a cleaner, better state \
     than before. A missing, partial or unverified obligation is a high \
     finding. ";

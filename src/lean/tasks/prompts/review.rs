use super::*;

pub fn verifier_prompt(
    project: &str,
    task: &TaskSpec,
    report: &str,
    changes: Option<&str>,
) -> String {
    let task_context = verifier_task_context(task);
    let blocker = crate::lean::blocker::FORMAT;
    let obligations = obligations_text(&obligations(task));
    let (binding, risks) = verifier_prompt_notes(task);
    format!(
        "You verify another agent's work. The project is your current working \
         directory: {project}. Check the worker's claims against the actual \
         files yourself with your tools. Work read-only. {REVIEW_RULES} {PROJECT_RULES}\n\n\
         {task_context}\n\n{binding}\n\n\
         Your obligations (the task's risks, the checks of every verification, \
         then those for this kind of work, then the catalog risks of this kind of work):\n{obligations}{risks}\n\n\
         The worker's report:\n<<<\n{report}\n>>>\n\n{}{PROGRESS_FORMAT} {VERDICT_FORMAT} {COVERAGE_FORMAT}\n\n{blocker}",
        change_note(changes),
    )
}

fn verifier_prompt_notes(task: &TaskSpec) -> (&'static str, &'static str) {
    let binding = if task.writes {
        OPTIONS_VERIFICATION
    } else {
        ""
    };
    let risks = if task.verifier.risks.is_empty() {
        "\nNo risks were named for this task; use your judgement for its domain."
    } else {
        ""
    };
    (binding, risks)
}

fn change_note(changes: Option<&str>) -> String {
    changes
        .map(|text| format!("{text}\n\n"))
        .unwrap_or_default()
}

fn verifier_task_context(task: &TaskSpec) -> String {
    format!(
        "Task: {}\nGoal: {}\nScope: {}\nWorker validation to assess from its recorded evidence only: {}",
        task.title,
        task.goal,
        scope_text(&task.scope),
        task.verifier.focus,
    )
}

/// Resumed verifiers need the same current dependency evidence as new sessions.
pub fn verifier_turn_prompt(
    project: &str,
    task: &TaskSpec,
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

pub fn verifier_followup(task: &TaskSpec, report: &str, changes: Option<&str>) -> String {
    let blocker = crate::lean::blocker::FORMAT;
    let binding = if task.writes {
        OPTIONS_VERIFICATION
    } else {
        ""
    };
    format!(
        "The worker's updated report:\n<<<\n{report}\n>>>\n\n{}Check it again against \
         the files. {binding} {REVIEW_RULES} account again for your obligations:\n{}\n\n{PROGRESS_FORMAT} {VERDICT_FORMAT} {COVERAGE_FORMAT}\n\n{blocker}",
        changes
            .map(|text| format!("{text}\n\n"))
            .unwrap_or_default(),
        obligations_text(&obligations(task)),
    )
}

/// A code review the user asked for on a writer's change.
pub fn code_review_prompt(
    dir: &str,
    task: &TaskSpec,
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
         Task: {}\nGoal: {}\nScope: {}\nAlso check: {}\n\n{comments}{}",
        task.title,
        task.goal,
        scope_text(&task.scope),
        checklist(task.kind),
        VERDICT_FORMAT
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

pub fn plan_review_prompt(project: &str, request: &str, plan: &[TaskSpec]) -> String {
    format!(
        "You review a plan before any of it runs or the user sees it. The \
         project is your current working directory: {project}. Read the files \
         you need yourself; work read-only. {OUTSIDE}\n\n\
         The user's request:\n<<<\n{}\n>>>\n\n\
         The orchestrator's plan:\n{}\n\n{}{}",
        request,
        plan_text(plan),
        PLAN_CHECKS,
        VERDICT_FORMAT
    )
}

pub fn plan_review_followup(request: &str, plan: &[TaskSpec]) -> String {
    format!(
        "{}The orchestrator's new plan:\n{}\n\nCheck it again. {}{}",
        if request.is_empty() {
            String::new()
        } else {
            format!("The user wrote since:\n<<<\n{}\n>>>\n\n", request)
        },
        plan_text(plan),
        PLAN_CHECKS,
        VERDICT_FORMAT
    )
}

const PLAN_CHECKS: &str = "Check that: every part of the request is covered \
     by some task and nothing unrequested is planned; dependencies are complete \
     and ordered (a task needing another's result depends on it; tasks changing \
     the same files or contract are not unordered); no ordering or intermediate \
     state is assumed that the system does not guarantee; no unknown that \
     matters is assumed instead of being established first; each goal is \
     checkable; both task.summary and verifier.summary are concise single-sentence \
     semantic summaries (about 60–80 characters), rather than the first sentence \
     of goal or focus; summaries authored by the orchestrator use the user's language, \
     while standard descriptions of built-in roles and automatically added quality gates \
     may use the interface language; scope names the places that matter, including related projects \
     or services; tasks that change files are marked `writes`; key quality and \
     verification agents from the predefined pool are included so touched code \
     is left in a better state; each verifier's focus and risks fit its own task and require only inspection of files and recorded evidence, never execution or edits. ";

fn plan_text(plan: &[TaskSpec]) -> String {
    serde_json::to_string_pretty(plan).unwrap_or_default()
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
        request, AUDIT_CHECKS, VERDICT_FORMAT
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
        VERDICT_FORMAT
    )
}

const AUDIT_CHECKS: &str = "List every obligation in the request, each \
     qualifier included, and decide for each whether the results and the files \
     show it done and whether touched code was left in a cleaner, better state \
     than before. A missing, partial or unverified obligation is a high \
     finding. ";

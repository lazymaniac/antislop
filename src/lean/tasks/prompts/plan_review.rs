use super::*;

pub fn plan_review_prompt(project: &str, request: &str, plan: &[TaskContract]) -> String {
    format!(
        "You review a plan before any of it runs or the user sees it. The \
         project is your current working directory: {project}. Read the files \
         you need yourself; work read-only. {OUTSIDE}\n\n\
         The user's request:\n<<<\n{}\n>>>\n\n\
         The proposed plan:\n{}\n\n{}{}{}",
        request,
        plan_text(plan),
        PLAN_CHECKS,
        super::instructions::FILESYSTEM_GUIDANCE,
        REVIEW_FORMAT
    )
}

pub fn plan_review_followup(request: &str, plan: &[TaskContract]) -> String {
    format!(
        "{}The new plan:\n{}\n\nCheck it again. {}{}{}",
        if request.is_empty() {
            String::new()
        } else {
            format!("The user wrote since:\n<<<\n{}\n>>>\n\n", request)
        },
        plan_text(plan),
        PLAN_CHECKS,
        super::instructions::FILESYSTEM_GUIDANCE,
        REVIEW_FORMAT
    )
}

const PLAN_CHECKS: &str = "Check that: every part of the request is covered \
     by some task and nothing unrequested is planned; dependencies are complete \
     and ordered (a task needing another's result depends on it; tasks changing \
     the same files or contract are not unordered); no ordering or intermediate \
     state is assumed that the system does not guarantee; no unknown that \
     matters is assumed instead of being established first; each goal is \
     checkable; tasks and reviewer criteria do not gate work on commit or tree hashes, \
     snapshot adoption, a clean tree or unchanged input paths: agents use their allocated \
     checkout as it is; task.summary, where given, is a concise single-sentence \
     semantic summary (about 60–80 characters) in the user's language, rather than the \
     first sentence of the goal; standard descriptions of built-in roles and automatically \
     added quality gates may use the interface language; filesystem access follows the allocated checkout and CLI permissions, not task path lists. Writers stay within their worktrees. Existing toolchain executables need no separate path grant. Tasks that change files are marked `writes`; only change_kind=code requires quality and \
     review agents from the predefined pool; none/artifact must not acquire code gates. Analysis results describe failing project checks without requiring repairs. Touched code \
     is left in a better state; each goal's result can be checked by its reviewer from \
     the files and the worker's recorded evidence, without the reviewer executing or editing \
     anything. This limits reviewers, not workers: a task whose kind writes (implementation, \
     refactoring, testing, static analysis, integration) runs the builds, tests, audits and \
     commands its goal names and records their output, so never flag a goal for requiring \
     its worker to execute. Each reviewer checks its task's goal, \
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
        "change_kind": task.change_kind(),
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
        "{head}\nReview a revision within accepted requirements. Preserve every obligation and permission boundary. Validate dependencies under the current runtime policy. Options and quality gates are generated and maintained by the runtime, not authored in a revision. Only change_kind=code is eligible for code options and gates; writes=true alone does not require them. Never demand manual options or code gates for none/artifact tasks, or new options for preserved approved work. Review the generated nodes already shown and their dependencies; do not require nodes the runtime policy disabled. Reason: {reason}. Earlier contracts of tasks this revision changed or removed: {}\n\nTasks already approved, whose work is done and kept: {}. Judge the revision by the work still to do. Do not ask to change an approved task, or to restate a decision in its contract, unless its result is wrong for the request; presentation-only changes preserve approval; a changed goal or requirement triggers reassessment of existing work. An explicit correction request can name redo, but it never discards an integrated change or starts the executor from scratch.\n\nPreviously approved tasks affected by changed contracts or redo: they retain their original report and checkpoint and are reassessed first. Only unresolved obligations go to an executor continuation. Retired tasks remain history and are not runnable; replacements inherit unfinished work. New tasks may be inserted anywhere in the acyclic graph. Generated options must match the current writer objective before review: {}.",
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
        "The plan reviewer sent the plan back before the user saw it:\n{}\n\n\
         Submit the complete corrected plan. If a finding cannot be resolved within \
         the agreed requirements, report the blocker with finish_turn for the orchestrator.",
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
     than before. A missing, partial or unreviewed obligation is a high \
     finding. ";

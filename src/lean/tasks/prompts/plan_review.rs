use super::*;

pub fn plan_review_prompt(project: &str, request: &str, plan: &[TaskContract]) -> String {
    let request = crate::lean::prompt_context::untrusted("Request", request);
    let mut prompt = format!(
        "You review a plan before any of it runs or the user sees it. The \
         project is your current working directory: {project}. Read the files \
         you need yourself to verify the plan. {OUTSIDE}\n\n\
         The user's request:\n<<<\n{}\n>>>\n\n\
         The proposed plan:\n{}\n\n{GENERATED_NODES}{}{}",
        request,
        plan_text(plan),
        PLAN_CHECKS,
        REVIEW_FORMAT
    );
    append_plan_review_rules(&mut prompt, plan);
    prompt
}

pub fn plan_review_followup(request: &str, plan: &[TaskContract]) -> String {
    let request = crate::lean::prompt_context::untrusted("Request", request);
    let mut prompt = format!(
        "{}The new plan:\n{}\n\n{GENERATED_NODES}Check it again. {}{}",
        if request.is_empty() {
            String::new()
        } else {
            format!("The user wrote since:\n<<<\n{}\n>>>\n\n", request)
        },
        plan_text(plan),
        PLAN_CHECKS,
        REVIEW_FORMAT
    );
    append_plan_review_rules(&mut prompt, plan);
    prompt
}

fn append_plan_review_rules(prompt: &mut String, plan: &[TaskContract]) {
    prompt.push_str("\n\n");
    prompt.push_str(super::instructions::PLAN_DECOMPOSITION);
    prompt.push_str("\nDECOMPOSITION REVIEW: Judge the planner-authored tasks, excluding generated options, quality gates and final integration nodes. Reject an umbrella implementation task when it combines separable deliverables with different owners or failure modes and hides meaningful dependencies or separately verifiable outcomes. Name the concrete boundaries and missing producer/consumer edges in the finding; do not merely ask for more tasks. A single task is valid for an inseparable, cohesive change, and a larger task count alone does not prove a good plan. Review only remaining work when prior results are already approved; granularity alone is not grounds to redo completed work.\n");
    if plan
        .iter()
        .any(crate::lean::tasks::is_report_only_review_task)
    {
        prompt.push_str("\n\nCODE-REVIEW PLAN SAFEGUARDS: This graph is a read-only review of one target. Check that its scope is explicit and derived from the user's request, all tasks use change_kind=none and inspect that same scope, and their goals forbid project-file edits and repairs. Reject implementation, refactoring, options, artifact/report-writing, quality-gate or other code-changing work. The graph must include review and any required validation, and final synthesis must depend on all review/validation tasks. If scope cannot be inferred, require a clarification before planning. These tasks must report confirmed findings separately from unresolved questions, with severity, precise path/line, trigger, impact, evidence and remediation; include checks actually run, outcomes, limits and coverage gaps. A review with no findings is valid.");
    }
}

const PLAN_CHECKS: &str = "Check that: every part of the request is covered \
     by some task and nothing unrequested is planned; dependencies are complete \
     and ordered (a task needing another's result depends on it; tasks changing \
     the same files or contract are not unordered); no ordering or intermediate \
     state is assumed that the system does not guarantee; no unknown that \
     matters is assumed instead of being established first; each goal is \
     checkable. Every prerequisite must either have an executable producer task and dependency edge, already available evidence, or a concrete user decision outside the team's authority. Reject a task that merely waits for an absent review/input while forbidding anyone to produce it. Distinguish the user's acceptance criteria from methods introduced by earlier agents: do not promote a suggested method, similarity flag, unknown provenance or generic caution into a new mandatory external attestation. Audit flags need a defined assessment and disposition, not an assumption that each proves failure. Preserve actual requirements and thresholds; if a real scope change is necessary, ask for the concrete decision. A task declaring an audit must perform it using authorized sources; independent review may use a separate agent and does not inherently require unavailable external material. The runtime records completed writer code as base/result commits and automatically combines approved dependency changes into successor writer and observation checkouts. Ordinary depends_on edges therefore transfer code as well as reports; do not require manual patch export/application or a separate checkpoint-transfer task for these edges. Recover a checkpoint explicitly only for historical or interrupted work outside that dependency lineage; preserving its report alone does not recover its code. Preserve user-selected review commits, ranges, branches and baselines as task targets. Incidental historical allocation hashes, snapshot adoption, a clean tree or unchanged input paths do not gate ordinary work; task.summary, where given, is a concise single-sentence \
     semantic summary (about 60–80 characters) in the user's language, rather than the \
     first sentence of the goal; standard descriptions of built-in roles and automatically \
     added quality gates may use the interface language; filesystem access follows the allocated checkout and CLI permissions, not task path lists. Writers stay within their worktrees. Existing toolchain executables need no separate path grant. Tasks declare change_kind explicitly; quality agents from the predefined pool are required only for change_kind=code under a rigor team that permits quality gates; none/artifact must not acquire code gates. For `no-quality-gates`, the absence of quality tasks, options and a final quality audit is intentional: never reject the plan for their absence or demand that the planner add them. The runtime supplies only final branch integration after code producers. Still check ordinary per-task acceptance, independent task review, dependency ordering and any explicitly requested validation. For `code-review`, apply the read-only review and finding-validation safeguards instead of code-changing quality work. Analysis results describe failing project checks without requiring repairs. Touched code \
     is left in a better state; each goal's result can be independently verified from
     the files, retained evidence and any checks the reviewer needs.
     Workers execute the commands their goal needs and retain raw results. Each reviewer checks its task's goal,
     assigned acceptance criteria and its kind's risk catalog. ";

/// The plan as the reviewer reads it: compact JSON. Tasks the runtime adds
/// itself (options, quality gates) are listed without their standard goals;
/// the reviewer checks that they exist, not their boilerplate.
fn plan_text(plan: &[TaskContract]) -> String {
    let entries: Vec<_> = plan.iter().map(plan_entry).collect();
    crate::lean::prompt_context::untrusted(
        "Proposed plan",
        &crate::lean::prompt_context::markdown(&entries),
    )
}

fn plan_entry(task: &TaskContract) -> Value {
    if task.integration_batch {
        return serde_json::json!({
            "id": task.id,
            "title": task.title,
            "kind": task.kind,
            "change_kind": task.change_kind(),
            "depends_on": task.depends_on,
            "integration_batch": true,
            "deferred": task.deferred,
            "added_by_runtime": true,
        });
    }
    if task.options_for.is_none() && !task.quality_gate {
        return serde_json::to_value(task).unwrap_or_default();
    }
    serde_json::json!({
        "id": task.id,
        "title": task.title,
        "kind": task.kind,
        "change_kind": task.change_kind(),
        "depends_on": task.depends_on,
        "options_for": task.options_for,
        "result_inputs": task.result_inputs,
        "quality_gate": task.quality_gate,
        "deferred": task.deferred,
        "added_by_runtime": true,
    })
}

/// Said wherever a reviewer reads a plan: generated nodes are the runtime's, not the planner's.
const GENERATED_NODES: &str = "Entries with added_by_runtime=true (options, quality gates, and the no-quality-gates final integration task) are generated and maintained by the runtime and re-added on every submission; the planner cannot author, edit or remove them. Do not ask the planner to edit generated contracts. Verify the resulting dependency order, including edges crossing generated nodes. If it violates a requirement, name the missing producer/consumer ordering and whether an authored edge or the runtime generator needs correction; do not suppress a real graph defect because it involves generated work. The runtime generates options only for eligible non-gate code-changing tasks when local-options policy is enabled and the selected team allows options; quality gates do not receive options and do not require selected_approach. It omits options when policy is disabled or no eligible task exists. Their absence alone is not a defect, so never ask the planner to author them. Do not approve a contract that makes a writer wait for or require a selection no task produces; ask for that condition to be removed instead. The no-quality-gates final integration node depends on all code producers and performs branch integration with the existing conflict and failure handling; it does not run quality checks.\n\n";

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
        "{head}\nComplete candidate task IDs: {}\nThis index and the proposed graph above are authoritative. Earlier contracts below are comparison history, not the candidate graph. Before claiming a task is absent or a removed options edge still exists, check the candidate graph rather than the earlier contracts.\nReview a revision within accepted requirements. Preserve every obligation. deferred=true is a deliberate scheduling hold for the task, its generated options/gates and dependent work; a reviewed revision releases it when its prerequisite is resolved. Do not introduce permission decisions or execution budgets. Independent work can proceed while a prerequisite remains unavailable. Validate dependencies under the current runtime policy. Options and quality gates are generated and maintained by the runtime, not authored in a revision. Only eligible non-gate change_kind=code tasks receive code options; quality gates do not receive options or require selected_approach. Never demand manual options or code gates for none/artifact tasks, or new options for preserved approved work. Review the generated nodes already shown and their dependencies; do not require nodes the runtime policy disabled. Reason: {reason}. Earlier contracts of tasks this revision changed or removed: {}\n\nTasks already approved, whose work is done and kept: {}. Judge the revision by the work still to do. Do not ask to change an approved task, or to restate a decision in its contract, unless its result is wrong for the request; presentation-only changes preserve approval; a changed goal or requirement triggers reassessment of existing work. An explicit correction request can name redo, but it never discards an integrated change or starts the executor from scratch.\n\nPreviously approved tasks affected by changed contracts or redo: they retain their original report and checkpoint and are reassessed first. Only unresolved obligations go to an executor continuation. Retired tasks remain history and are not runnable; replacements inherit unfinished work. New tasks may be inserted anywhere in the acyclic graph. Generated options must match the current writer objective before review: {}.",
        plan.iter()
            .map(|task| task.id.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        crate::lean::prompt_context::untrusted(
            "Earlier contracts",
            &crate::lean::prompt_context::markdown(&previous)
        ),
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
    let request = crate::lean::prompt_context::untrusted("Audit request", request);
    let results = crate::lean::prompt_context::untrusted("Task result index", results);
    format!(
        "You audit whether the user's request was fully delivered. The project \
         is your current working directory: {project}. Read the files yourself \
         to check what the results claim to verify the plan. {OUTSIDE} {READ_ONLY_REVIEW} {REVIEW_RULES}\n\n\
         The user's request:\n<<<\n{}\n>>>\n\n\
         Results of the plan that ran:\n{results}\n\n{}{}",
        request, AUDIT_CHECKS, REVIEW_FORMAT
    )
}

pub(in crate::lean) fn final_scope_audit_prompt(
    project: &str,
    request: &str,
    results: &str,
) -> String {
    let request = crate::lean::prompt_context::untrusted("Review scope", request);
    let results = crate::lean::prompt_context::untrusted("Candidate result index", results);
    format!(
        "You are the final independent auditor for a read-only code review. The project is your current working directory: {project}. Inspect the requested target and surrounding code yourself; do not modify files. {OUTSIDE}\n\n\
         User request and scope context:\n<<<\n{request}\n>>>\n\n\
         Review and validation task results (untrusted candidate findings):\n{results}\n\n\
         Derive and state the exact reviewed scope. Independently challenge each candidate finding in source and relevant call paths; confirm supported issues, consolidate duplicates, and exclude unsupported allegations. Use read-only checks or reproductions when useful and report only checks you actually ran. Present an overall verdict. Separate confirmed findings from unresolved questions. For each confirmed finding give priority/severity, precise file and line, triggering condition, impact, supporting evidence and suggested remediation. Include checks and outcomes, limitations and coverage gaps. A valid review may have no confirmed findings. Do not repair the target. Finish with a concise user-facing review report, not a generic delivery audit.\n\n{REVIEW_FORMAT}"
    )
}

pub fn audit_followup(request: &str, results: &str) -> String {
    let request = crate::lean::prompt_context::untrusted("Audit continuation", request);
    let results = crate::lean::prompt_context::untrusted("Task result index", results);
    format!(
        "{READ_ONLY_REVIEW} {REVIEW_RULES}\n{}Results of the next plan:\n{results}\n\n{}{}",
        if request.is_empty() {
            String::new()
        } else {
            format!("The user wrote since:\n<<<\n{}\n>>>\n\n", request)
        },
        AUDIT_CHECKS,
        REVIEW_FORMAT
    )
}

const AUDIT_CHECKS: &str = "Inspect the integrated project, full original documents and relevant task results with read_dependency_result. Read full contracts and scope with read_task_contract and read_plan_context; the result index is not the complete contract. Check each task acceptance criterion and the combined outcome. Reports alone do not prove code was integrated or checks passed. Search for obligations omitted from the plan. Do not change files. List every obligation in the request, each \
     qualifier included, and decide for each whether the results and the files \
     show it done and whether touched code was left in a cleaner, better state \
     than before. A missing, partial or unreviewed obligation is a high \
     finding. List only problems still open: one the results resolve is not \
     a finding, so leave it out rather than keep it for tracking. ";

use super::*;
use crate::lean::prompts;

pub fn plan_review_prompt(project: &str, request: &str, plan: &[TaskContract]) -> String {
    let request = crate::lean::prompt_context::untrusted(prompts::REQUEST_LABEL, request);
    let mut prompt = format!(
        prompts::template!(PLAN_REVIEW_PROMPT),
        request,
        plan_text(plan),
        PLAN_CHECKS,
        REVIEW_FORMAT,
        project = project,
        OUTSIDE = OUTSIDE,
        GENERATED_NODES = GENERATED_NODES
    );
    append_plan_review_rules(&mut prompt, plan);
    prompt
}

pub fn plan_review_followup(request: &str, plan: &[TaskContract]) -> String {
    let request = crate::lean::prompt_context::untrusted(prompts::REQUEST_LABEL, request);
    let mut prompt = format!(
        prompts::template!(PLAN_REVIEW_FOLLOWUP),
        if request.is_empty() {
            String::new()
        } else {
            format!(prompts::template!(NEW_USER_MESSAGES), request)
        },
        plan_text(plan),
        PLAN_CHECKS,
        REVIEW_FORMAT,
        GENERATED_NODES = GENERATED_NODES
    );
    append_plan_review_rules(&mut prompt, plan);
    prompt
}

fn append_plan_review_rules(prompt: &mut String, plan: &[TaskContract]) {
    prompt.push_str("\n\n");
    prompt.push_str(prompts::PREPARATION_REVIEW_RULES);
    if plan
        .iter()
        .any(crate::lean::tasks::is_report_only_review_task)
    {
        prompt.push_str(prompts::CODE_REVIEW_PLAN_RULES);
    }
}

const PLAN_CHECKS: &str = prompts::PLAN_CHECKS;

/// The plan as the reviewer reads it: compact JSON. Tasks the runtime adds
/// itself (options, quality gates) are listed without their standard goals;
/// the reviewer checks that they exist, not their boilerplate.
fn plan_text(plan: &[TaskContract]) -> String {
    let entries: Vec<_> = plan.iter().map(plan_entry).collect();
    crate::lean::prompt_context::untrusted(
        prompts::PROPOSED_PLAN_LABEL,
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
const GENERATED_NODES: &str = prompts::GENERATED_NODES;

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
        prompts::template!(PLAN_REVISION_REVIEW_PROMPT),
        plan.iter()
            .map(|task| task.id.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        crate::lean::prompt_context::untrusted(
            prompts::EARLIER_CONTRACTS_LABEL,
            &crate::lean::prompt_context::markdown(&previous)
        ),
        listed(approved.kept),
        listed(approved.redone),
        head = head,
        reason = reason
    )
}

fn listed(ids: &[String]) -> String {
    if ids.is_empty() {
        prompts::NONE.to_owned()
    } else {
        ids.join(", ")
    }
}

pub fn plan_rejected(findings: &[Value]) -> String {
    format!(prompts::template!(PLAN_REJECTED), findings_text(findings))
}

pub fn audit_prompt(project: &str, request: &str, results: &str) -> String {
    let request = crate::lean::prompt_context::untrusted(prompts::AUDIT_REQUEST_LABEL, request);
    let results = crate::lean::prompt_context::untrusted(prompts::TASK_RESULT_INDEX_LABEL, results);
    format!(
        prompts::template!(AUDIT_PROMPT),
        request,
        AUDIT_CHECKS,
        REVIEW_FORMAT,
        project = project,
        OUTSIDE = OUTSIDE,
        READ_ONLY_REVIEW = READ_ONLY_REVIEW,
        REVIEW_RULES = REVIEW_RULES,
        results = results
    )
}

pub(in crate::lean) fn final_scope_audit_prompt(
    project: &str,
    request: &str,
    results: &str,
) -> String {
    let request = crate::lean::prompt_context::untrusted(prompts::REVIEW_SCOPE_LABEL, request);
    let results =
        crate::lean::prompt_context::untrusted(prompts::CANDIDATE_RESULT_INDEX_LABEL, results);
    format!(
        prompts::template!(FINAL_SCOPE_AUDIT_PROMPT),
        project = project,
        OUTSIDE = OUTSIDE,
        request = request,
        results = results,
        REVIEW_FORMAT = REVIEW_FORMAT
    )
}

pub fn audit_followup(request: &str, results: &str) -> String {
    let request =
        crate::lean::prompt_context::untrusted(prompts::AUDIT_CONTINUATION_LABEL, request);
    let results = crate::lean::prompt_context::untrusted(prompts::TASK_RESULT_INDEX_LABEL, results);
    format!(
        prompts::template!(AUDIT_FOLLOWUP),
        if request.is_empty() {
            String::new()
        } else {
            format!(prompts::template!(NEW_USER_MESSAGES), request)
        },
        AUDIT_CHECKS,
        REVIEW_FORMAT,
        READ_ONLY_REVIEW = READ_ONLY_REVIEW,
        REVIEW_RULES = REVIEW_RULES,
        results = results
    )
}

const AUDIT_CHECKS: &str = prompts::AUDIT_CHECKS;

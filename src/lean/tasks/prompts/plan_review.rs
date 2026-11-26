use super::*;
use crate::lean::agent_tools::turn_inputs::{ReadFor, TurnInputs};
use crate::lean::prompts;

pub fn plan_review_prompt(project: &str, request: &str, plan: &[TaskContract]) -> String {
    render_candidate_review(project, request, plan, true, &mut TurnInputs::embedded())
}

pub fn plan_review_followup(request: &str, plan: &[TaskContract]) -> String {
    render_candidate_review("", request, plan, false, &mut TurnInputs::embedded())
}

pub(crate) fn render_candidate_review(
    project: &str,
    request: &str,
    plan: &[TaskContract],
    initial: bool,
    inputs: &mut TurnInputs,
) -> String {
    let request = inputs.block(prompts::REQUEST_LABEL, request, ReadFor::Review);
    let contracts = plan_text(plan, inputs);
    let mut prompt = if initial {
        format!(
            prompts::template!(PLAN_REVIEW_PROMPT),
            request,
            contracts,
            PLAN_CHECKS,
            REVIEW_FORMAT,
            project = project,
            OUTSIDE = OUTSIDE,
            GENERATED_NODES = GENERATED_NODES
        )
    } else {
        format!(
            prompts::template!(PLAN_REVIEW_FOLLOWUP),
            if request.is_empty() {
                String::new()
            } else {
                format!(prompts::template!(NEW_USER_MESSAGES), request)
            },
            contracts,
            PLAN_CHECKS,
            REVIEW_FORMAT,
            GENERATED_NODES = GENERATED_NODES
        )
    };
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
fn plan_text(plan: &[TaskContract], inputs: &mut TurnInputs) -> String {
    let entries: Vec<_> = plan.iter().map(plan_entry).collect();
    let body = crate::lean::prompt_context::markdown(&entries);
    let Some(contracts) = inputs.offload(prompts::PROPOSED_PLAN_LABEL, &body, ReadFor::Review)
    else {
        return crate::lean::prompt_context::untrusted(prompts::PROPOSED_PLAN_LABEL, &body);
    };
    let index: Vec<_> = plan
        .iter()
        .map(|task| {
            serde_json::json!({
                "id": task.id,
                "depends_on": task.depends_on,
            })
        })
        .collect();
    format!(
        "{}\n{contracts}",
        crate::lean::prompt_context::untrusted(
            "Candidate task graph",
            &crate::lean::prompt_context::markdown(&index),
        )
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
    render_revision_review(
        project,
        request,
        plan,
        baseline,
        approved,
        reason,
        initial,
        &mut TurnInputs::embedded(),
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_revision_review(
    project: &str,
    request: &str,
    plan: &[TaskContract],
    baseline: &[Value],
    approved: ApprovedWork<'_>,
    reason: &str,
    initial: bool,
    inputs: &mut TurnInputs,
) -> String {
    let current: Vec<Value> = plan
        .iter()
        .filter_map(|task| serde_json::to_value(task).ok())
        .collect();
    let previous: Vec<_> = baseline
        .iter()
        .filter(|spec| !current.contains(spec))
        .collect();
    let head = render_candidate_review(project, request, plan, initial, inputs);
    format!(
        prompts::template!(PLAN_REVISION_REVIEW_PROMPT),
        plan.iter()
            .map(|task| task.id.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        inputs.block(
            prompts::EARLIER_CONTRACTS_LABEL,
            &crate::lean::prompt_context::markdown(&previous),
            ReadFor::Review,
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
    render_result_audit(project, request, results, &mut TurnInputs::embedded())
}

pub(crate) fn render_result_audit(
    project: &str,
    request: &str,
    results: &str,
    inputs: &mut TurnInputs,
) -> String {
    let request = inputs.block(prompts::AUDIT_REQUEST_LABEL, request, ReadFor::Review);
    let results = inputs.block(
        prompts::TASK_RESULT_INDEX_LABEL,
        results,
        ReadFor::Reference,
    );
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
    inputs: &mut TurnInputs,
) -> String {
    let request = inputs.block(prompts::REVIEW_SCOPE_LABEL, request, ReadFor::Review);
    let results = inputs.block(
        prompts::CANDIDATE_RESULT_INDEX_LABEL,
        results,
        ReadFor::Review,
    );
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
    render_audit_continuation(request, results, &mut TurnInputs::embedded())
}

pub(crate) fn render_audit_continuation(
    request: &str,
    results: &str,
    inputs: &mut TurnInputs,
) -> String {
    let request = inputs.block(prompts::AUDIT_CONTINUATION_LABEL, request, ReadFor::Review);
    let results = inputs.block(
        prompts::TASK_RESULT_INDEX_LABEL,
        results,
        ReadFor::Reference,
    );
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn candidate_and_baseline_pages_are_the_review_snapshot() {
        let (_dir, core, conversation) = crate::lean::agent_tools::tests::core();
        let spec = |goal: String| {
            json!({
                "id":"owner", "title":"Owner", "kind":"implementation", "goal":goal,
                "change_kind":"code", "depends_on":[],
                "acceptance_criteria":[{"id":"C1","text":"The wording is exact"}],
            })
        };
        let baseline = spec("accepted baseline é👩‍💻\n".repeat(900));
        let tasks = core
            .store
            .add_plan(&conversation, std::slice::from_ref(&baseline))
            .unwrap();
        let candidate = spec("candidate draft é👩‍💻\n".repeat(900));
        let candidate: TaskContract = serde_json::from_value(candidate).unwrap();
        let mut inputs = TurnInputs::default();
        let prompt = render_revision_review(
            "project",
            "Preserve the required behavior",
            &[candidate],
            &[baseline],
            ApprovedWork {
                kept: &[],
                redone: &[],
            },
            "revise owner",
            true,
            &mut inputs,
        );
        assert!(prompt.contains("id: owner"));
        assert!(!prompt.contains("candidate draft"));
        assert!(!prompt.contains("accepted baseline"));
        assert!(inputs.require_reads("review").is_err());
        let turn = core
            .store
            .start_turn(
                &conversation,
                crate::lean::PLAN_REVIEWER,
                "fixture",
                0,
                Some(&tasks[0].plan_id),
            )
            .unwrap();
        let read = |inputs: &mut TurnInputs, input: &str| {
            let mut body = String::new();
            let mut cursor = Value::Null;
            loop {
                let page = inputs
                    .read(&turn.id, &json!({"input":input,"cursor":cursor}))
                    .unwrap();
                body.push_str(page["text"].as_str().unwrap());
                if page["complete"] == true {
                    return body;
                }
                cursor = page["cursor"].clone();
            }
        };
        let draft = read(&mut inputs, "input-1");
        assert!(draft.contains("candidate draft"));
        assert!(!draft.contains("accepted baseline"));
        assert!(
            inputs
                .require_reads("review")
                .unwrap_err()
                .to_string()
                .contains("input-2")
        );
        let previous = read(&mut inputs, "input-2");
        assert!(previous.contains("accepted baseline"));
        assert!(!previous.contains("candidate draft"));
        inputs.require_reads("review").unwrap();
    }
}

use super::*;
use crate::lean::agent_tools::turn_inputs::{INLINE_BYTES, ReadFor, TurnInputs};
use crate::lean::prompts;

pub fn reviewer_prompt(
    project: &str,
    task: &TaskContract,
    report: &str,
    changes: Option<&str>,
) -> String {
    let report = crate::lean::prompt_context::untrusted(prompts::WORKER_REPORT_LABEL, report);
    render_reviewer_prompt(project, task, &report, changes)
}

fn render_reviewer_prompt(
    project: &str,
    task: &TaskContract,
    report: &str,
    changes: Option<&str>,
) -> String {
    let task_context = reviewer_task_context(task);
    let blocker = prompts::REVIEWER_BLOCKER_GUIDANCE;
    let obligations = obligations_text(&obligations(task));
    let binding = reviewer_binding(task);
    let risks = risks_note(task);
    let code_review_rules = code_review_rules(task);
    format!(
        prompts::template!(REVIEWER_PROMPT),
        change_note(changes),
        project = project,
        REVIEW_RULES = REVIEW_RULES,
        task_context = task_context,
        binding = binding,
        code_review_rules = code_review_rules,
        obligations = obligations,
        risks = risks,
        report = report,
        COVERAGE_FORMAT = COVERAGE_FORMAT,
        blocker = blocker
    )
}

fn reviewer_binding(task: &TaskContract) -> &'static str {
    if is_report_only_review_task(task) {
        CODE_REVIEW_REVIEW
    } else if task.changes_code() && task.quality_gate {
        GATE_REVIEW
    } else if task.changes_code() && super::worker::has_options_prerequisite(task) {
        OPTIONS_REVIEW
    } else if task.changes_code() {
        prompts::IMPLEMENTATION_REVIEW_BINDING
    } else if task.kind == crate::config::WorkKind::Options {
        SELECTION_REVIEW
    } else {
        REPORT_REVIEW
    }
}

fn code_review_rules(task: &TaskContract) -> &'static str {
    if is_report_only_review_task(task) {
        prompts::CODE_REVIEW_RULES
    } else {
        ""
    }
}

const CODE_REVIEW_REVIEW: &str = prompts::CODE_REVIEW_REVIEW;

fn change_note(changes: Option<&str>) -> String {
    changes
        .map(|text| format!(prompts::template!(CHANGE_NOTE), text = text))
        .unwrap_or_default()
}

fn reviewer_task_context(task: &TaskContract) -> String {
    format!(
        prompts::template!(REVIEWER_TASK_CONTEXT),
        task.title,
        task.id,
        if task.change_kind() == ChangeKind::Artifact {
            task.outputs.join(", ")
        } else {
            prompts::NONE.into()
        },
    )
}

/// Resumed reviewers need the same current dependency evidence as new sessions.
/// `earlier` holds the previous round's findings, whose fixes the reviewer
/// confirms by id.
pub fn reviewer_turn_prompt(
    project: &str,
    task: &TaskContract,
    report: &str,
    changes: Option<&str>,
    fresh: bool,
    upstream: &str,
    earlier: &[Value],
) -> String {
    reviewer_result_prompt(
        project,
        task,
        ReviewReport::Text(report),
        changes,
        fresh,
        upstream,
        earlier,
        &mut TurnInputs::embedded(),
    )
}

pub(crate) enum ReviewReport<'a> {
    Text(&'a str),
    Retained(String),
}

impl<'a> ReviewReport<'a> {
    pub(crate) fn for_task(
        store: &crate::lean::Store,
        task: &crate::lean::Task,
        spec: &TaskContract,
        report: &'a str,
    ) -> Self {
        if !spec.change_kind.has_delivery()
            || spec.kind == WorkKind::Options
            || report.len() <= INLINE_BYTES
        {
            return Self::Text(report);
        }
        let Ok(history) = store.dependency_reports(task) else {
            return Self::Text(report);
        };
        let candidate = history.reports.iter().find(|entry| {
            entry.bytes == report.len() as u64
                && (entry.current_inputs || Some(&entry.result_ref) == history.retained.as_ref())
                && entry.result_type == "completed"
        });
        if let Some(reference) = candidate
            && let Ok(result) = store.referenced_worker_report(reference)
            && result["report"].as_str() == Some(report)
            && !result["selection"].is_object()
        {
            return Self::Retained(reference.result_ref.clone());
        }
        Self::Text(report)
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reviewer_result_prompt(
    project: &str,
    task: &TaskContract,
    report: ReviewReport<'_>,
    changes: Option<&str>,
    fresh: bool,
    upstream: &str,
    earlier: &[Value],
    inputs: &mut TurnInputs,
) -> String {
    let task = inputs.task_contract(task, ReadFor::Review);
    let label = if fresh {
        prompts::WORKER_REPORT_LABEL
    } else {
        prompts::UPDATED_WORKER_REPORT_LABEL
    };
    let report = match report {
        ReviewReport::Text(report) => inputs.block(
            label,
            report,
            if task.change_kind.has_delivery() && task.kind != WorkKind::Options {
                ReadFor::Reference
            } else {
                ReadFor::Review
            },
        ),
        ReviewReport::Retained(reference) => format!(
            "{label}: use read_dependency_result with dependency=self and result_ref={reference}. Follow cursor until complete=true."
        ),
    };
    let prompt = if fresh {
        render_reviewer_prompt(project, &task, &report, changes)
    } else {
        render_reviewer_followup(&task, &report, changes)
    };
    prompt + &earlier_findings(earlier, inputs) + upstream
}

/// The blocking findings of the previous round, so the reviewer can name
/// the ones the updated work fixes: their ids are the runtime's, not its own.
/// A broad finding the round narrowed counts as fixed and comes back as a
/// narrower one, or a large task that shrinks every round looks stalled.
pub(crate) fn earlier_findings(earlier: &[Value], inputs: &mut TurnInputs) -> String {
    let lines: Vec<String> = earlier
        .iter()
        .filter(|f| crate::lean::progress::blocks(f))
        .map(format_earlier_finding)
        .collect();
    if lines.is_empty() {
        return String::new();
    }
    let body = lines.join("\n");
    let findings = match inputs.offload(prompts::EARLIER_FINDINGS_LABEL, &body, ReadFor::Review) {
        Some(reference) => {
            let ids = earlier
                .iter()
                .filter(|finding| crate::lean::progress::blocks(finding))
                .filter_map(|finding| finding["id"].as_str())
                .collect::<Vec<_>>()
                .join(", ");
            format!("Finding IDs: {ids}\n{reference}")
        }
        None => crate::lean::prompt_context::untrusted(prompts::EARLIER_FINDINGS_LABEL, &body),
    };
    format!(prompts::template!(EARLIER_FINDINGS), findings)
}

fn format_earlier_finding(finding: &Value) -> String {
    let issue: String = finding["issue"].as_str().unwrap_or("").to_owned();
    format!(
        prompts::template!(FORMAT_EARLIER_FINDING),
        finding["id"].as_str().unwrap_or(""),
        finding["check"].as_str().unwrap_or("finding"),
        finding["severity"].as_str().unwrap_or(""),
        issue = issue
    )
}

pub fn reviewer_followup(task: &TaskContract, report: &str, changes: Option<&str>) -> String {
    let report =
        crate::lean::prompt_context::untrusted(prompts::UPDATED_WORKER_REPORT_LABEL, report);
    render_reviewer_followup(task, &report, changes)
}

fn render_reviewer_followup(task: &TaskContract, report: &str, changes: Option<&str>) -> String {
    let blocker = prompts::REVIEWER_BLOCKER_GUIDANCE;
    let binding = reviewer_binding(task);
    let code_review_rules = code_review_rules(task);
    format!(
        prompts::template!(REVIEWER_FOLLOWUP),
        changes
            .map(|text| format!(prompts::template!(CHANGE_NOTE), text = text))
            .unwrap_or_default(),
        obligations_text(&obligations(task)),
        risks_note(task),
        report = report,
        binding = binding,
        code_review_rules = code_review_rules,
        REVIEW_RULES = REVIEW_RULES,
        COVERAGE_FORMAT = COVERAGE_FORMAT,
        blocker = blocker
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
        prompts::template!(CODE_REVIEW_PROMPT),
        reviewer_task_context(task),
        obligations_text(&obligations(task)),
        risks_note(task),
        REVIEW_FORMAT,
        dir = dir,
        range = range,
        READ_ONLY_REVIEW = READ_ONLY_REVIEW,
        REVIEW_RULES = REVIEW_RULES,
        PROJECT_RULES = PROJECT_RULES,
        comments = comments
    )
}

fn review_range(base: &str, result: Option<&str>) -> String {
    match result {
        Some(result) => format!(
            prompts::template!(COMMIT_DIFF_RANGE),
            base = base,
            result = result
        ),
        None => format!(prompts::template!(WORKTREE_DIFF_RANGE), base = base),
    }
}

fn review_comments(comments: &[crate::lean::store::Comment]) -> String {
    if comments.is_empty() {
        return String::new();
    }
    format!(
        prompts::template!(REVIEW_COMMENTS),
        comments
            .iter()
            .map(|comment| format!(
                prompts::template!(REVIEW_COMMENT),
                comment.label, comment.text
            ))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reviewer_pages_large_previous_findings_and_keeps_ids_in_the_prompt() {
        let issue = "Earlier report issue é👩‍💻".repeat(600);
        let findings =
            [json!({"id":"goal:previous", "severity":"high", "check":"goal", "issue":issue})];
        let mut inputs = TurnInputs::default();
        let prompt = earlier_findings(&findings, &mut inputs);
        assert!(prompt.contains("Finding IDs: goal:previous"));
        assert!(!prompt.contains("Earlier report issue"));
        assert!(inputs.require_reads("review").is_err());
        let page = inputs
            .read("review-turn", &json!({"input":"input-1"}))
            .unwrap();
        assert!(
            page["text"]
                .as_str()
                .unwrap()
                .contains("Earlier report issue")
        );
    }

    #[test]
    fn reviewer_uses_retained_evidence_only_when_the_report_matches_the_reference() {
        let (_dir, core, conversation) = crate::lean::agent_tools::tests::core();
        let task = core.store.add_plan(&conversation, &[json!({
            "id":"writer", "title":"Writer", "goal":"Preserve drafts", "kind":"implementation",
            "change_kind":"code", "acceptance_criteria":[{"id":"C1","text":"Drafts survive switching"}],
        })]).unwrap().remove(0);
        let spec = serde_json::from_value::<TaskContract>(task.spec.clone()).unwrap();
        let report = "Retained evidence é👩‍💻\n".repeat(600);
        core.store.test_completion(&task, &report);
        let changed = report.replacen("Retained", "Altered!", 1);
        assert!(matches!(
            ReviewReport::for_task(&core.store, &task, &spec, &changed),
            ReviewReport::Text(_)
        ));
        let scoped = ReviewReport::for_task(&core.store, &task, &spec, &report);
        let ReviewReport::Retained(reference) = scoped else {
            panic!("expected a retained report reference")
        };
        let role = format!("reviewer:{}", task.id);
        let turn = core
            .store
            .start_turn(&conversation, &role, "fixture", 0, Some(&task.plan_id))
            .unwrap();
        let result = core
            .agent_tool(
                &conversation,
                &role,
                &turn.id,
                "read_dependency_result",
                &json!({
                    "dependency":"self", "result_ref":reference,
                }),
            )
            .unwrap();
        assert!(report.starts_with(result["report"].as_str().unwrap()));
        assert_eq!(result["complete"], false);
    }
}

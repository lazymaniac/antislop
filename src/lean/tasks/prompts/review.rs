use super::*;
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
    let prompt = if fresh {
        reviewer_prompt(project, task, report, changes)
    } else {
        reviewer_followup(task, report, changes)
    };
    prompt + &earlier_findings(earlier) + upstream
}

/// The blocking findings of the previous round, so the reviewer can name
/// the ones the updated work fixes: their ids are the runtime's, not its own.
/// A broad finding the round narrowed counts as fixed and comes back as a
/// narrower one, or a large task that shrinks every round looks stalled.
pub(crate) fn earlier_findings(earlier: &[Value]) -> String {
    let lines: Vec<String> = earlier
        .iter()
        .filter(|f| crate::lean::progress::blocks(f))
        .map(format_earlier_finding)
        .collect();
    if lines.is_empty() {
        return String::new();
    }
    format!(
        prompts::template!(EARLIER_FINDINGS),
        crate::lean::prompt_context::untrusted(prompts::EARLIER_FINDINGS_LABEL, &lines.join("\n"))
    )
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

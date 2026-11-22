use super::*;

pub fn reviewer_prompt(
    project: &str,
    task: &TaskContract,
    report: &str,
    changes: Option<&str>,
) -> String {
    let report = crate::lean::prompt_context::untrusted("Worker report", report);
    let task_context = reviewer_task_context(task);
    let blocker = "Ask the orchestrator about ambiguities. Report unrelated problems separately; do not expand this task.";
    let obligations = obligations_text(&obligations(task));
    let binding = reviewer_binding(task);
    let risks = risks_note(task);
    let code_review_rules = code_review_rules(task);
    format!(
        "You review another agent's work. The project is your current working \
         directory: {project}. Inspect the actual files and diffs independently; \
         do not modify project files. Check the worker's claims against the actual \
         files yourself with your tools. {REVIEW_RULES} {PROJECT_RULES}\n\n\
         {task_context}\n\n{binding}\n\n{code_review_rules}\n\n\
         Your obligations (the task's goal, its assigned acceptance criteria, then \
         the catalog risks of this kind of work). \
         Assess each from files, recorded evidence and any checks needed to verify it:\n{obligations}\n{risks}\n\n\
         Command results are recorded evidence: judge them by the command and the output \
         the report quotes or retains as immutable MCP execution evidence. Read the referenced \
         records with read_evidence, paging until complete=true, before judging missing raw output. \
         A valid MCP reference retains raw evidence without repeating its body in the report; \
         evaluate the content, not whether it was pasted inline. For earlier reports use \
         read_dependency_result with dependency=self and report_history.result_ref. Historical \
         input reports are context, not current acceptance evidence. Explicit artifact outputs listed in the contract are available \
         in your checkout even under .antislop/; inspect those documents. Other files the \
         worker left under paths git ignores (such as .antislop/ or target/) never reach \
         your checkout; their absence is not a finding. \
         Raise one only when the report itself lacks the command or output a check needs.\n\n\
         The worker's report:\n<<<\n{report}\n>>>\n\n{}End with review: your report, checks with a status and note for every obligation id above, and findings, each naming the check it breaks, its severity, the issue and where. The runtime approves only when every check is ok or n/a and no finding is critical, high or medium. Suggestions outside this contract belong in the report, not in findings. {COVERAGE_FORMAT}\n\n{blocker}",
        change_note(changes),
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
        "Independently verify the implementation against this task's goal and acceptance criteria, actual files and recorded checks. The writer chooses and justifies its approach; no options selection is required."
    } else if task.kind == crate::config::WorkKind::Options {
        SELECTION_REVIEW
    } else {
        REPORT_REVIEW
    }
}

fn code_review_rules(task: &TaskContract) -> &'static str {
    if is_report_only_review_task(task) {
        "Independently validate every candidate finding against the actual target and its surrounding call paths. Confirm its precise file and line, triggering condition, impact and supporting evidence; use read-only builds, tests or reproductions when useful. Remove duplicates and allegations that inspection cannot support. Separate confirmed findings from unresolved questions and explain what evidence is missing for each unresolved item. State the exact reviewed scope, an overall verdict, checks actually run with outcomes, and limitations/coverage gaps. An empty confirmed-findings list is valid when the review supports it. Do not modify project files or make repairs."
    } else {
        ""
    }
}

const CODE_REVIEW_REVIEW: &str = "Code-review task: challenge each candidate finding independently; confirm or reject it from source and evidence, consolidate duplicates, and separate confirmed findings from unresolved questions. Report-only, no repairs.";

fn change_note(changes: Option<&str>) -> String {
    changes
        .map(|text| format!("{text}\n\n"))
        .unwrap_or_default()
}

fn reviewer_task_context(task: &TaskContract) -> String {
    format!(
        "Task: {}\nTask id: {}\nArtifact outputs: {}",
        task.title,
        task.id,
        if task.change_kind() == ChangeKind::Artifact {
            task.outputs.join(", ")
        } else {
            "none".into()
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
        "\n\nYour earlier findings that sent this task back:\n{}\nList in resolved the ids of \
         those the updated work fixes. When it fixed part of one, list that id too and raise a \
         new finding for only what remains, naming the files, functions or cases still wrong \
         instead of repeating the earlier wording. An id whose defect is unchanged stays out \
         of resolved and in findings; a new defect is a new finding, whichever check it falls \
         under.",
        crate::lean::prompt_context::untrusted("Earlier findings", &lines.join("\n"))
    )
}

fn format_earlier_finding(finding: &Value) -> String {
    let issue: String = finding["issue"].as_str().unwrap_or("").to_owned();
    format!(
        "- {} [{}, {}] {issue}",
        finding["id"].as_str().unwrap_or(""),
        finding["check"].as_str().unwrap_or("finding"),
        finding["severity"].as_str().unwrap_or(""),
    )
}

pub fn reviewer_followup(task: &TaskContract, report: &str, changes: Option<&str>) -> String {
    let report = crate::lean::prompt_context::untrusted("Updated worker report", report);
    let blocker = "Ask the orchestrator about ambiguities. Report unrelated problems separately; do not expand this task.";
    let binding = reviewer_binding(task);
    let code_review_rules = code_review_rules(task);
    format!(
        "The worker's updated report:\n<<<\n{report}\n>>>\n\n{}Check it again against \
         the files. {READ_ONLY_REVIEW} {binding} {code_review_rules} {REVIEW_RULES} {PROJECT_RULES} account again for your obligations (from files and recorded evidence only):\n{}\n{}\n\nEnd with review: your report, checks with a status and note for every obligation id above, and findings, each naming the check it breaks, its severity, the issue and where. The runtime approves only when every check is ok or n/a and no finding is critical, high or medium. Suggestions outside this contract belong in the report, not in findings. {COVERAGE_FORMAT}\n\n{blocker}",
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
         and the surrounding files yourself. {READ_ONLY_REVIEW} {REVIEW_RULES} {PROJECT_RULES}\n\n\
         {}\nAll task obligations, including its acceptance criteria:\n{}\n{}\n\n{comments}{}",
        reviewer_task_context(task),
        obligations_text(&obligations(task)),
        risks_note(task),
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

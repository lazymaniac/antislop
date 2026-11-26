use super::*;
use crate::lean::prompts;

// Preserve visible text byte-for-byte, including combining marks and quotes.
// Escape only controls and Unicode line/paragraph separators in metadata so
// they cannot introduce another line or terminal control sequence.
pub(super) fn dependency_metadata(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_control() || matches!(ch, '\u{2028}' | '\u{2029}') {
            escaped.extend(ch.escape_default());
        } else {
            escaped.push(ch);
        }
    }
    escaped
}

#[derive(Debug)]
pub(crate) struct DependencyReport {
    pub id: String,
    pub title: String,
    pub report: String,
}

pub(crate) fn dependency_reports_note(reports: &[DependencyReport]) -> String {
    let records: Vec<_> = reports
        .iter()
        .map(|entry| {
            (
                entry.id.as_str(),
                entry.title.as_str(),
                entry.report.as_str(),
            )
        })
        .collect();
    upstream_note(&records)
}

/// What the tasks this one depends on concluded. Their results live only in
/// the runtime's store, out of reach of an agent reading files, so the short
/// summary of each approved dependency is handed over, never its rounds or its
/// full report; a task with no dependency gets nothing.
pub fn upstream_note(done: &[(&str, &str, &str)]) -> String {
    if done.is_empty() {
        return String::new();
    }
    let marker = dependency_marker(done);
    let mut note = dependency_header(&marker);
    for (index, (id, title, report)) in done.iter().enumerate() {
        note.push_str(&format!(
            prompts::template!(DEPENDENCY_REPORT),
            dependency_metadata(id),
            dependency_metadata(title),
            marker = marker,
            index = index,
            report = report
        ));
    }
    note.push_str(&format!(
        prompts::template!(DEPENDENCY_DATA_END),
        marker = marker
    ));
    note
}

fn dependency_marker(done: &[(&str, &str, &str)]) -> String {
    let mut marker = String::from(crate::lean::prompts::DEPENDENCY_DATA_MARKER);
    while done.iter().any(|(id, title, report)| {
        [*id, *title, *report]
            .iter()
            .any(|text| text.contains(&marker))
    }) {
        marker.push('_');
    }
    marker
}

fn dependency_header(marker: &str) -> String {
    format!(prompts::template!(DEPENDENCY_HEADER), marker = marker)
}

pub fn worker_prompt(project: &str, task: &TaskContract) -> String {
    worker_turn_prompt(project, task, None, "")
}

fn worker_prompt_opening(
    project: &str,
    task: &TaskContract,
    continuation: &str,
    checks: &str,
    binding: &str,
    change_rules: &str,
) -> String {
    format!(
        prompts::template!(WORKER_PROMPT_OPENING),
        task.title,
        task.id,
        work_kind_name(task.kind),
        task.goal,
        change_rules,
        project = project,
        continuation = continuation,
        checks = checks,
        binding = binding
    )
}

fn worker_prompt_closing(blocker: &str) -> String {
    format!(prompts::template!(WORKER_PROMPT_CLOSING), blocker = blocker)
}

/// What the independent reviewer will check, so the worker meets it up front.
fn worker_checks(task: &TaskContract) -> String {
    let mut checks = obligations(task);
    if let Some((_, goal)) = checks.iter_mut().find(|(id, _)| id == GOAL_CHECK) {
        *goal = prompts::GOAL_CHECK_DESCRIPTION.into();
    }
    format!(
        prompts::template!(WORKER_REVIEW_CHECKS),
        obligations_text(&checks),
        risks_note(task)
    )
}

fn worker_change_rules(task: &TaskContract) -> &'static str {
    if is_report_only_review_task(task) {
        prompts::READ_ONLY_CODE_REVIEW_WORKER_RULES
    } else if task.changes_code() {
        prompts::WRITER_CHANGE_RULES
    } else if task.change_kind() == ChangeKind::Artifact {
        ""
    } else if task.kind == WorkKind::Options {
        prompts::OPTIONS_WORKER_RULES
    } else if task.kind == WorkKind::Context {
        prompts::CONTEXT_WORKER_RULES
    } else {
        prompts::ANALYSIS_WORKER_RULES
    }
}

/// Every worker turn carries current dependency data, even after a session reset
/// or when a continuation is delivered through the inbox.
pub fn worker_turn_prompt(
    project: &str,
    task: &TaskContract,
    continuation: Option<&str>,
    upstream: &str,
) -> String {
    let mut presented = task.clone();
    normalize_options_presentation(&mut presented);
    let task = &presented;
    let (change_rules, blocker, binding, checks) = worker_prompt_parts(task);
    let prompt = worker_prompt_opening(
        project,
        task,
        continuation.unwrap_or_default(),
        &checks,
        binding,
        change_rules,
    );
    let artifact = if task.change_kind() == ChangeKind::Artifact {
        format!(
            prompts::template!(ARTIFACT_DELIVERY_RULES),
            task.outputs.join(", ")
        )
    } else {
        String::new()
    };
    let commit_instruction = if task.change_kind.has_delivery() {
        prompts::DELIVERY_COMMIT_MESSAGE_RULES
    } else {
        prompts::REPORT_COMMIT_MESSAGE_RULES
    };
    let delivery = if task.kind == WorkKind::Delivery {
        prompts::DELIVERY_WORKER_RULES
    } else {
        ""
    };
    prompt
        + delivery
        + &artifact
        + upstream
        + "\n\n"
        + commit_instruction
        + &worker_prompt_closing(blocker)
}

/// The reason for resuming a task, alongside its explicit round handoff.
pub fn worker_resume(findings: &[Value]) -> String {
    if findings.is_empty() {
        prompts::INTERRUPTED_WORKER_CONTINUATION.into()
    } else {
        format!(
            prompts::template!(REVIEWED_WORKER_CONTINUATION),
            prompts::UNRESOLVED_FINDINGS_REFERENCE
        )
    }
}

/// Whether a CLI's error means it waits for the user (a login, a usage or
/// rate limit) rather than that the task went wrong.
pub fn needs_user_action(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    [
        "[provider_limit]",
        "[login_required]",
        "5-hour limit",
        "login",
        "log in",
        "logged in",
        "authenticat",
        "unauthorized",
        "api key",
        "credentials",
        "rate limit",
        "rate_limit",
        "usage limit",
        "quota",
        "too many requests",
        "credit balance",
    ]
    .iter()
    .any(|marker| error.contains(marker))
}

pub fn worker_followup(_findings: &[Value]) -> String {
    format!(
        prompts::template!(WORKER_FOLLOWUP),
        prompts::UNRESOLVED_FINDINGS_REFERENCE,
        prompts::WORKER_RESULT_TOOL_RULES,
    )
}

/// How a reviewer finds a writer's changes: the commit range, not the diff.
pub fn changes_text(base: &str, result: &str, stat: &str) -> String {
    format!(
        prompts::template!(CHANGES_TEXT),
        base = base,
        result = result,
        stat = stat
    )
}

fn worker_prompt_parts(task: &TaskContract) -> (&'static str, &'static str, &'static str, String) {
    let blocker = prompts::WORKER_BLOCKER_RULES;
    let binding = if task.changes_code() && has_options_prerequisite(task) {
        OPTIONS_BINDING
    } else {
        ""
    };
    (
        worker_change_rules(task),
        blocker,
        binding,
        worker_checks(task),
    )
}

pub(super) fn has_options_prerequisite(task: &TaskContract) -> bool {
    let base = format!("{}-options", task.id);
    let base = if base.len() > 240 {
        format!("task-{}", crate::hash(base.as_bytes()))
    } else {
        base
    };

    task.depends_on.iter().any(|dependency| {
        dependency == &base
            || dependency
                .strip_prefix(&base)
                .and_then(|suffix| suffix.strip_prefix('-'))
                .is_some_and(|suffix| suffix.parse::<usize>().is_ok_and(|number| number >= 2))
    })
}

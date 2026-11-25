use crate::lean::prompts;
use crate::lean::*;

/// Current local failures and pending questions for orchestration.
pub(in crate::lean) fn blocked_report(tasks: &[Task]) -> String {
    let blocked = blocked_entries(tasks, |status| {
        matches!(
            status,
            status::COORDINATING
                | status::CANCELLED
                | status::SKIPPED
                | status::QUEUED
                | status::NEEDS_USER
                | status::WAITING
                | status::FAILED
        )
    });
    format!(
        prompts::template!(BLOCKED_REPORT),
        blocked = blocked,
        COORDINATION_GUIDANCE = COORDINATION_GUIDANCE
    )
}

/// Tasks that stopped for coordination while the rest of the plan runs on.
pub(in crate::lean) fn stopped_report(tasks: &[Task]) -> Option<String> {
    let stopped = blocked_entries(tasks, stopped_for_coordination);
    (stopped != "[]").then(|| {
        format!(
            prompts::template!(STOPPED_REPORT),
            stopped = stopped,
            COORDINATION_GUIDANCE = COORDINATION_GUIDANCE
        )
    })
}

fn stopped_for_coordination(status: &str) -> bool {
    matches!(
        status,
        status::COORDINATING | status::NEEDS_USER | status::WAITING | status::FAILED
    )
}

fn blocked_entries(tasks: &[Task], include: impl Fn(&str) -> bool) -> String {
    let blocked: Vec<_> = tasks
        .iter()
        .filter(|task| include(&task.status) || task.spec["deferred"] == true && include(status::QUEUED))
        .map(|task| {
            json!({
                "task": task.spec["id"],
                "status": task.status,
                "deferred": task.spec["deferred"] == true,
                "reason": task.current_reason().map(|text| tasks::bound(text, 2000)),
                "waiting_request": task.feedback.as_ref().and_then(|feedback| feedback.get("waiting_request")),
                "findings": task.verdict.as_ref().and_then(|verdict| verdict.get("findings")).map(|value| tasks::bound(&crate::lean::prompt_context::markdown(value), 4000)),
            })
        })
        .collect();
    if blocked.is_empty() {
        "[]".into()
    } else {
        crate::lean::prompt_context::untrusted(
            prompts::CURRENT_BLOCKERS_LABEL,
            &crate::lean::prompt_context::markdown(&blocked),
        )
    }
}

const COORDINATION_GUIDANCE: &str = prompts::COORDINATION_GUIDANCE;

pub(in crate::lean) fn orchestrator_prompt(
    project: &Path,
    first: bool,
    messages: &[Message],
    open: &[Comment],
) -> String {
    let mut prompt = if first {
        initial_orchestrator_prompt(project)
    } else {
        String::new()
    };
    prompt.push_str(&format_messages(messages));
    append_open_comments(&mut prompt, open);
    prompt
}

fn initial_orchestrator_prompt(project: &Path) -> String {
    format!(
        prompts::template!(INITIAL_ORCHESTRATOR_PROMPT),
        project.display(),
        tasks::orchestrator_instructions()
    )
}

pub(in crate::lean) fn format_messages(messages: &[Message]) -> String {
    messages
        .iter()
        .map(format_message)
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn format_message(message: &Message) -> String {
    match message.author.as_str() {
        RUNTIME => format!(
            prompts::template!(RUNTIME_MESSAGE),
            message.seq,
            tasks::bound(&message.text, 12000)
        ),
        PLAN_REVIEWER => format!(prompts::template!(PLAN_REVIEWER_MESSAGE), message.text),
        PLANNER_BLOCKER => format!(prompts::template!(PLANNER_BLOCKER_MESSAGE), message.text),
        REPLAN => format!(prompts::template!(REPLAN_MESSAGE), message.text),
        "user" => format!(prompts::template!(USER_MESSAGE), message.text),
        _ => message.text.clone(),
    }
}

fn append_open_comments(prompt: &mut String, open: &[Comment]) {
    if open.is_empty() {
        return;
    }
    prompt.push_str(OPEN_COMMENT_HEADER);
    for comment in open {
        prompt.push_str(&format!(
            prompts::template!(OPEN_COMMENT),
            comment.id, comment.label, comment.text
        ));
    }
}

const OPEN_COMMENT_HEADER: &str = prompts::OPEN_COMMENT_HEADER;

#[cfg(test)]
mod stopped_report_tests {
    use super::*;

    fn task(id: &str, task_status: &str) -> Task {
        serde_json::from_value(json!({
            "id": id,
            "plan_id": "plan",
            "spec": {"id": id},
            "status": task_status,
            "round": 1,
        }))
        .unwrap()
    }

    #[test]
    fn stopped_report_lists_coordination_states_and_notes_remaining_work() {
        let report = stopped_report(&[
            task("waiting", status::WAITING),
            task("queued", status::QUEUED),
            task("skipped", status::SKIPPED),
        ])
        .unwrap();
        assert!(report.contains("waiting"), "{report}");
        assert!(
            report.contains("Other tasks have not finished yet"),
            "{report}"
        );
        assert!(!report.contains("queued"), "{report}");
        assert!(!report.contains("skipped"), "{report}");
    }

    #[test]
    fn oversized_runtime_diagnostics_do_not_overflow_orchestration_prompts() {
        let raw = "binary patch failed\n".repeat(130_000);
        let mut failed = task("failed", status::FAILED);
        failed.feedback = Some(json!({"reason":raw}));
        let report = blocked_report(&[failed]);
        assert!(report.len() < 5000, "{}", report.len());
        assert!(report.contains("binary patch failed"));
        let message: Message = serde_json::from_value(json!({
            "seq":198,"conversation_id":"conversation","author":RUNTIME,"text":raw,"created_ms":0
        }))
        .unwrap();
        assert!(format_message(&message).len() < 13000);
        assert_eq!(
            message.text, raw,
            "full diagnostic remains available in history"
        );
    }

    #[test]
    fn stopped_report_is_absent_when_only_queued_tasks_remain() {
        assert!(stopped_report(&[task("queued", status::QUEUED)]).is_none());
    }
}

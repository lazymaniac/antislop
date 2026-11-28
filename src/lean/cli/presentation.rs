use super::super::tui::view::clean;
use serde_json::Value;
use std::io::Write;

pub(super) struct PresentedPlan {
    pub(super) id: String,
    pub(super) revision_token: Option<String>,
}

pub(super) fn latest_plan(events: &[Value]) -> Option<PresentedPlan> {
    events
        .iter()
        .rev()
        .find(|event| {
            matches!(
                event["kind"].as_str(),
                Some("plan_proposed" | "scope_revision_resolved")
            )
        })
        .and_then(|event| {
            let payload = &event["payload"];
            Some(PresentedPlan {
                id: payload["plan"].as_str()?.to_owned(),
                revision_token: if event["kind"] == "plan_proposed"
                    && payload["scope_revision"].is_string()
                {
                    payload["revision_token"].as_str().map(str::to_owned)
                } else {
                    None
                },
            })
        })
}

/// One terminal line for an event, if it is worth one.
pub fn describe(event: &Value) -> Option<String> {
    let payload = &event["payload"];
    let kind = event["kind"].as_str()?;
    match kind {
        "message" => describe_message(payload),
        "turn_started" => Some(format!(
            "· {} ({}) is working…",
            value(payload, "role").split(':').next().unwrap_or_default(),
            value(payload, "provider")
        )),
        "plan_proposed" => Some(describe_plan(payload)),
        "audit" => Some(review_lines("audit", payload).join("\n")),
        "task" => Some(describe_task(payload)),
        "turn_finished" if payload["status"] != "done" => Some(format!(
            "✗ {}: {}",
            value(payload, "status"),
            value(payload, "error")
        )),
        _ => describe_status(kind, payload),
    }
}

fn describe_status(kind: &str, payload: &Value) -> Option<String> {
    match kind {
        "paused" if payload["paused"] == true => {
            Some("⏸ paused: new steps wait, /continue resumes".into())
        }
        "paused" => Some("▶ resumed".into()),
        "provider_activity" if payload["state"] != "progress" => Some(value(payload, "text")),
        "notice" | "runtime_activation" => Some(format!("⚠ {}", value(payload, "text"))),
        "session_cleanup_pending" => Some(format!("⚠ {}", value(payload, "text"))),
        "session_finalizing" => Some(format!(
            "All tasks and the final audit passed. Finalizing branch {} and removing temporary session resources.",
            value(payload, "branch")
        )),
        "session_completed" if !payload["branch"].is_string() => Some(format!(
            "Session completed. {}\nReports and evidence are available in session history for {}. Temporary agent resources released.",
            value(payload, "summary"),
            value(payload, "project")
        )),
        "session_completed" => Some(format!(
            "Session completed. {}\nRepository: {}\nFinal branch: {}\nReview: git diff main...{}\nContinue: git switch {}\nTemporary task branches, worktrees and agent processes removed.",
            value(payload, "summary"),
            value(payload, "project"),
            value(payload, "branch"),
            value(payload, "branch"),
            value(payload, "branch")
        )),
        "mail"
            if payload["kind"] != "copy"
                && (payload["sender"] == "user" || payload["recipient"] == "user") =>
        {
            Some(describe_mail(payload))
        }
        "mail_delivered" => Some(format!("✓ delivered messages {}", payload["ids"])),
        "mail_rejected" => Some(format!(
            "✗ message to {} undeliverable: {}",
            value(payload, "recipient"),
            value(payload, "reason")
        )),
        "plan_invalid" => Some(format!("✗ plan rejected: {}", value(payload, "error"))),
        _ => None,
    }
}

fn value(payload: &Value, key: &str) -> String {
    payload[key].as_str().unwrap_or_default().to_owned()
}

fn describe_message(payload: &Value) -> Option<String> {
    if payload["internal"] == true {
        return None;
    }
    if payload["author"] == "runtime" {
        return Some("· task results handed to orchestrator".into());
    }
    if payload["author"] != "orchestrator" {
        return None;
    }
    Some(format!(
        "\n{}:\n{}\n",
        value(payload, "author"),
        value(payload, "text")
    ))
}

fn describe_plan(payload: &Value) -> String {
    let mut lines = review_lines("plan reviewer", &payload["review"]);
    lines.insert(0, format!("Plan:\n{}", plan_tasks(payload)));
    lines.push(plan_status(payload));
    lines.join("\n")
}

fn plan_tasks(payload: &Value) -> String {
    payload["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|task| {
            format!(
                "  [{}] {} ({}{})",
                task["id"].as_str().unwrap_or_default(),
                task["title"].as_str().unwrap_or_default(),
                task["kind"].as_str().unwrap_or_default(),
                if task["change_kind"]
                    .as_str()
                    .is_some_and(|kind| kind != "none")
                {
                    ", modifies files"
                } else {
                    ""
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn plan_status(payload: &Value) -> String {
    if payload["automatic"] == true {
        return "Running.".to_owned();
    }
    if payload["review"]["approved"] == true {
        return "Plan reviewed; waiting for your acceptance.".to_owned();
    }
    if let Some(error) = payload["review"]["error"].as_str() {
        return format!("Plan review failed: {error}");
    }
    payload["waiting"].as_str().map_or_else(
        || "Plan received.".to_owned(),
        |reason| format!("Plan review: {reason}."),
    )
}

fn describe_mail(payload: &Value) -> String {
    format!(
        "✉ {} → {} ({}): {}",
        value(payload, "sender"),
        value(payload, "recipient"),
        value(payload, "kind"),
        value(payload, "text")
    )
}

fn describe_task(payload: &Value) -> String {
    if payload["status"] == "approved" {
        let selection = &payload["feedback"]["selection"];
        if let Some(summary) = selection["user_summary"]
            .as_str()
            .filter(|text| !text.trim().is_empty())
        {
            return format!("✓ {}: {}", value(payload, "title"), clean(summary));
        }
        if let Some(approach) = selection["approach"].as_str() {
            return format!(
                "✓ {}: selected {}",
                value(payload, "title"),
                clean(approach)
            );
        }
        if let Some(message) = payload["feedback"]["integration"]["commit_message"].as_str() {
            return format!(
                "✓ Completed: {}\nCommit: {}",
                value(payload, "title"),
                clean(message)
            );
        }
    }
    let reason = match payload["reason"].as_str() {
        Some(reason) if payload["status"] == "waiting" => {
            format!(" — {reason}; agent CLI is waiting for login or rate limit reset, then /resume")
        }
        Some(reason) => format!(" — {reason}"),
        None => String::new(),
    };
    format!(
        "· [{}] {}: {} (round {}){}",
        value(payload, "id"),
        value(payload, "title"),
        value(payload, "status"),
        payload["round"],
        reason
    )
}

/// What a reviewer (plan reviewer, auditor) concluded, for the user.
pub fn review_lines(who: &str, review: &Value) -> Vec<String> {
    if review.is_null() {
        return vec![];
    }
    if let Some(error) = review["error"].as_str() {
        return vec![format!("⚠ {who} failed: {error}")];
    }
    let findings = review_findings(review);
    let mut lines = vec![if review["approved"] == true {
        format!("✓ {who}: no issues")
    } else {
        format!("⚠ {who}: issues found")
    }];
    if review["approved"] == true && !findings.is_empty() {
        lines[0] = format!("✓ {who}: minor notes");
    }
    if who == "audit"
        && let Some(report) = review["report"]
            .as_str()
            .filter(|report| !report.trim().is_empty())
    {
        lines.extend(report.lines().map(|line| format!("  {line}")));
    }
    lines.extend(findings);
    lines.extend(coverage_lines(review));
    lines
}

fn review_findings(review: &Value) -> Vec<String> {
    review["findings"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|finding| match finding.as_str() {
            Some(text) => format!("  - {text}"),
            None => format!(
                "  - [{}] {}",
                finding["severity"].as_str().unwrap_or("?"),
                finding["issue"].as_str().unwrap_or_default()
            ),
        })
        .collect()
}

fn coverage_lines(review: &Value) -> Vec<String> {
    let Some(coverage) = review["coverage"].as_object().filter(|map| !map.is_empty()) else {
        return vec![];
    };
    let mut lines = vec!["  Obligation coverage:".into()];
    for (id, entry) in coverage {
        let mark = match entry["status"]
            .as_str()
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("ok") => "✓",
            Some("issue") => "✗",
            Some("blocked") => "!",
            Some("n/a") => "–",
            _ => "?",
        };
        lines.push(format!(
            "    {mark} {id}: {}",
            entry["note"].as_str().unwrap_or_default()
        ));
    }
    lines
}

pub(super) fn print(event: &Value) {
    if let Some(line) = describe(event) {
        let mut out = std::io::stdout().lock();
        // Agents' text may carry terminal escapes (design §13).
        let _ = writeln!(out, "{}", clean(&line));
        let _ = out.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn event_presentation_keeps_message_filtering_and_plan_guidance() {
        assert_eq!(
            describe(&json!({"kind":"message","payload":{"author":"user","text":"hello"}})),
            None
        );
        assert_eq!(
            describe(&json!({"kind":"message","payload":{"author":"runtime"}})).as_deref(),
            Some("· task results handed to orchestrator")
        );
        let plan = describe(&json!({"kind":"plan_proposed","payload":{"tasks":[{"id":"R1","title":"Split","kind":"implementation","change_kind":"code"}],"review":null,"automatic":false}})).unwrap();
        assert!(plan.contains("[R1] Split (implementation, modifies files)"));
        assert!(plan.ends_with("Plan received."));
    }

    #[test]
    fn reviewer_coverage_keeps_status_marks_and_notes() {
        let lines = review_lines(
            "auditor",
            &json!({"approved":false,"findings":[],"coverage":{"R1":{"status":"issue","note":"missing check"}}}),
        );
        assert_eq!(
            lines,
            [
                "⚠ auditor: issues found",
                "  Obligation coverage:",
                "    ✗ R1: missing check"
            ]
        );
    }

    #[test]
    fn audit_report_is_visible_with_a_clean_verdict_and_without_claiming_checks() {
        let lines = review_lines(
            "audit",
            &json!({"approved":true,"findings":[],"report":"Scope: src/lib.rs\nVerdict: no confirmed findings\nChecks: none run; coverage gap stated."}),
        );
        assert_eq!(lines[0], "✓ audit: no issues");
        assert!(lines.iter().any(|line| line.contains("Scope: src/lib.rs")));
        assert!(lines.iter().any(|line| line.contains("Checks: none run")));
        let normal = review_lines(
            "reviewer:t1",
            &json!({"approved":true,"findings":[],"report":"internal"}),
        );
        assert!(!normal.iter().any(|line| line.contains("internal")));
    }
}

use super::super::tui::view::clean;
use serde_json::Value;
use std::io::Write;

pub(super) fn latest_plan(events: &[Value]) -> Option<String> {
    events
        .iter()
        .rev()
        .find(|event| event["kind"] == "plan_proposed")
        .and_then(|event| event["payload"]["plan"].as_str().map(str::to_owned))
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
        "notice" => Some(format!("⚠ {}", value(payload, "text"))),
        "mode" => Some(describe_mode(payload)),
        "mail" if payload["kind"] != "copy" => Some(describe_mail(payload)),
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
    if payload["author"] == "runtime" {
        return Some("· task results handed to orchestrator".into());
    }
    if payload["author"] == "plan-verifier" {
        return Some(format!(
            "\nplan verifier sent the plan back:\n{}\n",
            value(payload, "text")
        ));
    }
    if payload["author"] == "user" {
        return None;
    }
    Some(format!(
        "\n{}:\n{}\n",
        value(payload, "author"),
        value(payload, "text")
    ))
}

fn describe_plan(payload: &Value) -> String {
    let mut lines = review_lines("plan verifier", &payload["review"]);
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
                if task["writes"] == true {
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
        return "Running (autonomous mode).".to_owned();
    }
    let waiting = payload["waiting"]
        .as_str()
        .map(|reason| format!("Waiting for you: {reason}. "))
        .unwrap_or_default();
    format!("{waiting}Type /accept to run it.")
}

fn describe_mode(payload: &Value) -> String {
    if payload["autonomous"] == true {
        "⚙ autonomous mode: next plans and rounds start automatically".into()
    } else {
        "⚙ supervised mode: next plans wait for /accept".into()
    }
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

/// What a reviewer (plan verifier, auditor) concluded, for the user.
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
        let plan = describe(&json!({"kind":"plan_proposed","payload":{"tasks":[{"id":"R1","title":"Split","kind":"implementation","writes":true}],"review":null,"automatic":false}})).unwrap();
        assert!(plan.contains("[R1] Split (implementation, modifies files)"));
        assert!(plan.ends_with("Type /accept to run it."));
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
}

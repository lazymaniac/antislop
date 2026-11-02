use super::*;
use serde_json::Value;

/// The submitted deliverable of any role, without exposing its storage protocol.
pub(in crate::lean::tui) fn agent_result(raw: &str) -> String {
    let Ok(result) = serde_json::from_str::<TurnResult>(raw) else {
        return raw.to_owned();
    };
    match result {
        TurnResult::Report { text, .. } => text,
        TurnResult::Requirements { markdown } => markdown,
        TurnResult::Question { question } => question.transcript(),
        TurnResult::Completed { .. } => worker_report(raw),
        TurnResult::Plan {
            document, tasks, ..
        } => {
            format!(
                "{document}\n\n{}",
                section("Tasks", &serde_json::json!(tasks))
            )
        }
        TurnResult::Review { report, verdict } => {
            format!(
                "{report}\n\n{}",
                section("Verdict", &serde_json::json!(verdict))
            )
        }
        TurnResult::Blocked {
            report,
            resume_condition,
        } => {
            format!("{report}\n\nResume condition\n{resume_condition}")
        }
        TurnResult::NeedsInput {
            request,
            checkpoint,
        } => {
            format!("Input needed\n{request}\n\nCheckpoint\n{checkpoint}")
        }
        TurnResult::Ledger { extraction } => section(
            "Extracted requirements",
            &serde_json::json!(extraction.entries),
        ),
        TurnResult::Audit { findings, evidence } => format!(
            "{}\n\n{}",
            section("Findings", &serde_json::json!(findings)),
            section("Evidence", &serde_json::json!(evidence))
        ),
        TurnResult::AuditReview { note, assessments } => {
            format!(
                "{note}\n\n{}",
                section("Evidence assessments", &serde_json::json!(assessments))
            )
        }
    }
}

fn section(title: &str, value: &Value) -> String {
    let mut lines = vec![title.to_owned()];
    append_value(value, 0, &mut lines);
    lines.join("\n")
}

fn append_value(value: &Value, depth: usize, lines: &mut Vec<String>) {
    let indent = "  ".repeat(depth);
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                lines.push(format!("{indent}{}:", key.replace('_', " ")));
                append_value(value, depth + 1, lines);
            }
        }
        Value::Array(items) if !items.is_empty() => {
            for (index, value) in items.iter().enumerate() {
                lines.push(format!("{indent}{}.", index + 1));
                append_value(value, depth + 1, lines);
            }
        }
        Value::Array(_) => lines.push(format!("{indent}None.")),
        Value::String(text) => lines.extend(text.lines().map(|line| format!("{indent}{line}"))),
        Value::Bool(value) => lines.push(format!("{indent}{}", if *value { "yes" } else { "no" })),
        Value::Null => lines.push(format!("{indent}—")),
        Value::Number(value) => lines.push(format!("{indent}{value}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn formats_all_session_deliverables_and_keeps_their_content() {
        let fixtures = [
            (
                json!({"type":"report","text":"Answer\n\nNext paragraph 👩‍💻"}),
                "Next paragraph 👩‍💻",
            ),
            (
                json!({"type":"plan","document":"Proposed plan","tasks":[]}),
                "Proposed plan",
            ),
            (
                json!({"type":"requirements","markdown":"Agreed scope"}),
                "Agreed scope",
            ),
            (
                json!({"type":"question","question":{"id":"q1","context":"Context","question":"Which approach?","options":["First","Second"]}}),
                "Which approach?",
            ),
            (
                json!({"type":"completed","summary":"Summary","report":"Implemented","selection":null}),
                "Implemented",
            ),
            (
                json!({"type":"review","report":"Reviewed every file","verdict":{"approved":false,"findings":[{"issue":"Missing check","severity":"high"}],"coverage":{"R1":{"status":"issue","note":"Not covered"}}}}),
                "Not covered",
            ),
            (
                json!({"type":"blocked","report":"Could not finish","resume_condition":"Restore access"}),
                "Restore access",
            ),
            (
                json!({"type":"needs_input","request":"Choose a format","checkpoint":"Files inspected"}),
                "Files inspected",
            ),
            (
                json!({"type":"ledger","extraction":{"run_id":"r1","expected_spec_revision":0,"entries":[{"id":"R1","source":{"source_id":"s1","start":0,"end":10},"interpretation":"Preserve café"}]}}),
                "Preserve café",
            ),
            (
                json!({"type":"audit","findings":[{"issue":"Missing check","where":"src/main.rs","severity":"high"}],"evidence":[{"criterion_id":"C1","task_id":"t1","result":"Reviewed café","artifact":"src/main.rs"}]}),
                "Reviewed café",
            ),
            (
                json!({"type":"audit_review","note":"Checked all evidence","assessments":[{"evidence":"E1","approved":false,"note":"Check failed"}]}),
                "Check failed",
            ),
        ];
        for (value, expected) in fixtures {
            let raw = value.to_string();
            // Ensure fixtures exercise typed formatting rather than the legacy fallback.
            serde_json::from_str::<TurnResult>(&raw).unwrap();
            let rendered = agent_result(&raw);
            assert!(rendered.contains(expected), "{rendered}");
            assert!(!rendered.contains("\"type\":"), "{rendered}");
            let mut stream = ReportStream::default();
            let mut streamed = String::new();
            let mut sink = |chunk: &str| {
                streamed.push_str(chunk);
                Ok(())
            };
            for chunk in raw.split_inclusive(|_: char| true) {
                stream.push(chunk, &mut sink).unwrap();
            }
            stream.finish_with(agent_result, &mut sink).unwrap();
            assert_eq!(streamed, rendered);
        }
    }
}

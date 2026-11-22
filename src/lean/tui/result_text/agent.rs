use super::*;
use serde_json::Value;

/// The submitted deliverable of any role, without exposing its storage protocol.
pub(in crate::lean::tui) fn agent_result(raw: &str) -> String {
    let Ok(result) = serde_json::from_str::<TurnResult>(raw) else {
        return raw.to_owned();
    };
    match result {
        TurnResult::Report { text, .. } => text,
        TurnResult::Requirements { markdown, .. } => markdown,
        TurnResult::Question { question } => question.transcript(),
        TurnResult::Completed { .. } => worker_report(raw),
        TurnResult::Plan { document, .. } => document,
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
    fn planner_results_display_only_the_authored_document() {
        let raw = json!({"type":"plan","document":"# Plan\nFull authored explanation", "tasks":[{"id":"hidden-task","title":"Raw task title","goal":"Raw task goal","kind":"analysis","change_kind":"none"}]}).to_string();
        serde_json::from_str::<TurnResult>(&raw).unwrap();
        assert_eq!(agent_result(&raw), "# Plan\nFull authored explanation");
    }

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

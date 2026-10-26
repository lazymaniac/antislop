//! Stable recovery proposals: bookkeeping and renamed tasks are not progress.
use serde_json::{Value, json};

pub(crate) fn text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn family(action: &str) -> &str {
    match action {
        "extend_rounds" | "retry_with_guidance" => "retry_task",
        other => other,
    }
}

pub(crate) fn proposal(
    action: &str,
    spec: Option<&Value>,
    message: Option<&str>,
    parts: &[Value],
) -> Value {
    match action {
        "clarify_task" => {
            json!({"spec": normalized_spec(spec.cloned().unwrap_or_default()), "reason":text(message.unwrap_or_default())})
        }
        "add_task" => normalized_spec(spec.cloned().unwrap_or_default()),
        "split_task" => {
            let ids: Vec<_> = parts
                .iter()
                .filter_map(|part| part["id"].as_str())
                .collect();
            Value::Array(
                parts
                    .iter()
                    .map(|part| {
                        let mut part = normalized_spec(part.clone());
                        if let Some(deps) = part["depends_on"].as_array_mut() {
                            for dep in deps {
                                if let Some(index) =
                                    ids.iter().position(|id| Some(*id) == dep.as_str())
                                {
                                    *dep = json!({"part":index});
                                }
                            }
                        }
                        part
                    })
                    .collect(),
            )
        }
        _ => json!({"guidance":text(message.unwrap_or_default())}),
    }
}

pub(crate) fn normalized_spec(mut spec: Value) -> Value {
    if let Some(object) = spec.as_object_mut() {
        for field in ["id", "title", "summary", "section", "resolves", "part_of"] {
            object.remove(field);
        }
        for field in ["goal"] {
            if let Some(value) = object
                .get_mut(field)
                .and_then(|value| value.as_str())
                .map(text)
            {
                object.insert(field.into(), json!(value));
            }
        }
        if let Some(verifier) = object.get_mut("verifier").and_then(Value::as_object_mut) {
            verifier.remove("summary");
            if let Some(focus) = verifier.get("focus").and_then(Value::as_str).map(text) {
                verifier.insert("focus".into(), json!(focus));
            }
        }
    }
    spec
}

/// Session UUIDs in raw failures change on every retry, unlike the failure.
pub(crate) fn outcome(report: &str) -> String {
    let mut session_identity = false;
    report
        .split_whitespace()
        .map(|word| {
            let candidate = word.trim_matches(|c: char| !c.is_ascii_hexdigit() && c != '-');
            let replace = session_identity && uuid::Uuid::parse_str(candidate).is_ok();
            session_identity = matches!(
                word.trim_matches(|c: char| !c.is_alphabetic())
                    .to_ascii_lowercase()
                    .as_str(),
                "turn" | "session" | "thread"
            );
            if replace { "<session>" } else { word }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn needs_user_or_stalled(review: &crate::lean::tasks::Verdict) -> bool {
    review.findings.iter().any(|finding| {
        matches!(
            finding["code"].as_str(),
            Some("recovery_no_progress" | "recovery_user_decision")
        )
    })
}

pub(crate) fn actionable(review: &crate::lean::tasks::Verdict) -> bool {
    !needs_user_or_stalled(review)
        && review.findings.iter().any(|finding| {
            finding["issue"]
                .as_str()
                .is_some_and(|issue| !issue.trim().is_empty())
        })
}

pub(crate) fn review_evidence(value: Option<&Value>) -> Value {
    let review = value
        .cloned()
        .and_then(|value| serde_json::from_value::<crate::lean::tasks::Verdict>(value).ok());
    review.map_or(Value::Null, |review| {
        json!({
            "approved":review.approved,"findings":review.findings,
            "open_obligations":crate::lean::progress::open_obligations(&review)
        })
    })
}

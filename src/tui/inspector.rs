//! Readable agent records, independent of terminal width and runtime mutation.
use super::{app::App, model::AGENT_TABS};
use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct State {
    pub offset: usize,
    pub query: String,
    pub raw: bool,
    pub total: usize,
    pub page: usize,
}
impl State {
    pub fn max_offset(&self) -> usize {
        self.total.saturating_sub(self.page)
    }
}
#[derive(Debug, Clone, Copy)]
pub enum Kind {
    Heading,
    Field,
    Status,
    Body,
    Record,
}
#[derive(Debug)]
pub struct Line {
    pub text: String,
    pub kind: Kind,
}
fn push(out: &mut Vec<Line>, text: impl Into<String>, kind: Kind) {
    out.push(Line {
        text: text.into(),
        kind,
    });
}
fn label(key: &str) -> String {
    let words = key.replace('_', " ");
    let mut chars = words.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default()
}
fn field_order(key: &str) -> usize {
    // Put identity, observed state and actual content before audit/context data.
    const ORDER: &[&str] = &[
        "current_session",
        "id",
        "check_id",
        "artifact_hash",
        "hash",
        "role",
        "status",
        "state",
        "verdict",
        "severity",
        "from",
        "to",
        "session",
        "session_id",
        "provider",
        "model",
        "description",
        "task",
        "title",
        "summary",
        "text",
        "rationale",
        "artifact",
        "output",
        "content",
        "findings",
        "checks",
        "evidence_ids",
        "reactions",
        "version",
        "target",
        "sessions",
        "turns",
    ];
    ORDER
        .iter()
        .position(|name| *name == key)
        .unwrap_or(ORDER.len())
}
/// Decode strings into real paragraphs; keep every field, identity and hash.
fn fields(out: &mut Vec<Line>, value: &Value, depth: usize, record_name: &str) {
    let indent = "  ".repeat(depth.min(8));
    if depth > 24 {
        push(out, format!("{indent}{value}"), Kind::Body);
        return;
    }
    match value {
        Value::Object(map) if !map.is_empty() => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by_key(|(key, _)| field_order(key));
            for (key, value) in entries {
                let name = label(key);
                match value {
                    Value::Object(v) if !v.is_empty() => {
                        push(out, format!("{indent}{name}"), Kind::Heading);
                        fields(out, value, depth + 1, &label(key));
                    }
                    Value::Array(v) if !v.is_empty() => {
                        push(out, format!("{indent}{name} ({})", v.len()), Kind::Heading);
                        fields(out, value, depth + 1, &label(key));
                    }
                    Value::String(text)
                        if matches!(key.as_str(), "output" | "artifact" | "content" | "result")
                            && serde_json::from_str::<Value>(text)
                                .is_ok_and(|v| v.is_object() || v.is_array()) =>
                    {
                        push(out, format!("{indent}{name}"), Kind::Heading);
                        if let Ok(decoded) = serde_json::from_str::<Value>(text) {
                            fields(out, &decoded, depth + 1, &name);
                        }
                    }
                    Value::String(text) if text.contains('\n') => {
                        push(out, format!("{indent}{name}"), Kind::Heading);
                        for line in text.split('\n') {
                            push(out, format!("{indent}  {line}"), Kind::Body);
                        }
                    }
                    _ => push(
                        out,
                        format!("{indent}{name}: {}", scalar(value)),
                        if matches!(key.as_str(), "state" | "status") {
                            Kind::Status
                        } else {
                            Kind::Field
                        },
                    ),
                }
            }
        }
        Value::Array(items) if !items.is_empty() => {
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    push(out, "", Kind::Body);
                }
                push(
                    out,
                    format!("{indent}Record {} / {}", i + 1, items.len()),
                    if depth <= 1 {
                        Kind::Record
                    } else {
                        Kind::Heading
                    },
                );
                if let Some(record) = out.last_mut() {
                    let identity = ["id", "check_id", "hash", "artifact_hash"]
                        .iter()
                        .find_map(|key| item[*key].as_str())
                        .unwrap_or("");
                    record.text = format!(
                        "{indent}{record_name} {} / {}{}",
                        i + 1,
                        items.len(),
                        if identity.is_empty() {
                            String::new()
                        } else {
                            format!(" | {identity}")
                        }
                    );
                }
                fields(out, item, depth + 1, record_name);
            }
        }
        _ => push(out, format!("{indent}{}", scalar(value)), Kind::Body),
    }
}
fn scalar(value: &Value) -> String {
    match value {
        Value::Null => "Not recorded".into(),
        Value::String(v) => {
            if v.is_empty() {
                "(empty)".into()
            } else {
                v.clone()
            }
        }
        Value::Array(v) if v.is_empty() => "No records".into(),
        Value::Object(v) if v.is_empty() => "No fields".into(),
        _ => value.to_string(),
    }
}
fn names_agent(value: &Value, agent: &str) -> bool {
    match value {
        Value::Object(map) => map.iter().any(|(key, value)| {
            (matches!(
                key.as_str(),
                "agent_id" | "owner_id" | "verifier_id" | "supervisor_id"
            ) && value.as_str() == Some(agent))
                || (key == "agent" && value["id"].as_str() == Some(agent))
                || names_agent(value, agent)
        }),
        Value::Array(items) => items.iter().any(|item| names_agent(item, agent)),
        _ => false,
    }
}
fn comments(app: &App) -> Value {
    let targets = app.comment_targets();
    Value::Array(
        app.snapshot
            .element_comments
            .iter()
            .filter(|c| {
                !c.deleted
                    && (targets.iter().any(|e| {
                        e.reference.kind == c.target.kind && e.reference.id == c.target.id
                    }) || (c.target.kind == crate::comments::ElementKind::Agent
                        && Some(c.target.id.as_str()) == app.selected_agent_id.as_deref())
                        || app
                            .selected_agent_id
                            .as_deref()
                            .is_some_and(|id| names_agent(&c.context, id)))
            })
            .map(|c| {
                let current = targets
                    .iter()
                    .find(|e| e.reference.kind == c.target.kind && e.reference.id == c.target.id);
                let mut record = serde_json::to_value(c).unwrap_or_default();
                record["state"] =
                    Value::String(if c.resolved { "resolved" } else { "open" }.into());
                record["version"] = Value::String(
                    if current.is_some_and(|e| e.reference.version_hash == c.target.version_hash) {
                        "current"
                    } else {
                        "stale or historical"
                    }
                    .into(),
                );
                record
            })
            .collect(),
    )
}
fn messages(app: &App) -> Value {
    let Some(agent) = app.selected() else {
        return Value::Array(vec![]);
    };
    let ledger = agent
        .details
        .get("Messages")
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .unwrap_or_default();
    Value::Array(
        app.snapshot
            .messages
            .iter()
            .filter(|m| m.target == agent.id || m.sender == agent.id)
            .map(|m| {
                let mut record = ledger
                    .as_array()
                    .and_then(|records| records.iter().find(|r| r["id"] == m.id))
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));
                for (key, value) in [
                    ("id", &m.id),
                    ("from", &m.sender),
                    ("to", &m.target),
                    ("state", &m.state),
                    ("session", &m.session),
                    ("text", &m.text),
                ] {
                    record[key] = Value::String(value.clone());
                }
                record
            })
            .collect(),
    )
}
pub fn document(app: &App, tab: usize, raw: bool) -> Vec<Line> {
    let mut out = vec![];
    let name = AGENT_TABS[tab];
    let Some(agent) = app.selected() else {
        push(&mut out, "Agent no longer available.", Kind::Body);
        return out;
    };
    let value = match name {
        "Messages" => Some(messages(app)),
        "Comments" => Some(comments(app)),
        _ => agent
            .details
            .get(name)
            .and_then(|text| serde_json::from_str::<Value>(text).ok()),
    };
    if raw {
        let text = value
            .as_ref()
            .map(|v| serde_json::to_string_pretty(v).unwrap_or_default())
            .or_else(|| agent.details.get(name).cloned())
            .unwrap_or_else(|| "No recorded data available.".into());
        for line in text.split('\n') {
            push(&mut out, line, Kind::Body);
        }
        return out;
    }
    if tab == 0 {
        push(&mut out, "Agent summary", Kind::Heading);
        for (key, value) in [
            ("Id", &agent.id),
            ("Role", &agent.role),
            ("Status", &agent.status),
            ("Provider", &agent.provider),
            ("Model", &agent.model),
            ("Stage", &agent.group),
            ("Task", &agent.task),
        ] {
            push(
                &mut out,
                format!(
                    "{key}: {}",
                    if value.is_empty() {
                        "Not recorded"
                    } else {
                        value
                    }
                ),
                Kind::Field,
            );
        }
        push(&mut out, "", Kind::Body);
    }
    push(&mut out, name, Kind::Heading);
    match value {
        Some(Value::Array(ref items)) if items.is_empty() => push(
            &mut out,
            format!("No recorded {}.", name.to_lowercase()),
            Kind::Body,
        ),
        Some(ref value) => fields(&mut out, value, 0, name),
        None => {
            for line in agent
                .details
                .get(name)
                .map(String::as_str)
                .unwrap_or("No recorded data available.")
                .split('\n')
            {
                push(&mut out, line, Kind::Body);
            }
        }
    }
    if tab == 4 {
        push(
            &mut out,
            "m: write to agent. Queued messages require a session receipt for delivery.",
            Kind::Body,
        );
    }
    if tab == 7 {
        push(
            &mut out,
            "c: add comment. u: comment history, resolve/reopen or delete.",
            Kind::Body,
        );
    }
    out
}

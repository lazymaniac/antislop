use super::*;

/// Adds the final answers of `turn` to `builder`; the agent's report is the
/// only session text retained for other agents to build on.
pub(super) fn append_answers(
    client: &Client,
    conversation: &str,
    turn: &str,
    builder: &mut document::Builder,
) -> Result<bool> {
    let mut found = false;
    let mut after = 0;
    loop {
        let page = session_page(client, conversation, turn, after)?;
        found |= page["events"]
            .as_array()
            .is_some_and(|events| events.iter().any(|event| event["kind"] == "agent_final"));
        append_final_events(builder, client, conversation, &page)?;
        if page["has_more"] != true {
            break;
        }
        after = next_event_cursor(&page, after)?;
    }
    Ok(found)
}

fn session_page(client: &Client, conversation: &str, turn: &str, after: u64) -> Result<Value> {
    client.call(&Request::Session(
        crate::lean::daemon::SessionRequest::SessionEvents {
            conversation: conversation.into(),
            turn: turn.into(),
            after,
            limit: Some(100),
            include_messages: Some(false),
        },
    ))
}

fn append_final_events(
    builder: &mut document::Builder,
    client: &Client,
    conversation: &str,
    page: &Value,
) -> Result<()> {
    for event in page["events"].as_array().cloned().unwrap_or_default() {
        if event["kind"] == "agent_final" {
            append_final_event(builder, client, conversation, &event)?;
        }
    }
    Ok(())
}

fn append_final_event(
    builder: &mut document::Builder,
    client: &Client,
    conversation: &str,
    event: &Value,
) -> Result<()> {
    let payload = &event["payload"];
    builder.append("Submitted result\n\n", |sink| {
        if payload["text"].get("session_content").is_some() {
            append_session_content(
                client,
                conversation,
                event["seq"].as_u64().unwrap_or(0),
                sink,
            )
        } else {
            sink(&super::super::result_text::agent_result(
                payload["text"].as_str().unwrap_or_default(),
            ))
        }
    })
}

fn append_session_content(
    client: &Client,
    conversation: &str,
    event: u64,
    sink: &mut dyn FnMut(&str) -> Result<()>,
) -> Result<()> {
    let mut formatter = super::super::result_text::ReportStream::default();
    let mut offset = 0;
    loop {
        let chunk = client.call(&Request::Session(
            crate::lean::daemon::SessionRequest::SessionContent {
                conversation: conversation.into(),
                event,
                field: "text".into(),
                offset,
            },
        ))?;
        formatter.push(chunk["text"].as_str().unwrap_or_default(), sink)?;
        if chunk["done"] == true {
            return formatter.finish_with(super::super::result_text::agent_result, sink);
        }
        let next = chunk["next_offset"].as_u64().unwrap_or(0);
        crate::require(next > offset, "session content cursor did not advance")?;
        offset = next;
    }
}

fn next_event_cursor(page: &Value, after: u64) -> Result<u64> {
    let next = page["next_after"].as_u64().unwrap_or(0);
    crate::require(next > after, "session event cursor did not advance")?;
    Ok(next)
}

pub(in crate::lean::tui) fn run_snapshot(
    client: &Client,
    conversation: &str,
    role: &str,
    selected: Option<u64>,
) -> Result<Snapshot> {
    let runs = session_runs(client, conversation, role)?;
    let index = selected_run_index(&runs, selected);
    let task = session_metadata(&runs, index);
    match runs.get(index) {
        Some(run) => selected_run_snapshot(client, conversation, run, task),
        None => Ok(Snapshot {
            task,
            documents: Default::default(),
        }),
    }
}

fn session_runs(client: &Client, conversation: &str, role: &str) -> Result<Vec<Value>> {
    let sessions = client.call(&Request::Session(
        crate::lean::daemon::SessionRequest::Sessions {
            conversation: conversation.into(),
        },
    ))?;
    Ok(sessions
        .as_array()
        .into_iter()
        .flatten()
        .filter(|run| run["role"] == role)
        .cloned()
        .collect())
}

fn selected_run_index(runs: &[Value], selected: Option<u64>) -> usize {
    selected.unwrap_or(runs.len().saturating_sub(1) as u64) as usize
}

fn session_metadata(runs: &[Value], index: usize) -> Value {
    let mut task = serde_json::json!({
        "round": index,
        "_rounds": runs.iter().enumerate().map(|(round, run)| {
            serde_json::json!({"round": round, "turn_id": run["turn_id"]})
        }).collect::<Vec<_>>()
    });
    task["_latest_run"] = runs
        .last()
        .map(|run| run["turn_id"].clone())
        .unwrap_or_default();
    task
}

fn selected_run_snapshot(
    client: &Client,
    conversation: &str,
    run: &Value,
    mut task: Value,
) -> Result<Snapshot> {
    task["turns"] = serde_json::json!([run]);
    task["_run"] = run["turn_id"].clone();
    let turn = run["turn_id"].as_str().unwrap_or_default();
    let mut builder = document::Builder::new(client.state_dir())?;
    append_run_status(&mut builder, run)?;
    if !append_answers(client, conversation, turn, &mut builder)? {
        builder.append("", |sink| sink("No result submitted for this turn yet."))?;
    }
    let document = builder.finish()?;
    let documents = std::collections::HashMap::from([
        ("result".into(), document.clone()),
        ("review".into(), document),
    ]);
    Ok(Snapshot { task, documents })
}

fn append_run_status(builder: &mut document::Builder, run: &Value) -> Result<()> {
    builder.append("", |sink| {
        sink(&format!(
            "Status: {}\n",
            run["status"].as_str().unwrap_or("unknown")
        ))?;
        if let Some(error) = run["error"].as_str() {
            sink(&format!("Error: {error}\n"))?;
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn session_results_are_formatted_and_selected_turns_keep_their_identity() {
        let state = tempfile::tempdir().unwrap();
        let client = Client::at(state.path());
        let runs = vec![json!({"turn_id":"earlier"}), json!({"turn_id":"latest"})];
        assert_eq!(
            session_metadata(&runs, selected_run_index(&runs, Some(0)))["_rounds"][0]["turn_id"],
            "earlier"
        );
        assert_eq!(selected_run_index(&runs, None), 1);
        let mut builder = document::Builder::new(client.state_dir()).unwrap();
        append_run_status(
            &mut builder,
            &json!({"status":"failed","error":"login required"}),
        )
        .unwrap();
        append_final_events(&mut builder, &client, "conversation", &json!({"events":[
            {"kind":"agent","payload":{"text":"Internal activity"}},
            {"kind":"agent_final","payload":{"text":json!({"type":"audit_review","note":"Independent check","assessments":[{"evidence":"E1","approved":false,"note":"Missing café check"}]}).to_string()}}
        ]})).unwrap();
        let document = builder.finish().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let page = loop {
            if let Some(page) = document.viewport(100, 100, 0, false).unwrap() {
                break page;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        let text = page
            .lines
            .into_iter()
            .map(|(_, line)| line)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            text.contains("login required") && text.contains("Missing café check"),
            "{text}"
        );
        assert!(
            !text.contains("Internal activity") && !text.contains("\"type\":"),
            "{text}"
        );
    }
}

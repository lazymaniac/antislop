use super::*;

/// Adds the final answers of `turn` to `builder`; the agent's report is the
/// only session text retained for other agents to build on.
fn append_answers(
    client: &Client,
    conversation: &str,
    turn: &str,
    builder: &mut document::Builder,
) -> Result<()> {
    let mut after = 0;
    loop {
        let page = session_page(client, conversation, turn, after)?;
        append_final_events(builder, client, conversation, &page)?;
        if page["has_more"] != true {
            break;
        }
        after = next_event_cursor(&page, after)?;
    }
    Ok(())
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
    builder.append("Final answer\n", |sink| {
        if payload["text"].get("session_content").is_some() {
            append_session_content(
                client,
                conversation,
                event["seq"].as_u64().unwrap_or(0),
                sink,
            )
        } else {
            sink(payload["text"].as_str().unwrap_or_default())
        }
    })
}

fn append_session_content(
    client: &Client,
    conversation: &str,
    event: u64,
    sink: &mut dyn FnMut(&str) -> Result<()>,
) -> Result<()> {
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
        sink(chunk["text"].as_str().unwrap_or_default())?;
        if chunk["done"] == true {
            return Ok(());
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
    append_answers(client, conversation, turn, &mut builder)?;
    let documents = run_documents(client, conversation, turn, builder.finish()?)?;
    Ok(Snapshot { task, documents })
}

fn run_documents(
    client: &Client,
    conversation: &str,
    turn: &str,
    review: document::Document,
) -> Result<std::collections::HashMap<String, document::Document>> {
    let mut documents = std::collections::HashMap::new();
    documents.insert("review".into(), review);
    documents.insert(
        "messages".into(),
        super::message_document(
            client,
            conversation,
            &super::super::super::store::HistoryScope::Session { turn: turn.into() },
        )?,
    );
    Ok(documents)
}

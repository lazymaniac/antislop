use super::*;

pub(in crate::lean::tui) fn task_snapshot(
    client: &Client,
    conversation: &str,
    plan: &str,
    id: &str,
    round: Option<u64>,
    checks_only: bool,
) -> Result<Snapshot> {
    if uuid::Uuid::parse_str(id).is_ok() {
        return round_snapshot(client, conversation, plan, id, round);
    }
    task_id_snapshot(client, conversation, plan, id, round, checks_only)
}

fn task_id_snapshot(
    client: &Client,
    conversation: &str,
    plan: &str,
    id: &str,
    round: Option<u64>,
    checks_only: bool,
) -> Result<Snapshot> {
    let request = task_request(conversation, plan, id, checks_only);
    let task = client.call(&request)?;
    let Some(uuid) = task["id"]
        .as_str()
        .filter(|id| uuid::Uuid::parse_str(id).is_ok())
    else {
        return Ok(Snapshot {
            task,
            documents: Default::default(),
        });
    };
    task_snapshot(client, conversation, plan, uuid, round, checks_only)
}

fn task_request(conversation: &str, plan: &str, id: &str, checks_only: bool) -> Request {
    let conversation = Some(conversation.into());
    if checks_only {
        Request::Conversation(crate::lean::daemon::ConversationRequest::TaskChecks {
            conversation,
            plan: plan.into(),
            id: id.into(),
        })
    } else {
        Request::Conversation(crate::lean::daemon::ConversationRequest::TaskScoped {
            conversation,
            plan: plan.into(),
            id: id.into(),
        })
    }
}

fn round_snapshot(
    client: &Client,
    conversation: &str,
    plan: &str,
    uuid: &str,
    round: Option<u64>,
) -> Result<Snapshot> {
    let rounds = client.call(&Request::History(
        crate::lean::daemon::HistoryRequest::TaskRounds {
            conversation: conversation.into(),
            plan: plan.into(),
            id: uuid.into(),
        },
    ))?;
    let selected = selected_round(round, &rounds);
    let mut task = client.call(&Request::History(
        crate::lean::daemon::HistoryRequest::TaskRound {
            conversation: conversation.into(),
            plan: plan.into(),
            id: uuid.into(),
            round: selected as u32,
            references: Some(true),
        },
    ))?;
    task["_rounds"] = rounds;
    let documents = round_documents(client, conversation, uuid, selected, &task)?;
    Ok(Snapshot { task, documents })
}

fn selected_round(requested: Option<u64>, rounds: &Value) -> u64 {
    requested
        .or_else(|| rounds.as_array()?.last()?["round"].as_u64())
        .unwrap_or(0)
}

fn round_documents(
    client: &Client,
    conversation: &str,
    uuid: &str,
    round: u64,
    task: &Value,
) -> Result<std::collections::HashMap<String, document::Document>> {
    let mut documents = std::collections::HashMap::new();
    documents.insert(
        "result".into(),
        result_document(client, conversation, uuid, round, task)?,
    );
    Ok(documents)
}

fn result_document(
    client: &Client,
    conversation: &str,
    uuid: &str,
    round: u64,
    task: &Value,
) -> Result<document::Document> {
    let mut result = document::Builder::new(client.state_dir())?;
    append_history_notice(&mut result, task, round)?;
    let summary = format!(
        "Status: {}\nBase: {}\nResult: {}\n",
        task["status"].as_str().unwrap_or_default(),
        task["base"].as_str().unwrap_or("—"),
        task["result"].as_str().unwrap_or("—")
    );
    result.append("", |sink| sink(&summary))?;
    append_round_fields(&mut result, client, conversation, uuid, round, task)?;
    result.finish()
}

fn append_history_notice(result: &mut document::Builder, task: &Value, round: u64) -> Result<()> {
    if task["history_unavailable"] == true && round > 0 {
        result.append("History incomplete\n", |sink| sink("Historical results may be unavailable and earlier text may have been truncated. Previously clipped content cannot be recovered."))?;
    }
    Ok(())
}

fn append_round_fields(
    result: &mut document::Builder,
    client: &Client,
    conversation: &str,
    uuid: &str,
    round: u64,
    task: &Value,
) -> Result<()> {
    if task["report"].is_null() {
        return Ok(());
    }
    let owner = super::super::super::store::ContentOwner::Round {
        task: uuid.into(),
        round: round as u32,
        field: "report".into(),
    };
    result.append("\nWorker report\n\n", |sink| {
        append_field_content(client, conversation, &owner, &task["report"], sink)
    })
}

fn append_field_content(
    client: &Client,
    conversation: &str,
    owner: &super::super::super::store::ContentOwner,
    value: &Value,
    sink: &mut dyn FnMut(&str) -> Result<()>,
) -> Result<()> {
    use super::super::result_text::{ReportStream, worker_report};
    if value.get("session_content").is_some() {
        let mut formatter = ReportStream::default();
        read_content(client, conversation, owner, &mut |chunk| {
            formatter.push(chunk, sink)
        })?;
        formatter.finish(sink)
    } else if let Some(report) = value.as_str() {
        sink(&worker_report(report))
    } else {
        sink(&worker_report(&value.to_string()))
    }
}

fn read_content(
    client: &Client,
    conversation: &str,
    owner: &super::super::super::store::ContentOwner,
    sink: &mut dyn FnMut(&str) -> Result<()>,
) -> Result<()> {
    let mut offset = 0;
    loop {
        let chunk = client.call(&Request::History(
            crate::lean::daemon::HistoryRequest::HistoryContent {
                conversation: conversation.into(),
                owner: owner.clone(),
                offset,
            },
        ))?;
        sink(chunk["text"].as_str().unwrap_or_default())?;
        if chunk["done"] == true {
            return Ok(());
        }
        let next = chunk["next_offset"].as_u64().unwrap_or(0);
        crate::require(next > offset, "history content cursor did not advance")?;
        offset = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn historical_result_formats_options_and_omits_the_verdict() {
        let state = tempfile::tempdir().unwrap();
        let client = Client::at(state.path());
        let selection = serde_json::from_value(json!({
            "selected":"A", "rationale":"Smallest change.",
            "alternatives":[
                {"id":"A","approach":"Reuse the store","tradeoffs":"Existing constraints"},
                {"id":"B","approach":"Replace the store","tradeoffs":"Migration"}
            ]
        }))
        .unwrap();
        let report =
            crate::lean::turn_result::report_with_deliverables("Worker text", Some(&selection));
        let task = json!({"status":"approved", "report":report,
            "verdict":{"findings":[{"issue":"Verifier-only finding"}]}});
        let document = result_document(&client, "conversation", "task", 1, &task).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let page = loop {
            if let Some(page) = document.viewport(80, 100, 0, false).unwrap() {
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
            text.contains("Selected option: A") && text.contains("Option B"),
            "{text}"
        );
        assert!(
            !text.contains("Verifier")
                && !text.contains("```json")
                && !text.contains("\"alternatives\""),
            "{text}"
        );
    }
}

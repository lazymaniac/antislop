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
    documents.insert(
        "messages".into(),
        message_document(
            client,
            conversation,
            &super::super::super::store::HistoryScope::Round {
                task: uuid.into(),
                round: round as u32,
            },
        )?,
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
    for field in ["report", "verdict"] {
        if !task[field].is_null() {
            append_round_field(result, client, conversation, uuid, round, task, field)?;
        }
    }
    Ok(())
}

fn append_round_field(
    result: &mut document::Builder,
    client: &Client,
    conversation: &str,
    uuid: &str,
    round: u64,
    task: &Value,
    field: &str,
) -> Result<()> {
    let owner = super::super::super::store::ContentOwner::Round {
        task: uuid.into(),
        round: round as u32,
        field: field.into(),
    };
    let title = if field == "report" {
        "Worker report\n"
    } else {
        "Verifier verdict\n"
    };
    result.append(title, |sink| {
        append_field_content(client, conversation, &owner, &task[field], sink)
    })
}

fn append_field_content(
    client: &Client,
    conversation: &str,
    owner: &super::super::super::store::ContentOwner,
    value: &Value,
    sink: &mut dyn FnMut(&str) -> Result<()>,
) -> Result<()> {
    if value.get("session_content").is_some() {
        read_content(client, conversation, owner, sink)
    } else if let Some(text) = value.as_str() {
        sink(text)
    } else {
        sink(&value.to_string())
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

pub(in crate::lean::tui) fn message_document(
    client: &Client,
    conversation: &str,
    scope: &super::super::super::store::HistoryScope,
) -> Result<document::Document> {
    let mut builder = document::Builder::new(client.state_dir())?;
    let mut after = 0;
    loop {
        let page = message_page(client, conversation, scope, after)?;
        append_message_page(&mut builder, client, conversation, &page)?;
        if page["has_more"] != true {
            break;
        }
        after = next_message_cursor(&page, after)?;
    }
    builder.finish()
}

fn message_page(
    client: &Client,
    conversation: &str,
    scope: &super::super::super::store::HistoryScope,
    after: u64,
) -> Result<Value> {
    client.call(&Request::History(
        crate::lean::daemon::HistoryRequest::HistoryMessages {
            conversation: conversation.into(),
            scope: scope.clone(),
            after,
            limit: Some(100),
        },
    ))
}

fn append_message_page(
    builder: &mut document::Builder,
    client: &Client,
    conversation: &str,
    page: &Value,
) -> Result<()> {
    for message in page["messages"].as_array().into_iter().flatten() {
        append_message(builder, client, conversation, message)?;
    }
    Ok(())
}

fn append_message(
    builder: &mut document::Builder,
    client: &Client,
    conversation: &str,
    message: &Value,
) -> Result<()> {
    let header = format!(
        "{} → {} · {}\n",
        message["sender"].as_str().unwrap_or_default(),
        message["recipient"].as_str().unwrap_or_default(),
        message["state"].as_str().unwrap_or_default()
    );
    let owner = super::super::super::store::ContentOwner::Mail {
        id: message["id"].as_u64().unwrap_or(0),
    };
    builder.append(&header, |sink| {
        read_content(client, conversation, &owner, sink)
    })
}

fn next_message_cursor(page: &Value, after: u64) -> Result<u64> {
    let next = page["next_after"].as_u64().unwrap_or(0);
    crate::require(next > after, "message cursor did not advance")?;
    Ok(next)
}

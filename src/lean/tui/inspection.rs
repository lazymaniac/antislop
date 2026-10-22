//! History retrieval runs on the existing task-fetch thread, never while drawing.
use super::*;

pub(super) struct Snapshot {
    pub task: Value,
    pub documents: std::collections::HashMap<String, document::Document>,
}

pub(super) fn task_snapshot(
    client: &Client,
    conversation: &str,
    plan: &str,
    id: &str,
    round: Option<u64>,
    checks_only: bool,
) -> Result<Snapshot> {
    if uuid::Uuid::parse_str(id).is_err() {
        let request = if checks_only {
            Request::TaskChecks {
                conversation: Some(conversation.into()),
                plan: plan.into(),
                id: id.into(),
            }
        } else {
            Request::TaskScoped {
                conversation: Some(conversation.into()),
                plan: plan.into(),
                id: id.into(),
            }
        };
        let task = client.call(&request)?;
        if let Some(uuid) = task["id"]
            .as_str()
            .filter(|id| uuid::Uuid::parse_str(id).is_ok())
        {
            return task_snapshot(client, conversation, plan, uuid, round, checks_only);
        }
        return Ok(Snapshot {
            task,
            documents: Default::default(),
        });
    }
    let uuid = id;
    let rounds = client.call(&Request::TaskRounds {
        conversation: conversation.into(),
        plan: plan.into(),
        id: uuid.into(),
    })?;
    let selected = round
        .or_else(|| {
            rounds
                .as_array()
                .and_then(|rounds| rounds.last())
                .and_then(|round| round["round"].as_u64())
        })
        .unwrap_or(0);
    let mut snapshot = client.call(&Request::TaskRound {
        conversation: conversation.into(),
        plan: plan.into(),
        id: uuid.into(),
        round: selected as u32,
        references: Some(true),
    })?;
    snapshot["_rounds"] = rounds;
    let turns = snapshot["turns"].as_array().cloned().unwrap_or_default();
    let mut transcripts = serde_json::Map::new();
    let mut builders = std::collections::HashMap::new();
    for turn in turns {
        let Some(turn_id) = turn["turn_id"].as_str() else {
            continue;
        };
        let Some(role) = turn["role"].as_str() else {
            continue;
        };
        let local_role = role.split_once(':').map_or_else(
            || role.to_owned(),
            |(role, _)| format!("{role}:{}", snapshot["spec"]["id"].as_str().unwrap_or(id)),
        );
        if !builders.contains_key(&local_role) {
            builders.insert(
                local_role.clone(),
                document::Builder::new(client.state_dir())?,
            );
        }
        let events = append_session(
            client,
            conversation,
            turn_id,
            builders.get_mut(&local_role).expect("builder"),
        )?;
        transcripts
            .entry(local_role)
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .expect("array")
            .extend(events);
    }
    snapshot["_transcripts"] = transcripts.into();
    let mut documents = builders
        .into_iter()
        .map(|(role, builder)| Ok((role, builder.finish()?)))
        .collect::<Result<std::collections::HashMap<_, _>>>()?;
    let mut result = document::Builder::new(client.state_dir())?;
    if snapshot["history_unavailable"] == true && selected > 0 {
        result.append("plain","History incomplete\n",|sink|sink("Historical results may be unavailable and earlier text may have been truncated. Previously clipped content cannot be recovered."))?;
    }
    let summary = format!(
        "Status: {}\nBase: {}\nResult: {}\n",
        snapshot["status"].as_str().unwrap_or_default(),
        snapshot["base"].as_str().unwrap_or("—"),
        snapshot["result"].as_str().unwrap_or("—")
    );
    result.append("plain", "", |sink| sink(&summary))?;
    for field in ["report", "verdict"] {
        if snapshot[field].is_null() {
            continue;
        }
        let owner = super::super::store::ContentOwner::Round {
            task: uuid.into(),
            round: selected as u32,
            field: field.into(),
        };
        result.append(
            "plain",
            if field == "report" {
                "Worker report\n"
            } else {
                "Verifier verdict\n"
            },
            |sink| {
                if snapshot[field].get("session_content").is_some() {
                    read_content(client, conversation, &owner, sink)
                } else if let Some(text) = snapshot[field].as_str() {
                    sink(text)
                } else {
                    sink(&snapshot[field].to_string())
                }
            },
        )?;
    }
    documents.insert("result".into(), result.finish()?);
    documents.insert(
        "messages".into(),
        message_document(
            client,
            conversation,
            &super::super::store::HistoryScope::Round {
                task: uuid.into(),
                round: selected as u32,
            },
        )?,
    );
    Ok(Snapshot {
        task: snapshot,
        documents,
    })
}

fn read_content(
    client: &Client,
    conversation: &str,
    owner: &super::super::store::ContentOwner,
    sink: &mut dyn FnMut(&str) -> Result<()>,
) -> Result<()> {
    let mut offset = 0;
    loop {
        let chunk = client.call(&Request::HistoryContent {
            conversation: conversation.into(),
            owner: owner.clone(),
            offset,
        })?;
        sink(chunk["text"].as_str().unwrap_or_default())?;
        if chunk["done"] == true {
            return Ok(());
        }
        let next = chunk["next_offset"].as_u64().unwrap_or(0);
        crate::require(next > offset, "history content cursor did not advance")?;
        offset = next;
    }
}

fn message_document(
    client: &Client,
    conversation: &str,
    scope: &super::super::store::HistoryScope,
) -> Result<document::Document> {
    let mut builder = document::Builder::new(client.state_dir())?;
    let mut after = 0;
    loop {
        let page = client.call(&Request::HistoryMessages {
            conversation: conversation.into(),
            scope: scope.clone(),
            after,
            limit: Some(100),
        })?;
        for message in page["messages"].as_array().into_iter().flatten() {
            let header = format!(
                "{} → {} · {}\n",
                message["sender"].as_str().unwrap_or_default(),
                message["recipient"].as_str().unwrap_or_default(),
                message["state"].as_str().unwrap_or_default()
            );
            builder.append("plain", &header, |sink| {
                read_content(
                    client,
                    conversation,
                    &super::super::store::ContentOwner::Mail {
                        id: message["id"].as_u64().unwrap_or(0),
                    },
                    sink,
                )
            })?;
        }
        if page["has_more"] != true {
            break;
        }
        let next = page["next_after"].as_u64().unwrap_or(0);
        crate::require(next > after, "message cursor did not advance")?;
        after = next;
    }
    builder.append("plain", "", |_| Ok(()))?;
    builder.finish()
}

fn append_session(
    client: &Client,
    conversation: &str,
    turn: &str,
    builder: &mut document::Builder,
) -> Result<Vec<Value>> {
    let mut metadata = Vec::new();
    let mut after = 0;
    loop {
        let page = client.call(&Request::SessionEvents {
            conversation: conversation.into(),
            turn: turn.into(),
            after,
            limit: Some(100),
            include_messages: Some(false),
        })?;
        for mut event in page["events"].as_array().cloned().unwrap_or_default() {
            event["turn_id"] = turn.into();
            let kind = event["kind"].as_str().unwrap_or_default();
            let payload = &event["payload"];
            let (block, header, field) = match (kind, payload["type"].as_str().unwrap_or_default())
            {
                ("agent_final", _) => ("final", "Final answer\n", "text"),
                ("agent", "prompt") => ("prompt", "Prompt\n", "text"),
                ("agent", "user") => ("user", "User\n", "text"),
                ("agent", "text") => ("text", "", "delta"),
                ("agent", "tool") => {
                    builder.append("tool", "Tool call\n", |sink| {
                        sink(payload["name"].as_str().unwrap_or_default())?;
                        sink(" ")?;
                        sink(payload["detail"].as_str().unwrap_or_default())
                    })?;
                    continue;
                }
                ("turn_started", _) => {
                    metadata.clear();
                    metadata.push(event);
                    continue;
                }
                ("turn_finished", _) => {
                    if payload["status"] == "failed" || payload["status"] == "interrupted" {
                        builder.append("failure", "Session failed: ", |sink| {
                            sink(payload["error"].as_str().unwrap_or("interrupted"))
                        })?;
                    } else {
                        builder.append("end", "", |_| Ok(()))?;
                    }
                    metadata.push(event);
                    continue;
                }
                _ => continue,
            };
            builder.append(block, header, |sink| {
                if payload[field].get("session_content").is_some() {
                    let mut offset = 0;
                    loop {
                        let chunk = client.call(&Request::SessionContent {
                            conversation: conversation.into(),
                            event: event["seq"].as_u64().unwrap_or(0),
                            field: field.into(),
                            offset,
                        })?;
                        sink(chunk["text"].as_str().unwrap_or_default())?;
                        if chunk["done"] == true {
                            break;
                        }
                        let next = chunk["next_offset"].as_u64().unwrap_or(0);
                        crate::require(next > offset, "session content cursor did not advance")?;
                        offset = next;
                    }
                } else {
                    sink(payload[field].as_str().unwrap_or_default())?;
                }
                Ok(())
            })?;
        }
        if page["has_more"] != true {
            break;
        }
        let next = page["next_after"].as_u64().unwrap_or(0);
        crate::require(next > after, "session event cursor did not advance")?;
        after = next;
    }
    Ok(metadata)
}

pub(super) fn run_snapshot(
    client: &Client,
    conversation: &str,
    role: &str,
    selected: Option<u64>,
) -> Result<Snapshot> {
    let sessions = client.call(&Request::Sessions {
        conversation: conversation.into(),
    })?;
    let runs = sessions
        .as_array()
        .map(|runs| {
            runs.iter()
                .filter(|run| run["role"] == role)
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let latest = runs.len().saturating_sub(1);
    let index = selected.unwrap_or(latest as u64) as usize;
    let mut snapshot = serde_json::json!({"round":index,"_rounds":runs.iter().enumerate().map(|(index,run)|serde_json::json!({"round":index,"turn_id":run["turn_id"]})).collect::<Vec<_>>()});
    snapshot["_latest_run"] = runs
        .last()
        .map(|run| run["turn_id"].clone())
        .unwrap_or_default();
    if let Some(run) = runs.get(index) {
        snapshot["_run"] = run["turn_id"].clone();
        let mut builder = document::Builder::new(client.state_dir())?;
        let events = append_session(
            client,
            conversation,
            run["turn_id"].as_str().unwrap_or_default(),
            &mut builder,
        )?;
        snapshot["_transcripts"] = serde_json::json!({role:events});
        let mut documents = std::collections::HashMap::new();
        let document = builder.finish()?;
        documents.insert("review".into(), document.answer());
        documents.insert(role.to_owned(), document);
        documents.insert(
            "messages".into(),
            message_document(
                client,
                conversation,
                &super::super::store::HistoryScope::Session {
                    turn: run["turn_id"].as_str().unwrap_or_default().into(),
                },
            )?,
        );
        return Ok(Snapshot {
            task: snapshot,
            documents,
        });
    }
    Ok(Snapshot {
        task: snapshot,
        documents: Default::default(),
    })
}

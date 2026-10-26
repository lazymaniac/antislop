use super::*;

pub(super) fn agent_message_signature(request: &AgentMessage) -> Result<String> {
    crate::require(
        uuid::Uuid::parse_str(&request.operation_id).is_ok(),
        "operation_id must be a UUID",
    )?;
    crate::require(!request.text.trim().is_empty(), "the message is empty")?;
    Ok(crate::hash(serde_json::to_vec(&(
        &request.conversation,
        &request.plan,
        &request.recipient,
        &request.text,
    ))?))
}

pub(super) fn existing_agent_message(
    tx: &rusqlite::Transaction<'_>,
    request: &AgentMessage,
    signature: &str,
) -> Result<Option<Value>> {
    let existing: Option<(String, i64)> = tx.query_row(
        "SELECT signature,mail_id FROM agent_operations WHERE conversation_id=?1 AND operation_id=?2",
        params![request.conversation, request.operation_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional()?;
    existing
        .map(|(old, mail)| existing_agent_result(tx, signature, &old, mail))
        .transpose()
}

fn existing_agent_result(
    tx: &rusqlite::Transaction<'_>,
    signature: &str,
    old: &str,
    mail: i64,
) -> Result<Value> {
    crate::require(
        old == signature,
        "operation_id was already used for a different message",
    )?;
    let delivered: Option<String> = tx.query_row(
        "SELECT delivered_turn FROM mail WHERE id=?1",
        [mail],
        |row| row.get(0),
    )?;
    Ok(
        serde_json::json!({"mail":mail,"state":if delivered.is_some(){"delivered"}else{"queued"},"delivered_turn":delivered}),
    )
}

pub(super) fn validate_agent_recipient(
    tx: &rusqlite::Transaction<'_>,
    request: &AgentMessage,
) -> Result<Option<String>> {
    match request.recipient.split_once(':') {
        Some((role, id)) => validate_task_recipient(tx, request, role, id).map(Some),
        None => validate_role_recipient(tx, request).map(|()| None),
    }
}

fn validate_task_recipient(
    tx: &rusqlite::Transaction<'_>,
    request: &AgentMessage,
    role: &str,
    id: &str,
) -> Result<String> {
    crate::require(
        matches!(role, "worker" | "verifier") && uuid::Uuid::parse_str(id).is_ok(),
        "recipient must use a task UUID and an explicit role",
    )?;
    let (conversation, plan, round, status): (String, String, u32, String) = tx.query_row(
        "SELECT conversation_id,plan_id,round,status FROM tasks WHERE id=?1",
        [id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    validate_task_scope(request, &conversation, &plan, round, &status)?;
    Ok(id.to_owned())
}

fn validate_task_scope(
    request: &AgentMessage,
    conversation: &str,
    plan: &str,
    round: u32,
    status: &str,
) -> Result<()> {
    crate::require(
        status != "superseded",
        "task was retired; submit a reviewed revision before assigning work",
    )?;
    crate::require(
        status != "stale",
        "task inputs are stale; update the plan before sending work",
    )?;
    crate::require(
        conversation == request.conversation && request.plan.as_deref() == Some(plan),
        "task is outside the selected conversation or plan",
    )?;
    crate::require(
        request.expected_round == Some(round),
        "stale round; select the current round before sending",
    )
}

fn validate_role_recipient(tx: &rusqlite::Transaction<'_>, request: &AgentMessage) -> Result<()> {
    crate::require(
        matches!(
            request.recipient.as_str(),
            "orchestrator" | "auditor" | "plan-verifier"
        ),
        "unknown agent role",
    )?;
    let current: Option<String> = tx.query_row(
        "SELECT id FROM turns WHERE conversation_id=?1 AND role=?2 ORDER BY started_ms DESC,rowid DESC LIMIT 1",
        params![request.conversation, request.recipient], |row| row.get(0),
    ).optional()?;
    crate::require(
        current == request.expected_run,
        "stale run; select the current run before sending",
    )
}

pub(super) fn insert_agent_mail(
    tx: &rusqlite::Transaction<'_>,
    request: &AgentMessage,
    signature: &str,
    task: Option<&str>,
    stored_text: &str,
    now: i64,
) -> Result<i64> {
    insert_mail_row(tx, request, stored_text, now)?;
    let mail = tx.last_insert_rowid();
    insert_mail_scope_and_operation(tx, request, signature, task, mail)?;
    let user_message = insert_orchestrator_message(tx, request, stored_text, now)?;
    insert_mail_events(tx, request, stored_text, now, mail, user_message)?;
    Ok(mail)
}

fn insert_mail_row(
    tx: &rusqlite::Transaction<'_>,
    request: &AgentMessage,
    text: &str,
    now: i64,
) -> Result<()> {
    tx.execute("INSERT INTO mail(conversation_id,sender,recipient,kind,text,created_ms) VALUES(?1,'user',?2,'agent_work',?3,?4)",
        params![request.conversation, request.recipient, text, now])?;
    Ok(())
}

fn insert_mail_scope_and_operation(
    tx: &rusqlite::Transaction<'_>,
    request: &AgentMessage,
    signature: &str,
    task: Option<&str>,
    mail: i64,
) -> Result<()> {
    tx.execute(
        "INSERT INTO mail_scopes VALUES(?1,?2,?3,?4)",
        params![mail, task, request.expected_round, request.expected_run],
    )?;
    tx.execute(
        "INSERT INTO agent_operations VALUES(?1,?2,?3,?4)",
        params![request.conversation, request.operation_id, signature, mail],
    )?;
    Ok(())
}

fn insert_orchestrator_message(
    tx: &rusqlite::Transaction<'_>,
    request: &AgentMessage,
    text: &str,
    now: i64,
) -> Result<Option<i64>> {
    if request.recipient != "orchestrator" {
        return Ok(None);
    }
    tx.execute(
        "INSERT INTO messages(conversation_id,author,text,created_ms) VALUES(?1,'user',?2,?3)",
        params![request.conversation, text, now],
    )?;
    Ok(Some(tx.last_insert_rowid()))
}

fn insert_mail_events(
    tx: &rusqlite::Transaction<'_>,
    request: &AgentMessage,
    text: &str,
    now: i64,
    mail: i64,
    user_message: Option<i64>,
) -> Result<()> {
    let payload = serde_json::json!({"id":mail,"sender":"user","recipient":request.recipient,"text":text,"kind":"agent_work","state":"queued","created_ms":now,"user_message":user_message});
    tx.execute("INSERT INTO events(conversation_id,plan_id,kind,payload,created_ms) VALUES(?1,?2,'mail',?3,?4)",
        params![request.conversation, request.plan, payload.to_string(), now])?;
    if let Some(seq) = user_message {
        insert_user_message_event(tx, request, text, seq, now)?;
    }
    Ok(())
}

fn insert_user_message_event(
    tx: &rusqlite::Transaction<'_>,
    request: &AgentMessage,
    text: &str,
    seq: i64,
    now: i64,
) -> Result<()> {
    tx.execute(
        "INSERT INTO events(conversation_id,kind,payload,created_ms) VALUES(?1,'message',?2,?3)",
        params![
            request.conversation,
            serde_json::json!({"seq":seq,"author":"user","text":text}).to_string(),
            now
        ],
    )?;
    Ok(())
}

pub(super) fn update_agent_task(
    tx: &rusqlite::Transaction<'_>,
    request: &AgentMessage,
    task: Option<&str>,
    now: i64,
) -> Result<()> {
    let Some(task) = task else {
        return Ok(());
    };
    if request.recipient.starts_with("worker:") {
        super::invalidate_dependents(
            tx,
            task,
            request.plan.as_deref().expect("task plan"),
            &request.conversation,
            &request.operation_id,
            now,
        )?;
    }
    queue_agent_task(tx, task, request.recipient.starts_with("worker:"))
}

fn queue_agent_task(tx: &rusqlite::Transaction<'_>, task: &str, worker: bool) -> Result<()> {
    tx.execute(
        "UPDATE tasks SET status='queued',verdict=NULL,feedback=NULL,
        base=CASE WHEN status='approved' AND ?2 THEN NULL ELSE base END,
        result=CASE WHEN status='approved' AND ?2 THEN NULL ELSE result END
        WHERE id=?1 AND status NOT IN ('working','verifying','queued','proposed','stale')",
        params![task, worker],
    )?;
    Ok(())
}

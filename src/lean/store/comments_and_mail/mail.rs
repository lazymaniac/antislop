use super::*;

pub(super) fn insert_mail_row(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    role: &str,
    mail: &Mail,
    source_turn: Option<&str>,
    stored_text: &str,
) -> Result<Mail> {
    let now = now_ms();
    tx.execute(
        "INSERT INTO mail(conversation_id,sender,recipient,kind,text,created_ms) VALUES(?1,?2,?3,?4,?5,?6)",
        params![conversation, mail.sender, role, mail.kind, stored_text, now],
    )?;
    let stored = Mail {
        id: tx.last_insert_rowid() as u64,
        created_ms: now,
        delivered_turn: None,
        delivered_ms: None,
        state: "queued".into(),
        ..mail.clone()
    };
    if let Some(turn) = source_turn {
        append_mail_source(tx, stored.id, conversation, turn)?;
    }
    Ok(stored)
}

pub(super) fn append_mail_source(
    tx: &rusqlite::Transaction<'_>,
    mail_id: u64,
    conversation: &str,
    turn: &str,
) -> Result<()> {
    tx.execute(
        "INSERT INTO mail_sources(mail_id,turn_id) SELECT ?1,id FROM turns WHERE id=?2 AND conversation_id=?3",
        params![mail_id as i64, turn, conversation],
    )?;
    Ok(())
}

pub(super) fn append_mail_event(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    mail: &Mail,
    stored_text: &str,
) -> Result<()> {
    let mut payload = serde_json::to_value(mail)?;
    payload["text"] = stored_text.into();
    tx.execute(
        "INSERT INTO events(conversation_id,turn_id,plan_id,kind,payload,created_ms) VALUES(?1,NULL,NULL,'mail',?2,CAST(unixepoch('subsec')*1000 AS INTEGER))",
        params![conversation, payload.to_string()],
    )?;
    Ok(())
}

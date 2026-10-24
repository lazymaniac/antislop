use super::*;

pub(in crate::lean::store::comments_and_mail) fn insert_draft_comment(
    store: &Store,
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    comment: &Comment,
    stored_text: &str,
) -> Result<Comment> {
    let now = now_ms();
    tx.execute(
        "INSERT INTO comments(conversation_id,plan_id,target,label,text,status,message,created_ms,updated_ms) VALUES(?1,?2,?3,?4,?5,'draft',0,?6,?6)",
        params![conversation, comment.plan_id, comment.target, comment.label, stored_text, now],
    )?;
    let draft = Comment {
        id: tx.last_insert_rowid() as u64,
        status: "draft".into(),
        message: 0,
        ..comment.clone()
    };
    append_comment_event(store, tx, conversation, &draft)?;
    Ok(draft)
}

pub(in crate::lean::store::comments_and_mail) fn append_comment_event(
    store: &Store,
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    comment: &Comment,
) -> Result<()> {
    tx.execute(
        "INSERT INTO events(conversation_id,turn_id,plan_id,kind,payload,created_ms) VALUES(?1,NULL,?3,'comment',?2,CAST(unixepoch('subsec')*1000 AS INTEGER))",
        params![conversation, store.comment_payload(comment)?.to_string(), comment.plan_id],
    )?;
    Ok(())
}

pub(in crate::lean::store::comments_and_mail) fn persist_comment(
    store: &Store,
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    comment: &Comment,
    round: Option<u32>,
    run: Option<&str>,
    text: impl Fn(u64) -> String,
) -> Result<Comment> {
    let stored = store.content.encode(&comment.text)?;
    let now = now_ms();
    let id = insert_comment_row(tx, conversation, comment, &stored, now)?;
    let message_text = store.content.encode(&text(id))?;
    let message = insert_comment_message(
        tx,
        conversation,
        &message_text,
        now,
        comment.plan_id.as_deref(),
    )?;
    let saved = link_comment_message(tx, id, message, comment)?;
    save_comment_metadata(store, tx, conversation, &saved, round, run)?;
    Ok(saved)
}

fn link_comment_message(
    tx: &rusqlite::Transaction<'_>,
    id: u64,
    message: u64,
    comment: &Comment,
) -> Result<Comment> {
    set_comment_message(tx, id, message)?;
    Ok(Comment {
        id,
        status: "open".into(),
        message,
        ..comment.clone()
    })
}

pub(in crate::lean::store::comments_and_mail) fn insert_comment_row(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    comment: &Comment,
    stored_text: &str,
    now: i64,
) -> Result<u64> {
    tx.execute(
        "INSERT INTO comments(conversation_id,plan_id,target,label,text,status,message,created_ms,updated_ms) VALUES(?1,?2,?3,?4,?5,'open',0,?6,?6)",
        params![conversation, comment.plan_id, comment.target, comment.label, stored_text, now],
    )?;
    Ok(tx.last_insert_rowid() as u64)
}

pub(in crate::lean::store::comments_and_mail) fn insert_comment_message(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    text: &str,
    now: i64,
    plan_id: Option<&str>,
) -> Result<u64> {
    tx.execute(
        "INSERT INTO messages(conversation_id,author,text,created_ms) VALUES(?1,'user',?2,?3)",
        params![conversation, text, now],
    )?;
    let message = tx.last_insert_rowid() as u64;
    tx.execute(
        "INSERT INTO events(conversation_id,turn_id,plan_id,kind,payload,created_ms) VALUES(?1,NULL,?3,'message',?2,CAST(unixepoch('subsec')*1000 AS INTEGER))",
        params![conversation, serde_json::json!({"seq": message, "author": "user", "text": text}).to_string(), plan_id],
    )?;
    Ok(message)
}

pub(in crate::lean::store::comments_and_mail) fn append_scoped_comment_event(
    store: &Store,
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    comment: &Comment,
    round: Option<u32>,
    run: Option<&str>,
) -> Result<()> {
    let mut payload = store.comment_payload(comment)?;
    payload["round"] = serde_json::to_value(round)?;
    payload["run"] = serde_json::to_value(run)?;
    tx.execute(
        "INSERT INTO events(conversation_id,turn_id,plan_id,kind,payload,created_ms) VALUES(?1,NULL,?3,'comment',?2,CAST(unixepoch('subsec')*1000 AS INTEGER))",
        params![conversation, payload.to_string(), comment.plan_id],
    )?;
    Ok(())
}

pub(in crate::lean::store::comments_and_mail) fn set_comment_message(
    tx: &rusqlite::Transaction<'_>,
    comment_id: u64,
    message: u64,
) -> Result<()> {
    tx.execute(
        "UPDATE comments SET message=?1 WHERE id=?2",
        params![message as i64, comment_id as i64],
    )?;
    Ok(())
}

fn save_comment_metadata(
    store: &Store,
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    comment: &Comment,
    round: Option<u32>,
    run: Option<&str>,
) -> Result<()> {
    tx.execute(
        "INSERT INTO comment_scopes(comment_id,round,run) VALUES(?1,?2,?3)",
        params![comment.id as i64, round, run],
    )?;
    append_scoped_comment_event(store, tx, conversation, comment, round, run)
}

use super::insert::append_comment_event;
use super::*;

pub(in crate::lean::store::comments_and_mail) fn draft_comments(
    store: &Store,
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    plan_id: &str,
) -> Result<Vec<Comment>> {
    let drafts = store.comment_rows(
        tx,
        "conversation_id=?1 AND status='draft' AND (plan_id=?2 OR plan_id IS NULL)",
        params![conversation, plan_id],
    )?;
    if drafts.is_empty() {
        return Err(Error::Invalid("there are no draft comments to send".into()));
    }
    Ok(drafts)
}

pub(in crate::lean::store::comments_and_mail) fn draft_message(drafts: &[Comment]) -> String {
    drafts
        .iter()
        .map(|comment| {
            format!(
                "{}{} on {}] {}",
                crate::lean::COMMENT_PREFIX,
                comment.id,
                comment.label,
                comment.text
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub(in crate::lean::store::comments_and_mail) fn insert_draft_message(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    plan_id: &str,
    text: &str,
    now: i64,
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

pub(in crate::lean::store::comments_and_mail) fn open_draft_comments(
    store: &Store,
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    drafts: Vec<Comment>,
    message: u64,
    now: i64,
) -> Result<Vec<Comment>> {
    let mut sent = Vec::with_capacity(drafts.len());
    for mut comment in drafts {
        reopen_draft_comment(store, tx, conversation, &mut comment, message, now)?;
        sent.push(comment);
    }
    Ok(sent)
}

pub(in crate::lean::store::comments_and_mail) fn reopen_draft_comment(
    store: &Store,
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    comment: &mut Comment,
    message: u64,
    now: i64,
) -> Result<()> {
    tx.execute(
        "UPDATE comments SET status='open', message=?1, updated_ms=?2 WHERE id=?3",
        params![message as i64, now, comment.id as i64],
    )?;
    comment.status = "open".into();
    comment.message = message;
    append_comment_event(store, tx, conversation, comment)
}

pub(in crate::lean::store::comments_and_mail) fn update_comment_status(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    id: u64,
    status: &str,
) -> Result<()> {
    let changed = tx.execute(
        "UPDATE comments SET status=?1, updated_ms=?2 WHERE id=?3 AND conversation_id=?4",
        params![status, now_ms(), id as i64, conversation],
    )?;
    if changed == 0 {
        return Err(Error::Invalid(format!("no comment {id}")));
    }
    Ok(())
}

pub(in crate::lean::store::comments_and_mail) fn load_comment(
    store: &Store,
    tx: &rusqlite::Transaction<'_>,
    id: u64,
) -> Result<Comment> {
    store
        .comment_rows(tx, "id=?1", params![id as i64])?
        .pop()
        .ok_or_else(|| Error::Invalid(format!("no comment {id}")))
}

pub(in crate::lean::store::comments_and_mail) fn ensure_comment_was_sent(
    comment: &Comment,
    status: &str,
) -> Result<()> {
    if comment.message == 0 && status != "deleted" {
        return Err(Error::Invalid(
            "this comment is a draft: delete it, or send it with Review".into(),
        ));
    }
    Ok(())
}

pub(in crate::lean::store::comments_and_mail) fn reopen_comment(
    store: &Store,
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    id: u64,
    comment: &mut Comment,
) -> Result<()> {
    let text = format!(
        "{}{} on {}] {}",
        crate::lean::COMMENT_PREFIX,
        id,
        comment.label,
        comment.text
    );
    let text = store.content.encode(&text)?;
    tx.execute(
        "INSERT INTO messages(conversation_id,author,text,created_ms) VALUES(?1,'user',?2,?3)",
        params![conversation, text, now_ms()],
    )?;
    comment.message = tx.last_insert_rowid() as u64;
    update_reopened_comment(tx, id, comment.message)?;
    append_reopened_message_event(tx, conversation, comment, &text)
}

pub(in crate::lean::store::comments_and_mail) fn update_reopened_comment(
    tx: &rusqlite::Transaction<'_>,
    id: u64,
    message: u64,
) -> Result<()> {
    tx.execute(
        "UPDATE comments SET message=?1 WHERE id=?2",
        params![message as i64, id as i64],
    )?;
    Ok(())
}

pub(in crate::lean::store::comments_and_mail) fn append_reopened_message_event(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    comment: &Comment,
    text: &str,
) -> Result<()> {
    tx.execute(
        "INSERT INTO events(conversation_id,plan_id,kind,payload,created_ms) VALUES(?1,?2,'message',?3,?4)",
        params![conversation, comment.plan_id, serde_json::json!({"seq": comment.message, "author": "user", "text": text}).to_string(), now_ms()],
    )?;
    Ok(())
}

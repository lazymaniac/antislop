use super::*;

type MessageRow = (i64, String, String, String, Option<String>);
type EventRow = (i64, String, String, i64);
const HISTORY_MESSAGES_SQL: &str = "SELECT m.id,m.sender,m.recipient,m.text,m.delivered_turn FROM mail m LEFT JOIN mail_scopes s ON s.mail_id=m.id
    WHERE m.conversation_id=?1 AND m.id>?5 AND (
        (?2 IS NOT NULL AND ((s.task_id=?2 AND s.round=?3 AND m.delivered_turn IS NULL AND NOT EXISTS(SELECT 1 FROM mail_sources source WHERE source.mail_id=m.id))
            OR m.delivered_turn IN (SELECT turn_id FROM turn_scopes WHERE task_id=?2 AND round=?3)
            OR m.id IN (SELECT mail_id FROM mail_sources WHERE turn_id IN (SELECT turn_id FROM turn_scopes WHERE task_id=?2 AND round=?3))))
        OR (?4 IS NOT NULL AND (m.delivered_turn=?4 OR m.id IN (SELECT mail_id FROM mail_sources WHERE turn_id=?4)))) ORDER BY m.id LIMIT ?6";

impl Store {
    /// Message metadata and content references are paged independently of the
    /// event cursor. No first event page materializes the entire mailbox.
    pub fn history_messages(
        &self,
        conversation: &str,
        scope: &HistoryScope,
        after: u64,
        limit: usize,
    ) -> Result<Value> {
        let (task, round, turn) = self.validate_history_scope(conversation, scope)?;
        let limit = limit.clamp(1, 500);
        let rows = self.history_message_rows(conversation, &task, round, &turn, after, limit)?;
        let (messages, has_more) = self.shape_history_messages(rows, limit)?;
        let next_after = messages
            .last()
            .and_then(|m| m["id"].as_u64())
            .unwrap_or(after);
        Ok(serde_json::json!({"messages":messages,"next_after":next_after,"has_more":has_more}))
    }

    fn validate_history_scope(
        &self,
        conversation: &str,
        scope: &HistoryScope,
    ) -> Result<(Option<String>, Option<u32>, Option<String>)> {
        let db = self.db();
        match scope {
            HistoryScope::Round { task, round } => {
                let owns: bool = db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM tasks WHERE id=?1 AND conversation_id=?2)",
                    params![task, conversation],
                    |r| r.get(0),
                )?;
                crate::require(owns, "task is outside this conversation")?;
                Ok((Some(task.clone()), Some(*round), None))
            }
            HistoryScope::Session { turn } => {
                let owns: bool = db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM turns WHERE id=?1 AND conversation_id=?2)",
                    params![turn, conversation],
                    |r| r.get(0),
                )?;
                crate::require(owns, "session is outside this conversation")?;
                Ok((None, None, Some(turn.clone())))
            }
        }
    }

    fn history_message_rows(
        &self,
        conversation: &str,
        task: &Option<String>,
        round: Option<u32>,
        turn: &Option<String>,
        after: u64,
        limit: usize,
    ) -> Result<Vec<MessageRow>> {
        let db = self.db();
        let mut statement = db.prepare(HISTORY_MESSAGES_SQL)?;
        statement
            .query_map(
                params![
                    conversation,
                    task,
                    round,
                    turn,
                    after as i64,
                    limit as i64 + 1
                ],
                message_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    fn shape_history_messages(
        &self,
        rows: Vec<MessageRow>,
        limit: usize,
    ) -> Result<(Vec<Value>, bool)> {
        let mut messages = Vec::new();
        let mut has_more = false;
        for (id, sender, recipient, text, delivered) in rows {
            if messages.len() == limit {
                has_more = true;
                break;
            }
            let reference = crate::lean::content::Content::reference(&text)?;
            let content = serde_json::json!({"session_content":reference});
            messages.push(serde_json::json!({"id":id,"sender":sender,"recipient":recipient,"content":content,"delivered_turn":delivered,"state":if delivered.is_some(){"delivered"}else{"queued"}}));
        }
        Ok((messages, has_more))
    }

    /// Event payloads retain references, allowing clients to fetch text chunks
    /// instead of loading every prompt and answer when listing a session.
    pub fn session_events(
        &self,
        conversation: &str,
        turn: &str,
        after: u64,
        limit: usize,
    ) -> Result<Value> {
        self.session_events_with_messages(conversation, turn, after, limit, true)
    }

    pub fn session_events_with_messages(
        &self,
        conversation: &str,
        turn: &str,
        after: u64,
        limit: usize,
        include_messages: bool,
    ) -> Result<Value> {
        self.require_session(conversation, turn)?;
        let db = self.db();
        let limit = limit.clamp(1, 500);
        let rows = load_session_event_rows(&db, conversation, turn, after, limit)?;
        drop(db);
        let (events, has_more) = shape_session_events(rows, limit)?;
        let messages = self.session_messages(conversation, turn, after, include_messages)?;
        let next_after = events
            .last()
            .and_then(|event| event["seq"].as_u64())
            .unwrap_or(after);
        Ok(
            serde_json::json!({"events":events,"messages":messages,"next_after":next_after,"has_more":has_more}),
        )
    }

    fn require_session(&self, conversation: &str, turn: &str) -> Result<()> {
        let owns: bool = self.db().query_row(
            "SELECT EXISTS(SELECT 1 FROM turns WHERE id=?1 AND conversation_id=?2)",
            params![turn, conversation],
            |row| row.get(0),
        )?;
        crate::require(owns, "session is outside this conversation")?;
        Ok(())
    }

    fn session_messages(
        &self,
        conversation: &str,
        turn: &str,
        after: u64,
        include_messages: bool,
    ) -> Result<Vec<Value>> {
        if after != 0 || !include_messages {
            return Ok(Vec::new());
        }
        let db = self.db();
        let mut statement = db.prepare("SELECT m.id,m.sender,m.recipient,m.text,m.delivered_turn FROM mail m WHERE m.conversation_id=?1
                AND (m.delivered_turn=?2 OR m.id IN (SELECT mail_id FROM mail_sources WHERE turn_id=?2)) ORDER BY m.id")?;
        statement.query_map(params![conversation,turn],|row|Ok(serde_json::json!({"id":row.get::<_,i64>(0)?,"sender":row.get::<_,String>(1)?,"recipient":row.get::<_,String>(2)?,"text":self.content.sql_text(row,3)?,"delivered_turn":row.get::<_,Option<String>>(4)?})))?.collect::<std::result::Result<Vec<_>,_>>().map_err(Into::into)
    }
}

fn message_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MessageRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
    ))
}

fn load_session_event_rows(
    db: &rusqlite::Connection,
    conversation: &str,
    turn: &str,
    after: u64,
    limit: usize,
) -> Result<Vec<EventRow>> {
    let mut statement = db.prepare("SELECT seq,kind,payload,created_ms FROM events WHERE conversation_id=?1 AND turn_id=?2 AND seq>?3 ORDER BY seq LIMIT ?4")?;
    statement
        .query_map(
            params![conversation, turn, after as i64, limit as i64 + 1],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn shape_session_events(rows: Vec<EventRow>, limit: usize) -> Result<(Vec<Value>, bool)> {
    let mut events = Vec::with_capacity(limit);
    let mut has_more = false;
    for (seq, kind, payload, created_ms) in rows {
        if events.len() == limit {
            has_more = true;
            break;
        }
        events.push(serde_json::json!({"seq": seq,"kind":kind,"payload":serde_json::from_str::<Value>(&payload)?,"created_ms":created_ms}));
    }
    Ok((events, has_more))
}

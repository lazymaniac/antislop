use super::*;

#[path = "mail_and_events/event_encoding.rs"]
mod event_encoding;

impl Store {
    /// Plans with durable task mail that still needs a successful recipient turn.
    pub fn pending_mail_plans(&self) -> Result<Vec<(String, String)>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT DISTINCT t.conversation_id,t.plan_id FROM tasks t JOIN mail m
             ON m.conversation_id=t.conversation_id
             AND m.recipient IN ('worker:' || t.id, 'verifier:' || t.id, 'code-review:' || t.id)
             WHERE m.delivered_turn IS NULL",
        )?;
        Ok(statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// All mail in the conversation with queue and delivery states, oldest first.
    pub fn mail(&self, conversation: &str) -> Result<Vec<Mail>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT id,sender,recipient,kind,text,created_ms,delivered_turn,delivered_ms FROM mail
             WHERE conversation_id=?1 ORDER BY id",
        )?;
        let rows = statement.query_map([conversation], |row| {
            let delivered_turn: Option<String> = row.get(6)?;
            let state = if delivered_turn.is_some() {
                "delivered"
            } else {
                "queued"
            };
            Ok(Mail {
                id: row.get::<_, i64>(0)? as u64,
                sender: row.get(1)?,
                recipient: row.get(2)?,
                kind: row.get(3)?,
                text: self.content.sql_text(row, 4)?,
                created_ms: row.get(5)?,
                delivered_turn,
                delivered_ms: row.get(7)?,
                state: state.into(),
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Marks messages delivered by a turn that completed, and announces it.
    pub fn mark_delivered(&self, conversation: &str, ids: &[u64], turn: &str) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let mut db = self.db();
        let tx = db.transaction()?;
        let now = now_ms();
        for id in ids {
            tx.execute(
                "UPDATE mail SET delivered_turn=?2,delivered_ms=?3 WHERE id=?1",
                params![*id as i64, turn, now],
            )?;
        }
        let recipient: Option<String> = tx
            .query_row("SELECT role FROM turns WHERE id=?1", [turn], |row| {
                row.get(0)
            })
            .optional()?;
        tx.execute(
            "INSERT INTO events(conversation_id,turn_id,plan_id,kind,payload,created_ms) VALUES(?1,?2,(SELECT plan_id FROM turns WHERE id=?2),'mail_delivered',?3,CAST(unixepoch('subsec')*1000 AS INTEGER))",
            params![conversation, turn, serde_json::json!({"ids": ids, "delivered_ms": now, "state": "delivered","recipient":recipient}).to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn sessions(&self, conversation: &str) -> Result<Vec<SessionInfo>> {
        self.conversation(conversation)?;
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT id, plan_id, role, provider, session_id, status, error, upto_message, started_ms, finished_ms, pid, (SELECT backend FROM run_contexts r WHERE r.turn_id=turns.id)
             FROM turns WHERE conversation_id=?1 ORDER BY started_ms ASC, rowid ASC",
        )?;
        let rows = statement.query_map([conversation], |row| {
            Ok(SessionInfo {
                turn_id: row.get(0)?,
                plan_id: row.get(1)?,
                role: row.get(2)?,
                provider: row.get(3)?,
                session_id: row.get(4)?,
                status: row.get(5)?,
                error: row.get(6)?,
                upto_message: row.get::<_, i64>(7)? as u64,
                started_ms: row.get(8)?,
                finished_ms: row.get(9)?,
                pid: row.get(10)?,
                backend: row.get(11)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn latest_event_seq(&self, conversation: &str) -> Result<u64> {
        let db = self.db();
        let seq: Option<i64> = db
            .query_row(
                "SELECT seq FROM events WHERE (conversation_id=?1 OR conversation_id IS NULL) ORDER BY seq DESC LIMIT 1",
                [conversation],
                |row| row.get(0),
            )
            .optional()?;
        Ok(seq.unwrap_or(0) as u64)
    }

    pub fn event_bounds(&self, conversation: &str) -> Result<(u64, u64, usize)> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT COALESCE(MIN(seq), 0), COALESCE(MAX(seq), 0), COUNT(*)
             FROM events WHERE (conversation_id=?1 OR conversation_id IS NULL)",
        )?;
        let row = statement.query_row([conversation], |row| {
            Ok((
                row.get::<_, i64>(0)? as u64,
                row.get::<_, i64>(1)? as u64,
                row.get::<_, i64>(2)? as usize,
            ))
        })?;
        Ok(row)
    }

    pub fn history_page(
        &self,
        conversation: &str,
        after: u64,
        limit: usize,
    ) -> Result<HistoryPage> {
        self.conversation(conversation)?;
        let (oldest_seq, latest_seq, total) = self.event_bounds(conversation)?;
        let events = self.events_since_budget(conversation, after, limit, 1024 * 1024)?;
        let next_after = events.last().map(|e| e.seq);
        let has_more = match next_after {
            Some(last_seq) => last_seq < latest_seq,
            None => false,
        };
        let history_unavailable = total > 0
            && (after > latest_seq || (oldest_seq > 1 && after < oldest_seq.saturating_sub(1)));
        Ok(HistoryPage {
            events,
            after,
            next_after,
            has_more,
            total,
            oldest_seq,
            latest_seq,
            history_unavailable,
        })
    }

    /// Reserve the event before publishing an already prepared file. The callback
    /// must only rename the file and swap the snapshot; writes and fsync happen outside this lock.
    pub fn publish_config_with_event(
        &self,
        path: &Path,
        publish: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        let db = self.db();
        let transaction = db.unchecked_transaction()?;
        let payload = serde_json::json!({"path": path});
        transaction.execute("INSERT INTO events(conversation_id,kind,payload,created_ms) VALUES(NULL,'config_updated',?1,CAST(unixepoch('subsec')*1000 AS INTEGER))", [payload.to_string()])?;
        publish()?;
        transaction.commit()?;
        Ok(())
    }

    pub fn append_event(
        &self,
        conversation: &str,
        turn: Option<&str>,
        plan_id: Option<&str>,
        kind: &str,
        payload: &Value,
    ) -> Result<u64> {
        let payload = self.encode_event_payload(kind, payload)?;
        let db = self.db();
        let resolved_plan = self.resolve_event_plan(&db, turn, plan_id, &payload)?;
        db.execute(
            "INSERT INTO events(conversation_id,turn_id,plan_id,kind,payload,created_ms) VALUES(?1,?2,?3,?4,?5,CAST(unixepoch('subsec')*1000 AS INTEGER))",
            params![conversation, turn, resolved_plan, kind, payload.to_string()],
        )?;
        Ok(db.last_insert_rowid() as u64)
    }

    pub fn events_since(&self, conversation: &str, after: u64, limit: usize) -> Result<Vec<Event>> {
        self.events_since_budget(conversation, after, limit, usize::MAX)
    }

    /// The first event is always returned whole, even when it is larger than
    /// the page budget. No later event is discarded from the durable stream.
    pub fn events_since_budget(
        &self,
        conversation: &str,
        after: u64,
        limit: usize,
        budget: usize,
    ) -> Result<Vec<Event>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT seq,turn_id,plan_id,kind,payload,created_ms FROM events
             WHERE (conversation_id=?1 OR conversation_id IS NULL) AND seq>?2 ORDER BY seq LIMIT ?3",
        )?;
        let rows =
            statement.query_map(params![conversation, after as i64, limit as i64], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            })?;
        self.collect_events(rows, budget)
    }

    fn collect_events(
        &self,
        rows: rusqlite::MappedRows<
            '_,
            impl FnMut(
                &rusqlite::Row<'_>,
            )
                -> rusqlite::Result<(i64, Option<String>, Option<String>, String, String, i64)>,
        >,
        budget: usize,
    ) -> Result<Vec<Event>> {
        let mut events = Vec::new();
        let mut bytes = 0usize;
        for row in rows {
            let event = self.decode_event_row(row?)?;
            let weight = event.payload.to_string().len();
            if !events.is_empty() && bytes.saturating_add(weight) > budget {
                break;
            }
            bytes = bytes.saturating_add(weight);
            events.push(event);
            if bytes >= budget {
                break;
            }
        }
        Ok(events)
    }

    fn decode_event_row(
        &self,
        (seq, turn_id, plan_id, kind, encoded, created_ms): (
            i64,
            Option<String>,
            Option<String>,
            String,
            String,
            i64,
        ),
    ) -> Result<Event> {
        Ok(Event {
            seq: seq as u64,
            turn_id,
            plan_id,
            payload: self.hydrate_event_payload(&kind, &encoded)?,
            kind,
            created_ms,
        })
    }
}

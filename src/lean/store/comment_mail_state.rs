use super::*;

impl Store {
    /// Resolve only plan feedback included in the successful revision.
    pub fn resolve_plan_comments(&self, conversation: &str, upto: u64) -> Result<()> {
        self.resolve_listed_plan_comments(conversation, upto, None)
    }

    /// Like [`Self::resolve_plan_comments`], but only the comments in `only`
    /// when given: the ones the comment agent said its plan took care of.
    pub fn resolve_listed_plan_comments(
        &self,
        conversation: &str,
        upto: u64,
        only: Option<&[u64]>,
    ) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let comments = self.comment_rows(
            &tx,
            "conversation_id=?1 AND status='open' AND message<=?2 AND (target='plan' OR target LIKE 'plan:%')",
            params![conversation, upto as i64],
        )?;
        for mut comment in comments {
            if only.is_some_and(|ids| !ids.contains(&comment.id)) {
                continue;
            }
            tx.execute(
                "UPDATE comments SET status='resolved',updated_ms=?1 WHERE id=?2",
                params![now_ms(), comment.id as i64],
            )?;
            comment.status = "resolved".into();
            tx.execute("INSERT INTO events(conversation_id,plan_id,kind,payload,created_ms) VALUES(?1,?2,'comment',?3,?4)",
                params![conversation, comment.plan_id, serde_json::to_string(&comment)?, now_ms()])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// A selective comment claim survives deferred options/requirements planning,
    /// but never a published plan or a subsequent user message.
    pub fn pending_plan_comment_ids(&self, conversation: &str) -> Result<Option<Vec<u64>>> {
        let encoded: Option<String> = self.db().query_row(
            "SELECT json_extract(e.payload,'$.ids') FROM events e
             WHERE e.conversation_id=?1 AND e.kind='plan_comments_addressed'
             AND NOT EXISTS(SELECT 1 FROM events p WHERE p.conversation_id=?1 AND p.kind='plan_proposed' AND p.seq>e.seq)
             AND NOT EXISTS(SELECT 1 FROM messages m WHERE m.conversation_id=?1 AND m.author='user' AND m.seq>json_extract(e.payload,'$.upto_message'))
             ORDER BY e.seq DESC LIMIT 1",
            params![conversation], |row| row.get(0),
        ).optional()?;
        encoded
            .map(|text| serde_json::from_str(&text).map_err(Error::from))
            .transpose()
    }

    /// The rendered plan document is runtime context, never repository contents.
    pub fn plan_document(&self, conversation: &str, plan: &str) -> Result<Option<String>> {
        let document = self.db().query_row(
            "SELECT json_extract(payload,'$.document') FROM events WHERE conversation_id=?1 AND plan_id=?2 AND kind='plan_proposed' ORDER BY seq DESC LIMIT 1",
            params![conversation, plan], |row| row.get::<_, Option<String>>(0),
        ).optional()?.flatten();
        if document.is_some() {
            return Ok(document);
        }
        // A rejected candidate has task rows but no published document yet.
        // Both orchestration sessions need its specs when revising it.
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT spec FROM tasks WHERE conversation_id=?1 AND plan_id=?2 ORDER BY position",
        )?;
        let specs = statement
            .query_map(params![conversation, plan], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if specs.is_empty() {
            Ok(None)
        } else {
            Ok(Some(format!("[{}]", specs.join(","))))
        }
    }

    /// Pauses or continues the conversation's plan work, with its event.
    pub fn set_paused(&self, conversation: &str, paused: bool) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let changed = if paused {
            tx.execute(
                "INSERT OR IGNORE INTO pauses(conversation_id) VALUES(?1)",
                [conversation],
            )?
        } else {
            tx.execute(
                "DELETE FROM pauses WHERE conversation_id=?1",
                [conversation],
            )?
        };
        if changed > 0 {
            tx.execute(
                "INSERT INTO events(conversation_id,turn_id,plan_id,kind,payload,created_ms) VALUES(?1,NULL,NULL,'paused',?2,CAST(unixepoch('subsec')*1000 AS INTEGER))",
                params![conversation, serde_json::json!({"paused": paused}).to_string()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Sets the conversation's mode for what it does from now on, with its
    /// event when it changes.
    pub fn set_mode(&self, conversation: &str, autonomous: bool) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let before: Option<bool> = tx
            .query_row(
                "SELECT autonomous FROM modes WHERE conversation_id=?1",
                [conversation],
                |row| row.get(0),
            )
            .optional()?;
        if before != Some(autonomous) {
            tx.execute(
                "INSERT INTO modes(conversation_id,autonomous) VALUES(?1,?2)
                 ON CONFLICT(conversation_id) DO UPDATE SET autonomous=excluded.autonomous",
                params![conversation, autonomous],
            )?;
            tx.execute(
                "INSERT INTO events(conversation_id,turn_id,plan_id,kind,payload,created_ms) VALUES(?1,NULL,NULL,'mode',?2,CAST(unixepoch('subsec')*1000 AS INTEGER))",
                params![
                    conversation,
                    serde_json::json!({"autonomous": autonomous}).to_string()
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The mode the user set for the conversation, if any.
    pub fn mode(&self, conversation: &str) -> Result<Option<bool>> {
        Ok(self
            .db()
            .query_row(
                "SELECT autonomous FROM modes WHERE conversation_id=?1",
                [conversation],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn paused(&self, conversation: &str) -> Result<bool> {
        Ok(self
            .db()
            .query_row(
                "SELECT 1 FROM pauses WHERE conversation_id=?1",
                [conversation],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// The conversation's comments with that status, oldest first.
    pub fn comments(&self, conversation: &str, status: &str) -> Result<Vec<Comment>> {
        self.comment_rows(
            &self.db(),
            "conversation_id=?1 AND status=?2",
            params![conversation, status],
        )
    }

    pub(super) fn comment_payload(&self, comment: &Comment) -> Result<Value> {
        let mut payload = serde_json::to_value(comment)?;
        payload["text"] = self.content.encode(&comment.text)?.into();
        Ok(payload)
    }

    pub(super) fn comment_rows(
        &self,
        db: &Connection,
        filter: &str,
        values: impl rusqlite::Params,
    ) -> Result<Vec<Comment>> {
        let mut statement = db.prepare(&format!(
            "SELECT id,plan_id,target,label,text,status,message FROM comments
             WHERE {filter} ORDER BY id"
        ))?;
        let rows = statement.query_map(values, |row| {
            Ok(Comment {
                id: row.get::<_, i64>(0)? as u64,
                plan_id: row.get(1)?,
                target: row.get(2)?,
                label: row.get(3)?,
                text: self.content.sql_text(row, 4)?,
                status: row.get(5)?,
                message: row.get::<_, i64>(6)? as u64,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Messages for `role` not yet delivered, oldest first.
    pub fn pending_mail(&self, conversation: &str, role: &str) -> Result<Vec<Mail>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT id,sender,recipient,kind,text,created_ms FROM mail
             WHERE conversation_id=?1 AND recipient=?2 AND delivered_turn IS NULL ORDER BY id",
        )?;
        let rows = statement.query_map(params![conversation, role], |row| {
            Ok(Mail {
                id: row.get::<_, i64>(0)? as u64,
                sender: row.get(1)?,
                recipient: row.get(2)?,
                kind: row.get(3)?,
                text: self.content.sql_text(row, 4)?,
                created_ms: row.get(5)?,
                delivered_turn: None,
                delivered_ms: None,
                state: "queued".into(),
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn pending_mail_conversations(&self) -> Result<Vec<String>> {
        let db = self.db();
        let mut statement = db.prepare("SELECT DISTINCT conversation_id FROM mail WHERE delivered_turn IS NULL AND kind!='copy'")?;
        Ok(statement
            .query_map([], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn pending_mail_targets(&self, conversation: &str) -> Result<Vec<MailTarget>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT m.recipient,MAX(m.id),t.id,t.plan_id FROM mail m LEFT JOIN tasks t
             ON t.conversation_id=m.conversation_id AND m.recipient IN ('worker:' || t.id,'verifier:' || t.id,'code-review:' || t.id)
             WHERE m.conversation_id=?1 AND m.delivered_turn IS NULL AND m.kind!='copy'
             GROUP BY m.recipient ORDER BY MIN(m.id)")?;
        Ok(statement
            .query_map([conversation], |row| {
                Ok(MailTarget {
                    role: row.get(0)?,
                    newest: row.get::<_, i64>(1)? as u64,
                    task_id: row.get(2)?,
                    plan_id: row.get(3)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }
}

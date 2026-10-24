use super::*;

impl Store {
    /// Approved writers for this project, most recently approved first.
    /// Task events give completion order, including writers from older plans
    /// and conversations; creation order need not match execution order.
    pub fn approved_writer_results(&self, project: &str) -> Result<Vec<(String, String)>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT t.base,t.result FROM events e
             JOIN tasks t ON t.id=json_extract(e.payload,'$.task')
             JOIN conversations c ON c.id=t.conversation_id
             WHERE c.project=?1 AND e.conversation_id=t.conversation_id
               AND e.kind='task' AND json_extract(e.payload,'$.status')='approved'
               AND t.status='approved' AND t.base IS NOT NULL AND t.result IS NOT NULL
             GROUP BY t.id ORDER BY max(e.seq) DESC",
        )?;
        let rows = statement.query_map([project], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Records a message for `role` and announces it.
    pub fn add_mail(&self, conversation: &str, role: &str, mail: &Mail) -> Result<Mail> {
        self.add_mail_from(conversation, role, mail, None)
    }

    pub fn add_mail_from(
        &self,
        conversation: &str,
        role: &str,
        mail: &Mail,
        source_turn: Option<&str>,
    ) -> Result<Mail> {
        let stored_text = self.content.encode(&mail.text)?;
        let mut db = self.db();
        let tx = db.transaction()?;
        let now = now_ms();
        tx.execute(
            "INSERT INTO mail(conversation_id,sender,recipient,kind,text,created_ms)
             VALUES(?1,?2,?3,?4,?5,?6)",
            params![conversation, mail.sender, role, mail.kind, stored_text, now],
        )?;
        let mail = Mail {
            id: tx.last_insert_rowid() as u64,
            created_ms: now,
            delivered_turn: None,
            delivered_ms: None,
            state: "queued".into(),
            ..mail.clone()
        };
        if let Some(turn) = source_turn {
            tx.execute("INSERT INTO mail_sources(mail_id,turn_id) SELECT ?1,id FROM turns WHERE id=?2 AND conversation_id=?3",params![mail.id as i64,turn,conversation])?;
        }
        let mut payload = serde_json::to_value(&mail)?;
        payload["text"] = stored_text.into();
        tx.execute(
            "INSERT INTO events(conversation_id,turn_id,plan_id,kind,payload,created_ms) VALUES(?1,NULL,NULL,'mail',?2,CAST(unixepoch('subsec')*1000 AS INTEGER))",
            params![conversation, payload.to_string()],
        )?;
        tx.commit()?;
        Ok(mail)
    }

    /// Stores a comment with the message that tells the orchestrator about
    /// it (`text` gets the comment's id), in one transaction.
    pub fn add_comment(
        &self,
        conversation: &str,
        comment: &Comment,
        text: impl Fn(u64) -> String,
    ) -> Result<Comment> {
        self.add_comment_at(conversation, comment, None, None, text)
    }

    pub fn add_comment_at(
        &self,
        conversation: &str,
        comment: &Comment,
        round: Option<u32>,
        run: Option<&str>,
        text: impl Fn(u64) -> String,
    ) -> Result<Comment> {
        let stored_text = self.content.encode(&comment.text)?;
        let mut db = self.db();
        let tx = db.transaction()?;
        let now = now_ms();
        tx.execute(
            "INSERT INTO comments(conversation_id,plan_id,target,label,text,status,message,
                                  created_ms,updated_ms)
             VALUES(?1,?2,?3,?4,?5,'open',0,?6,?6)",
            params![
                conversation,
                comment.plan_id,
                comment.target,
                comment.label,
                stored_text,
                now
            ],
        )?;
        let id = tx.last_insert_rowid() as u64;
        let text = self.content.encode(&text(id))?;
        tx.execute(
            "INSERT INTO messages(conversation_id,author,text,created_ms) VALUES(?1,'user',?2,?3)",
            params![conversation, text, now],
        )?;
        let message = tx.last_insert_rowid() as u64;
        tx.execute(
            "INSERT INTO events(conversation_id,turn_id,plan_id,kind,payload,created_ms)
             VALUES(?1,NULL,?3,'message',?2,CAST(unixepoch('subsec')*1000 AS INTEGER))",
            params![
                conversation,
                serde_json::json!({"seq": message, "author": "user", "text": text}).to_string(),
                comment.plan_id
            ],
        )?;
        tx.execute(
            "UPDATE comments SET message=?1 WHERE id=?2",
            params![message as i64, id as i64],
        )?;
        let comment = Comment {
            id,
            status: "open".into(),
            message,
            ..comment.clone()
        };
        tx.execute(
            "INSERT INTO comment_scopes(comment_id,round,run) VALUES(?1,?2,?3)",
            params![comment.id as i64, round, run],
        )?;
        let mut payload = self.comment_payload(&comment)?;
        payload["round"] = serde_json::to_value(round)?;
        payload["run"] = serde_json::to_value(run)?;
        tx.execute(
            "INSERT INTO events(conversation_id,turn_id,plan_id,kind,payload,created_ms) VALUES(?1,NULL,?3,'comment',?2,CAST(unixepoch('subsec')*1000 AS INTEGER))",
            params![conversation, payload.to_string(), comment.plan_id],
        )?;
        tx.commit()?;
        Ok(comment)
    }

    /// Stores a comment on the plan as a draft: the user is still writing,
    /// and the orchestrator hears of it only when the user sends the review.
    pub fn add_draft_comment(&self, conversation: &str, comment: &Comment) -> Result<Comment> {
        let stored_text = self.content.encode(&comment.text)?;
        let mut db = self.db();
        let tx = db.transaction()?;
        let now = now_ms();
        tx.execute(
            "INSERT INTO comments(conversation_id,plan_id,target,label,text,status,message,
                                  created_ms,updated_ms)
             VALUES(?1,?2,?3,?4,?5,'draft',0,?6,?6)",
            params![
                conversation,
                comment.plan_id,
                comment.target,
                comment.label,
                stored_text,
                now
            ],
        )?;
        let comment = Comment {
            id: tx.last_insert_rowid() as u64,
            status: "draft".into(),
            message: 0,
            ..comment.clone()
        };
        tx.execute(
            "INSERT INTO events(conversation_id,turn_id,plan_id,kind,payload,created_ms) VALUES(?1,NULL,?3,'comment',?2,CAST(unixepoch('subsec')*1000 AS INTEGER))",
            params![conversation, self.comment_payload(&comment)?.to_string(), comment.plan_id],
        )?;
        tx.commit()?;
        Ok(comment)
    }

    /// Hands every draft comment on `plan_id` to the orchestrator as one
    /// message, so that they are answered together in one turn.
    pub fn send_draft_comments(&self, conversation: &str, plan_id: &str) -> Result<Vec<Comment>> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let drafts = self.comment_rows(
            &tx,
            "conversation_id=?1 AND status='draft' AND (plan_id=?2 OR plan_id IS NULL)",
            params![conversation, plan_id],
        )?;
        if drafts.is_empty() {
            return Err(Error::Invalid("there are no draft comments to send".into()));
        }
        let now = now_ms();
        let text = drafts
            .iter()
            .map(|c| {
                format!(
                    "{}{} on {}] {}",
                    crate::lean::COMMENT_PREFIX,
                    c.id,
                    c.label,
                    c.text
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        let text = self.content.encode(&text)?;
        tx.execute(
            "INSERT INTO messages(conversation_id,author,text,created_ms) VALUES(?1,'user',?2,?3)",
            params![conversation, text, now],
        )?;
        let message = tx.last_insert_rowid() as u64;
        tx.execute(
            "INSERT INTO events(conversation_id,turn_id,plan_id,kind,payload,created_ms)
             VALUES(?1,NULL,?3,'message',?2,CAST(unixepoch('subsec')*1000 AS INTEGER))",
            params![
                conversation,
                serde_json::json!({"seq": message, "author": "user", "text": text}).to_string(),
                plan_id
            ],
        )?;
        let mut sent = Vec::new();
        for mut comment in drafts {
            tx.execute(
                "UPDATE comments SET status='open', message=?1, updated_ms=?2 WHERE id=?3",
                params![message as i64, now, comment.id as i64],
            )?;
            comment.status = "open".into();
            comment.message = message;
            tx.execute(
                "INSERT INTO events(conversation_id,turn_id,plan_id,kind,payload,created_ms) VALUES(?1,NULL,?3,'comment',?2,CAST(unixepoch('subsec')*1000 AS INTEGER))",
                params![conversation, self.comment_payload(&comment)?.to_string(), comment.plan_id],
            )?;
            sent.push(comment);
        }
        tx.commit()?;
        Ok(sent)
    }

    pub fn set_comment_status(&self, conversation: &str, id: u64, status: &str) -> Result<Comment> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let changed = tx.execute(
            "UPDATE comments SET status=?1, updated_ms=?2 WHERE id=?3 AND conversation_id=?4",
            params![status, now_ms(), id as i64, conversation],
        )?;
        if changed == 0 {
            return Err(Error::Invalid(format!("no comment {id}")));
        }
        let mut comment = self
            .comment_rows(&tx, "id=?1", params![id as i64])?
            .pop()
            .ok_or_else(|| Error::Invalid(format!("no comment {id}")))?;
        if comment.message == 0 && status != "deleted" {
            // A draft has not been sent: it can be deleted, and only the
            // review sends it.
            return Err(Error::Invalid(
                "this comment is a draft: delete it, or send it with Review".into(),
            ));
        }
        if status == "open" {
            let text = format!(
                "{}{} on {}] {}",
                crate::lean::COMMENT_PREFIX,
                id,
                comment.label,
                comment.text
            );
            let text = self.content.encode(&text)?;
            tx.execute("INSERT INTO messages(conversation_id,author,text,created_ms) VALUES(?1,'user',?2,?3)", params![conversation, text, now_ms()])?;
            comment.message = tx.last_insert_rowid() as u64;
            tx.execute(
                "UPDATE comments SET message=?1 WHERE id=?2",
                params![comment.message as i64, id as i64],
            )?;
            tx.execute(
                "INSERT INTO events(conversation_id,plan_id,kind,payload,created_ms) VALUES(?1,?2,'message',?3,?4)",
                params![conversation, comment.plan_id, serde_json::json!({"seq": comment.message, "author": "user", "text": text}).to_string(), now_ms()],
            )?;
        }
        tx.execute(
            "INSERT INTO events(conversation_id,turn_id,plan_id,kind,payload,created_ms) VALUES(?1,NULL,?3,'comment',?2,CAST(unixepoch('subsec')*1000 AS INTEGER))",
            params![conversation, self.comment_payload(&comment)?.to_string(), comment.plan_id],
        )?;
        tx.commit()?;
        Ok(comment)
    }
}

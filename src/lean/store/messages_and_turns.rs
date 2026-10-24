use super::*;

impl Store {
    pub fn messages_after(&self, conversation: &str, after: u64) -> Result<Vec<Message>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT seq,author,text FROM messages WHERE conversation_id=?1 AND seq>?2 ORDER BY seq",
        )?;
        let rows = statement.query_map(params![conversation, after as i64], |row| {
            Ok(Message {
                seq: row.get::<_, i64>(0)? as u64,
                author: row.get(1)?,
                text: self.content.sql_text(row, 2)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// A bounded context window for a fresh orchestration session.
    pub fn recent_messages_through(
        &self,
        conversation: &str,
        through: u64,
    ) -> Result<Vec<Message>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT seq,author,text FROM messages WHERE conversation_id=?1 AND seq<=?2 ORDER BY seq DESC LIMIT 32",
        )?;
        let rows = statement.query_map(params![conversation, through as i64], |row| {
            Ok(Message {
                seq: row.get::<_, i64>(0)? as u64,
                author: row.get(1)?,
                text: self.content.sql_text(row, 2)?,
            })
        })?;
        let mut messages = rows.collect::<std::result::Result<Vec<_>, _>>()?;
        messages.reverse();
        Ok(messages)
    }

    /// One author's messages after `after`.
    pub fn messages_by(
        &self,
        conversation: &str,
        author: &str,
        after: u64,
    ) -> Result<Vec<Message>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT seq,author,text FROM messages
             WHERE conversation_id=?1 AND author=?2 AND seq>?3 ORDER BY seq",
        )?;
        let rows = statement.query_map(params![conversation, author, after as i64], |row| {
            Ok(Message {
                seq: row.get::<_, i64>(0)? as u64,
                author: row.get(1)?,
                text: self.content.sql_text(row, 2)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// How many messages `author` wrote since the user's last message.
    pub fn count_since_user(&self, conversation: &str, author: &str) -> Result<usize> {
        let count: i64 = self.db().query_row(
            "SELECT COUNT(*) FROM messages WHERE conversation_id=?1 AND author=?2
               AND seq > COALESCE((SELECT MAX(seq) FROM messages
                                   WHERE conversation_id=?1 AND author='user'), 0)",
            params![conversation, author],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// How many messages `author` wrote since the user last spoke or approved
    /// a requirements change; either one renews the automatic-retry budget.
    pub fn count_since_requirements_reset(
        &self,
        conversation: &str,
        author: &str,
    ) -> Result<usize> {
        let count: i64 = self.db().query_row(
            "SELECT COUNT(*) FROM messages WHERE conversation_id=?1 AND author=?2
               AND seq > COALESCE((SELECT MAX(seq) FROM messages
                                   WHERE conversation_id=?1
                                     AND author IN ('user','requirements-approval')), 0)",
            params![conversation, author],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    pub fn start_turn(
        &self,
        conversation: &str,
        role: &str,
        provider: &str,
        upto_message: u64,
        plan_id: Option<&str>,
    ) -> Result<Turn> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let resolved_plan: Option<String> = match plan_id {
            Some(pid) => Some(pid.to_string()),
            None => {
                if let Some(task_uuid) = crate::lean::task_role(role).map(|(id, _)| id) {
                    tx.query_row(
                        "SELECT plan_id FROM tasks WHERE id=?1",
                        [task_uuid],
                        |row| row.get::<_, Option<String>>(0),
                    )
                    .optional()?
                    .flatten()
                } else {
                    None
                }
            }
        };
        let turn = Turn {
            id: crate::id(),
            role: role.into(),
            status: TurnStatus::Running,
            session_id: None,
            error: None,
            upto_message,
            plan_id: resolved_plan.clone(),
        };
        tx.execute(
            "INSERT INTO turns(id,conversation_id,plan_id,role,provider,status,upto_message,started_ms)
             VALUES(?1,?2,?3,?4,?5,'running',?6,?7)",
            params![
                turn.id,
                conversation,
                resolved_plan,
                role,
                provider,
                upto_message as i64,
                now_ms()
            ],
        )?;
        let scope = crate::lean::task_role(role).map(|(task, _)| task);
        tx.execute(
            "INSERT INTO turn_scopes(turn_id,task_id,round)
            SELECT ?1,id,round FROM tasks WHERE id=?2 AND conversation_id=?3 AND plan_id=?4",
            params![turn.id, scope, conversation, resolved_plan],
        )?;
        tx.commit()?;
        Ok(turn)
    }

    pub fn set_session(&self, turn: &str, session: &str) -> Result<()> {
        crate::require(!session.is_empty(), "provider session id is empty")?;
        let mut db = self.db();
        let tx = db.transaction()?;
        let conflicting: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM run_contexts r JOIN turns t ON t.id=r.turn_id
                WHERE t.id=?1 AND r.backend!='cli' AND
                ((t.provider='codex' AND r.provider_thread_id IS NOT NULL AND r.provider_thread_id!=?2)
                 OR (t.provider!='codex' AND r.provider_session_id IS NOT NULL AND r.provider_session_id!=?2)))",
            params![turn, session], |row| row.get(0),
        )?;
        crate::require(
            !conflicting,
            "native run is already bound to another provider session",
        )?;
        tx.execute(
            "UPDATE turns SET session_id=?2 WHERE id=?1",
            params![turn, session],
        )?;
        // `codex exec` calls this a session, but its thread.started event
        // actually contains a thread ID. Retain the legacy resume column and
        // record the provider's precise identity separately.
        tx.execute(
            "UPDATE run_contexts SET
                provider_thread_id=CASE WHEN (SELECT provider FROM turns WHERE id=?1)='codex'
                    THEN ?2 ELSE provider_thread_id END,
                provider_session_id=CASE WHEN (SELECT provider FROM turns WHERE id=?1)!='codex'
                    THEN ?2 ELSE provider_session_id END
             WHERE turn_id=?1",
            params![turn, session],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn set_turn_pid(&self, turn: &str, pid: u32) -> Result<()> {
        self.db()
            .execute("UPDATE turns SET pid=?2 WHERE id=?1", params![turn, pid])?;
        Ok(())
    }

    /// The CLIs of turns still marked running: (process group, turn start,
    /// provider). Read before those turns become interrupted.
    pub fn running_processes(&self) -> Result<Vec<(u32, i64, String)>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT pid, started_ms, provider FROM turns
             WHERE status='running' AND pid IS NOT NULL",
        )?;
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn finish_turn(&self, turn: &str, status: TurnStatus, error: Option<&str>) -> Result<()> {
        self.db().execute(
            "UPDATE turns SET status=?2,error=?3,finished_ms=?4 WHERE id=?1",
            params![turn, status.as_str(), error, now_ms()],
        )?;
        Ok(())
    }

    /// The latest turn of `role`, which carries the session to resume and the
    /// last message it answered.
    pub fn last_turn(&self, conversation: &str, role: &str) -> Result<Option<Turn>> {
        self.last_turn_for_roles(conversation, role, role)
    }

    pub fn last_orchestration_turn(&self, conversation: &str) -> Result<Option<Turn>> {
        self.last_turn_for_roles(
            conversation,
            crate::lean::ORCHESTRATOR,
            crate::lean::COMMENT_ORCHESTRATOR,
        )
    }

    pub(super) fn last_turn_for_roles(
        &self,
        conversation: &str,
        role: &str,
        other: &str,
    ) -> Result<Option<Turn>> {
        self.db()
            .query_row(
                "SELECT id,role,status,session_id,error,upto_message,plan_id FROM turns
                 WHERE conversation_id=?1 AND role IN (?2,?3) ORDER BY started_ms DESC, rowid DESC LIMIT 1",
                params![conversation, role, other],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, Option<String>>(6)?,
                    ))
                },
            )
            .optional()?
            .map(|(id, role, status, session_id, error, upto, plan_id)| {
                Ok(Turn {
                    id,
                    role,
                    status: TurnStatus::parse(&status)?,
                    session_id,
                    error,
                    upto_message: upto as u64,
                    plan_id,
                })
            })
            .transpose()
    }
}

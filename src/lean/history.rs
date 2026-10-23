//! Durable round attribution and bounded history reads. Legacy turns remain
//! unassigned unless the runtime recorded their scope explicitly.
use super::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentMessage {
    pub conversation: String,
    pub plan: Option<String>,
    pub recipient: String,
    pub operation_id: String,
    pub expected_round: Option<u32>,
    pub expected_run: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContentOwner {
    Round {
        task: String,
        round: u32,
        field: String,
    },
    Mail {
        id: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HistoryScope {
    Round { task: String, round: u32 },
    Session { turn: String },
}

impl Store {
    pub(super) fn initialize_history(connection: &Connection) -> Result<()> {
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS turn_scopes(
                turn_id TEXT PRIMARY KEY REFERENCES turns(id),
                task_id TEXT REFERENCES tasks(id), round INTEGER);
             CREATE INDEX IF NOT EXISTS events_by_session ON events(conversation_id,turn_id,seq);
             CREATE INDEX IF NOT EXISTS events_by_mail_id
                ON events(conversation_id,json_extract(payload,'$.id'))
                WHERE kind='mail' AND turn_id IS NULL;
             CREATE INDEX IF NOT EXISTS turns_by_round ON turn_scopes(task_id,round,turn_id);
             CREATE TABLE IF NOT EXISTS mail_sources(mail_id INTEGER PRIMARY KEY REFERENCES mail(id),turn_id TEXT NOT NULL REFERENCES turns(id));
             CREATE TABLE IF NOT EXISTS mail_scopes(
                mail_id INTEGER PRIMARY KEY REFERENCES mail(id),task_id TEXT,round INTEGER,run TEXT);
             CREATE INDEX IF NOT EXISTS mail_by_round ON mail_scopes(task_id,round,mail_id);
             CREATE INDEX IF NOT EXISTS mail_by_source ON mail_sources(turn_id,mail_id);
             CREATE TABLE IF NOT EXISTS comment_scopes(
                comment_id INTEGER PRIMARY KEY REFERENCES comments(id), round INTEGER, run TEXT);
             CREATE TABLE IF NOT EXISTS agent_operations(
                conversation_id TEXT NOT NULL REFERENCES conversations(id), operation_id TEXT NOT NULL,
                signature TEXT NOT NULL, mail_id INTEGER NOT NULL REFERENCES mail(id),
                PRIMARY KEY(conversation_id,operation_id));
             CREATE TABLE IF NOT EXISTS task_rounds(
                task_id TEXT NOT NULL REFERENCES tasks(id), round INTEGER NOT NULL,
                status TEXT NOT NULL, content TEXT NOT NULL,
                PRIMARY KEY(task_id,round));",
        )?;
        Ok(())
    }

    pub(crate) fn agent_mail_user_message(
        &self,
        conversation: &str,
        mail: u64,
    ) -> Result<Option<u64>> {
        let mail = i64::try_from(mail)
            .map_err(|_| Error::Invalid("mail identifier exceeds SQLite range".into()))?;
        let message: Option<i64> = self
            .db()
            .query_row(
                "SELECT COALESCE(json_extract(e.payload,'$.user_message'),
                (SELECT json_extract(n.payload,'$.seq') FROM events n
                 WHERE n.seq=e.seq+1 AND n.conversation_id=e.conversation_id
                 AND n.kind='message' AND json_extract(n.payload,'$.author')='user'))
             FROM events e WHERE e.conversation_id=?1 AND e.turn_id IS NULL
             AND e.kind='mail' AND json_extract(e.payload,'$.sender')='user'
             AND json_extract(e.payload,'$.recipient')='orchestrator'
             AND json_extract(e.payload,'$.id')=?2 ORDER BY e.seq DESC LIMIT 1",
                params![conversation, mail],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        Ok(message.map(|seq| seq as u64))
    }

    /// Compare the expected scope and enqueue exactly once in one transaction.
    pub fn enqueue_agent_message(&self, request: &AgentMessage) -> Result<Value> {
        crate::require(
            uuid::Uuid::parse_str(&request.operation_id).is_ok(),
            "operation_id must be a UUID",
        )?;
        crate::require(!request.text.trim().is_empty(), "the message is empty")?;
        let signature = crate::hash(serde_json::to_vec(&(
            &request.conversation,
            &request.plan,
            &request.recipient,
            &request.text,
        ))?);
        let mut db = self.db();
        let tx = db.transaction()?;
        let existing: Option<(String,i64)> = tx.query_row("SELECT signature,mail_id FROM agent_operations WHERE conversation_id=?1 AND operation_id=?2",
            params![request.conversation,request.operation_id], |row| Ok((row.get(0)?,row.get(1)?))).optional()?;
        if let Some((old, mail)) = existing {
            crate::require(
                old == signature,
                "operation_id was already used for a different message",
            )?;
            let delivered: Option<String> = tx.query_row(
                "SELECT delivered_turn FROM mail WHERE id=?1",
                [mail],
                |row| row.get(0),
            )?;
            return Ok(
                serde_json::json!({"mail":mail,"state":if delivered.is_some() {"delivered"} else {"queued"},"delivered_turn":delivered}),
            );
        }
        let task = if let Some((role, id)) = request.recipient.split_once(':') {
            crate::require(
                matches!(role, "worker" | "verifier") && uuid::Uuid::parse_str(id).is_ok(),
                "recipient must use a task UUID and an explicit role",
            )?;
            let (conversation, plan, round, status): (String, String, u32, String) = tx.query_row(
                "SELECT conversation_id,plan_id,round,status FROM tasks WHERE id=?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )?;
            crate::require(
                status != "stale",
                "task inputs are stale; update the plan before sending work",
            )?;
            crate::require(
                conversation == request.conversation && request.plan.as_ref() == Some(&plan),
                "task is outside the selected conversation or plan",
            )?;
            crate::require(
                request.expected_round == Some(round),
                "stale round; select the current round before sending",
            )?;
            Some(id)
        } else {
            crate::require(
                matches!(
                    request.recipient.as_str(),
                    "orchestrator" | "auditor" | "plan-verifier"
                ),
                "unknown agent role",
            )?;
            let current: Option<String> = tx.query_row("SELECT id FROM turns WHERE conversation_id=?1 AND role=?2 ORDER BY started_ms DESC,rowid DESC LIMIT 1",
                params![request.conversation,request.recipient],|row| row.get(0)).optional()?;
            crate::require(
                current == request.expected_run,
                "stale run; select the current run before sending",
            )?;
            None
        };
        let now = now_ms();
        let stored_text = self.content.encode(&request.text)?;
        tx.execute("INSERT INTO mail(conversation_id,sender,recipient,kind,text,created_ms) VALUES(?1,'user',?2,'agent_work',?3,?4)",
            params![request.conversation,request.recipient,stored_text,now])?;
        let mail = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO mail_scopes VALUES(?1,?2,?3,?4)",
            params![mail, task, request.expected_round, request.expected_run],
        )?;
        tx.execute(
            "INSERT INTO agent_operations VALUES(?1,?2,?3,?4)",
            params![request.conversation, request.operation_id, signature, mail],
        )?;
        let user_message = if request.recipient == "orchestrator" {
            tx.execute("INSERT INTO messages(conversation_id,author,text,created_ms) VALUES(?1,'user',?2,?3)",params![request.conversation,stored_text,now])?;
            Some(tx.last_insert_rowid())
        } else {
            None
        };
        tx.execute("INSERT INTO events(conversation_id,plan_id,kind,payload,created_ms) VALUES(?1,?2,'mail',?3,?4)",
            params![request.conversation,request.plan,serde_json::json!({"id":mail,"sender":"user","recipient":request.recipient,"text":stored_text,"kind":"agent_work","state":"queued","created_ms":now,"user_message":user_message}).to_string(),now])?;
        if let Some(seq) = user_message {
            tx.execute("INSERT INTO events(conversation_id,kind,payload,created_ms) VALUES(?1,'message',?2,?3)",params![request.conversation,serde_json::json!({"seq":seq,"author":"user","text":stored_text}).to_string(),now])?;
        }
        if let Some(task) = task {
            if request.recipient.starts_with("worker:") {
                invalidate_dependents(
                    &tx,
                    task,
                    request.plan.as_deref().expect("task plan"),
                    &request.conversation,
                    &request.operation_id,
                    now,
                )?;
            }
            // Keep a running task serialized under its scheduler. A completed
            // task is queued; its saved CLI session is retained.
            tx.execute(
                "UPDATE tasks SET status='queued',verdict=NULL,feedback=NULL,
                base=CASE WHEN status='approved' THEN NULL ELSE base END,
                result=CASE WHEN status='approved' THEN NULL ELSE result END
                WHERE id=?1 AND status NOT IN ('working','verifying','queued','proposed','stale')",
                [task],
            )?;
        }
        tx.commit()?;
        Ok(serde_json::json!({"mail":mail,"state":"queued","recipient":request.recipient}))
    }

    pub fn invalidate_task_dependents(&self, conversation: &str, task: &Task) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        invalidate_dependents(
            &tx,
            &task.id,
            &task.plan_id,
            conversation,
            "worker-correction",
            now_ms(),
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn history_content(
        &self,
        conversation: &str,
        owner: &ContentOwner,
        offset: u64,
    ) -> Result<crate::lean::content::Chunk> {
        let reference = match owner {
            ContentOwner::Mail { id } => {
                let encoded: String = self.db().query_row(
                    "SELECT text FROM mail WHERE id=?1 AND conversation_id=?2",
                    params![*id as i64, conversation],
                    |r| r.get(0),
                )?;
                if let Some(reference) = encoded.strip_prefix("\0antislop-content:") {
                    serde_json::from_str(reference)?
                } else {
                    return legacy_chunk(&encoded, offset);
                }
            }
            ContentOwner::Round { task, round, field } => {
                crate::require(
                    matches!(field.as_str(), "report" | "verdict"),
                    "invalid round content field",
                )?;
                let encoded:String=self.db().query_row("SELECT r.content FROM task_rounds r JOIN tasks t ON t.id=r.task_id WHERE t.conversation_id=?1 AND t.id=?2 AND r.round=?3",params![conversation,task,round],|r|r.get(0))?;
                let record: Value =
                    serde_json::from_str(&self.content.read(&serde_json::from_str(&encoded)?)?)?;
                if let Some(reference) = record[field].get("session_content") {
                    serde_json::from_value(reference.clone())?
                } else {
                    let text = record[field]
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| record[field].to_string());
                    return legacy_chunk(&text, offset);
                }
            }
        };
        self.content.chunk(&reference, offset)
    }

    /// Message metadata and content references are paged independently of the
    /// event cursor. No first event page materializes the entire mailbox.
    pub fn history_messages(
        &self,
        conversation: &str,
        scope: &HistoryScope,
        after: u64,
        limit: usize,
    ) -> Result<Value> {
        let db = self.db();
        let (task, round, turn) = match scope {
            HistoryScope::Round { task, round } => {
                let owns: bool = db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM tasks WHERE id=?1 AND conversation_id=?2)",
                    params![task, conversation],
                    |r| r.get(0),
                )?;
                crate::require(owns, "task is outside this conversation")?;
                (Some(task.as_str()), Some(*round), None)
            }
            HistoryScope::Session { turn } => {
                let owns: bool = db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM turns WHERE id=?1 AND conversation_id=?2)",
                    params![turn, conversation],
                    |r| r.get(0),
                )?;
                crate::require(owns, "session is outside this conversation")?;
                (None, None, Some(turn.as_str()))
            }
        };
        let limit = limit.clamp(1, 500);
        let mut statement=db.prepare("SELECT m.id,m.sender,m.recipient,m.text,m.delivered_turn FROM mail m LEFT JOIN mail_scopes s ON s.mail_id=m.id
            WHERE m.conversation_id=?1 AND m.id>?5 AND (
                (?2 IS NOT NULL AND ((s.task_id=?2 AND s.round=?3 AND m.delivered_turn IS NULL AND NOT EXISTS(SELECT 1 FROM mail_sources source WHERE source.mail_id=m.id))
                    OR m.delivered_turn IN (SELECT turn_id FROM turn_scopes WHERE task_id=?2 AND round=?3)
                    OR m.id IN (SELECT mail_id FROM mail_sources WHERE turn_id IN (SELECT turn_id FROM turn_scopes WHERE task_id=?2 AND round=?3))))
                OR (?4 IS NOT NULL AND (m.delivered_turn=?4 OR m.id IN (SELECT mail_id FROM mail_sources WHERE turn_id=?4)))) ORDER BY m.id LIMIT ?6")?;
        let mut messages = Vec::new();
        let mut has_more = false;
        for row in statement.query_map(
            params![
                conversation,
                task,
                round,
                turn,
                after as i64,
                limit as i64 + 1
            ],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                ))
            },
        )? {
            let (id, sender, recipient, text, delivered) = row?;
            if messages.len() == limit {
                has_more = true;
                break;
            }
            let content = if let Some(reference) = text.strip_prefix("\0antislop-content:") {
                serde_json::json!({"session_content":serde_json::from_str::<Value>(reference)?})
            } else {
                serde_json::json!({"legacy":true,"bytes":text.len()})
            };
            messages.push(serde_json::json!({"id":id,"sender":sender,"recipient":recipient,"content":content,"delivered_turn":delivered,"state":if delivered.is_some(){"delivered"}else{"queued"}}));
        }
        let next_after = messages
            .last()
            .and_then(|m| m["id"].as_u64())
            .unwrap_or(after);
        Ok(serde_json::json!({"messages":messages,"next_after":next_after,"has_more":has_more}))
    }

    /// Bind queued direct mail to the turn that actually consumes it, before
    /// starting the CLI. Delivery is acknowledged separately after success.
    pub fn attribute_mail(&self, conversation: &str, ids: &[u64], turn: &str) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        for id in ids {
            tx.execute("INSERT INTO mail_scopes(mail_id,task_id,round,run)
                SELECT m.id,s.task_id,s.round,t.id FROM mail m JOIN turns t ON t.id=?3 AND t.conversation_id=m.conversation_id AND t.role=m.recipient
                LEFT JOIN turn_scopes s ON s.turn_id=t.id
                WHERE m.id=?1 AND m.conversation_id=?2 AND m.kind='agent_work' AND m.delivered_turn IS NULL
                ON CONFLICT(mail_id) DO UPDATE SET task_id=excluded.task_id,round=excluded.round,run=excluded.run",params![*id as i64,conversation,turn])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn assert_comment_run(
        &self,
        conversation: &str,
        plan: Option<&str>,
        target: &str,
        run: &str,
    ) -> Result<()> {
        let db = self.db();
        let (role, run_plan): (String, Option<String>) = db.query_row(
            "SELECT role,plan_id FROM turns WHERE id=?1 AND conversation_id=?2",
            params![run, conversation],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let canonical = if let Some((kind, id)) = target.split_once(':') {
            let task:Option<String>=db.query_row("SELECT id FROM tasks WHERE conversation_id=?1 AND plan_id=?2 AND (id=?3 OR json_extract(spec,'$.id')=?3)",params![conversation,plan,id],|r|r.get(0)).optional()?;
            task.map_or_else(|| target.to_owned(), |task| format!("{kind}:{task}"))
        } else {
            target.to_owned()
        };
        crate::require(role == canonical, "comment run belongs to another agent")?;
        crate::require(
            run_plan
                .as_deref()
                .is_none_or(|run_plan| Some(run_plan) == plan),
            "comment run belongs to another plan",
        )?;
        Ok(())
    }

    pub fn comment_scope(&self, conversation: &str, id: u64) -> Result<Value> {
        let scope: Option<(Option<u32>,Option<String>)> = self.db().query_row(
            "SELECT s.round,s.run FROM comment_scopes s JOIN comments c ON c.id=s.comment_id WHERE c.conversation_id=?1 AND c.id=?2",
            params![conversation,id as i64],|row| Ok((row.get(0)?,row.get(1)?))).optional()?;
        Ok(scope.map_or(
            serde_json::json!({"round":null,"run":null}),
            |(round, run)| serde_json::json!({"round":round,"run":run}),
        ))
    }

    pub fn task_rounds(&self, task: &str) -> Result<Vec<Value>> {
        let db = self.db();
        let mut statement =
            db.prepare("SELECT round,status FROM task_rounds WHERE task_id=?1 ORDER BY round")?;
        let mut rounds = statement
            .query_map([task], |row| {
                Ok(serde_json::json!({
                    "round": row.get::<_, u32>(0)?, "status": row.get::<_, String>(1)?
                }))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut historical=db.prepare("SELECT DISTINCT round FROM turn_scopes WHERE task_id=?1 AND round IS NOT NULL ORDER BY round")?;
        for round in historical.query_map([task], |r| r.get::<_, u32>(0))? {
            let round = round?;
            if !rounds
                .iter()
                .any(|value| value["round"].as_u64() == Some(u64::from(round)))
            {
                rounds.push(serde_json::json!({"round":round,"status":"history","history_unavailable":true}));
            }
        }
        rounds.sort_by_key(|value| value["round"].as_u64());
        let current: Option<(u32, String)> = db
            .query_row(
                "SELECT round,status FROM tasks WHERE id=?1",
                [task],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((round, status)) = current
            && !rounds
                .iter()
                .any(|value| value["round"].as_u64() == Some(u64::from(round)))
        {
            rounds.push(
                serde_json::json!({"round":round,"status":status,"history_unavailable":true}),
            );
            rounds.sort_by_key(|value| value["round"].as_u64());
        }
        Ok(rounds)
    }

    pub fn task_round(&self, task: &str, round: u32) -> Result<Value> {
        self.task_round_with_references(task, round, false)
    }

    pub fn task_round_with_references(
        &self,
        task: &str,
        round: u32,
        references: bool,
    ) -> Result<Value> {
        let encoded: Option<(String, String)> = self
            .db()
            .query_row(
                "SELECT status,content FROM task_rounds WHERE task_id=?1 AND round=?2",
                params![task, round],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let mut result: Value = if let Some((status, encoded)) = encoded {
            let reference = serde_json::from_str(&encoded)?;
            let mut record: Value = serde_json::from_str(&self.content.read(&reference)?)?;
            record["status"] = status.into();
            record
        } else {
            let current = self
                .task_by_id(task)?
                .ok_or_else(|| Error::Invalid("task does not exist".into()))?;
            let mut value = if current.round == round {
                serde_json::to_value(current)?
            } else {
                let assigned: bool = self.db().query_row(
                    "SELECT EXISTS(SELECT 1 FROM turn_scopes WHERE task_id=?1 AND round=?2)",
                    params![task, round],
                    |r| r.get(0),
                )?;
                crate::require(assigned, "round history is unavailable")?;
                serde_json::json!({"id":current.id,"plan_id":current.plan_id,"round":round,"status":"history","spec":{"id":current.spec["id"],"title":current.spec["title"]}})
            };
            value["history_unavailable"] = true.into();
            value
        };
        for field in ["report", "verdict"] {
            if !references && let Some(reference) = result[field].get("session_content") {
                let text = self
                    .content
                    .read(&serde_json::from_value(reference.clone())?)?;
                result[field] = if field == "verdict" {
                    serde_json::from_str(&text)?
                } else {
                    text.into()
                };
            }
        }
        result["checks"] = serde_json::to_value(self.checks(task, round)?)?;
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT t.id,t.role,t.status,(SELECT backend FROM run_contexts r WHERE r.turn_id=t.id),t.session_id FROM turns t JOIN turn_scopes s ON s.turn_id=t.id
            WHERE s.task_id=?1 AND s.round=?2 ORDER BY t.started_ms,t.rowid",
        )?;
        let turns = statement.query_map(params![task,round], |row| Ok(serde_json::json!({
            "turn_id": row.get::<_,String>(0)?, "role": row.get::<_,String>(1)?, "status": row.get::<_,String>(2)?, "backend":row.get::<_,Option<String>>(3)?, "session_id":row.get::<_,Option<String>>(4)?
        })))?.collect::<std::result::Result<Vec<_>,_>>()?;
        result["turns"] = turns.into();
        let mut native = db.prepare(
            "SELECT t.role,r.backend,COALESCE(r.provider_thread_id,r.provider_session_id,t.session_id) FROM tasks k JOIN turns t
            ON t.conversation_id=k.conversation_id AND t.role IN ('worker:'||k.id,'verifier:'||k.id)
            JOIN run_contexts r ON r.turn_id=t.id
            JOIN turn_scopes current_scope ON current_scope.turn_id=t.id AND current_scope.round=k.round
            WHERE k.id=?1 AND k.round=?2 AND t.id=(SELECT p.id FROM turns p
                WHERE p.conversation_id=t.conversation_id AND p.role=t.role
                ORDER BY p.started_ms DESC,p.rowid DESC LIMIT 1)",
        )?;
        result["native_sessions"]=native.query_map(params![task,round],|row|Ok(serde_json::json!({"role":row.get::<_,String>(0)?,"backend":row.get::<_,String>(1)?,"session_id":row.get::<_,Option<String>>(2)?})))?
            .collect::<std::result::Result<Vec<_>,_>>()?.into();
        if !references {
            let mut statement = db.prepare("SELECT m.id,m.sender,m.recipient,m.text,m.delivered_turn FROM mail m LEFT JOIN mail_scopes s ON s.mail_id=m.id
            WHERE m.conversation_id=(SELECT conversation_id FROM tasks WHERE id=?1)
            AND ((s.task_id=?1 AND s.round=?2 AND m.delivered_turn IS NULL
                AND NOT EXISTS(SELECT 1 FROM mail_sources source WHERE source.mail_id=m.id)) OR m.delivered_turn IN (SELECT turn_id FROM turn_scopes WHERE task_id=?1 AND round=?2)
                OR m.id IN (SELECT mail_id FROM mail_sources WHERE turn_id IN (SELECT turn_id FROM turn_scopes WHERE task_id=?1 AND round=?2))) ORDER BY m.id")?;
            result["messages"] = statement.query_map(params![task,round], |row| Ok(serde_json::json!({
            "id":row.get::<_,i64>(0)?,"sender":row.get::<_,String>(1)?,"recipient":row.get::<_,String>(2)?,"text":self.content.sql_text(row,3)?,"delivered_turn":row.get::<_,Option<String>>(4)?
        })))?.collect::<std::result::Result<Vec<_>,_>>()?.into();
        }
        Ok(result)
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
        let db = self.db();
        let owns: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM turns WHERE id=?1 AND conversation_id=?2)",
            params![turn, conversation],
            |row| row.get(0),
        )?;
        crate::require(owns, "session is outside this conversation")?;
        let limit = limit.clamp(1, 500);
        let mut statement = db.prepare("SELECT seq,kind,payload,created_ms FROM events WHERE conversation_id=?1 AND turn_id=?2 AND seq>?3 ORDER BY seq LIMIT ?4")?;
        let rows = statement.query_map(
            params![conversation, turn, after as i64, limit as i64 + 1],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )?;
        let mut events = Vec::with_capacity(limit);
        let mut has_more = false;
        for row in rows {
            let (seq, kind, payload, created_ms) = row?;
            if events.len() == limit {
                has_more = true;
                break;
            }
            events.push(serde_json::json!({"seq": seq,"kind":kind,"payload":serde_json::from_str::<Value>(&payload)?,"created_ms":created_ms}));
        }
        let next_after = events
            .last()
            .and_then(|event| event["seq"].as_u64())
            .unwrap_or(after);
        let messages = if after == 0 && include_messages {
            let mut statement = db.prepare("SELECT m.id,m.sender,m.recipient,m.text,m.delivered_turn FROM mail m WHERE m.conversation_id=?1
                AND (m.delivered_turn=?2 OR m.id IN (SELECT mail_id FROM mail_sources WHERE turn_id=?2)) ORDER BY m.id")?;
            statement.query_map(params![conversation,turn],|row|Ok(serde_json::json!({"id":row.get::<_,i64>(0)?,"sender":row.get::<_,String>(1)?,"recipient":row.get::<_,String>(2)?,"text":self.content.sql_text(row,3)?,"delivered_turn":row.get::<_,Option<String>>(4)?})))?.collect::<std::result::Result<Vec<_>,_>>()?
        } else {
            Vec::new()
        };
        Ok(
            serde_json::json!({"events":events,"messages":messages,"next_after":next_after,"has_more":has_more}),
        )
    }
}

fn invalidate_dependents(
    tx: &rusqlite::Transaction<'_>,
    task: &str,
    plan: &str,
    conversation: &str,
    operation: &str,
    now: i64,
) -> Result<()> {
    let mut statement = tx.prepare("WITH RECURSIVE affected(id,local_id) AS (
                SELECT id,json_extract(spec,'$.id') FROM tasks WHERE id=?1
                UNION SELECT t.id,json_extract(t.spec,'$.id') FROM tasks t,affected a,json_each(t.spec,'$.depends_on') d
                WHERE t.plan_id=?2 AND (d.value=a.id OR d.value=a.local_id))
                SELECT id FROM affected WHERE id<>?1")?;
    let affected = statement
        .query_map(params![task, plan], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    drop(statement);
    for dependent in affected {
        tx.execute(
            "UPDATE task_inputs SET generation=generation+1 WHERE task_id=?1",
            [&dependent],
        )?;
        tx.execute(
            "UPDATE tasks SET status='queued',verdict=NULL,feedback=NULL,
                    base=CASE WHEN status='approved' THEN NULL ELSE base END,
                    result=CASE WHEN status='approved' THEN NULL ELSE result END
                    WHERE id=?1 AND status NOT IN ('proposed','stale')",
            [&dependent],
        )?;
        tx.execute("INSERT INTO events(conversation_id,plan_id,kind,payload,created_ms) VALUES(?1,?2,'task_inputs_invalidated',?3,?4)",
                    params![conversation,plan,serde_json::json!({"task":dependent,"source":task,"operation_id":operation}).to_string(),now])?;
    }
    Ok(())
}

fn legacy_chunk(text: &str, offset: u64) -> Result<crate::lean::content::Chunk> {
    let start = usize::try_from(offset)
        .map_err(|_| Error::Invalid("content offset is out of range".into()))?;
    crate::require(
        start <= text.len() && text.is_char_boundary(start),
        "invalid content offset",
    )?;
    let mut end = start
        .saturating_add(crate::lean::content::CHUNK_BYTES)
        .min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    Ok(crate::lean::content::Chunk {
        text: text[start..end].into(),
        next_offset: end as u64,
        done: end == text.len(),
    })
}

impl Store {
    /// Read compact round snapshots without loading transcripts, mailbox contents
    /// or historical reports. Only selected handoff fields are resolved later.
    pub(in crate::lean) fn handoff_rounds(
        &self,
        task: &str,
        through: u32,
    ) -> Result<Vec<(u32, Value)>> {
        let rows = {
            let db = self.db();
            let mut statement = db.prepare(
                "SELECT r.round,r.content,
                    EXISTS(SELECT 1 FROM turn_scopes s JOIN turns t ON t.id=s.turn_id WHERE s.task_id=r.task_id AND s.round=r.round AND t.role='worker:'||r.task_id),
                    EXISTS(SELECT 1 FROM turn_scopes s JOIN turns t ON t.id=s.turn_id WHERE s.task_id=r.task_id AND s.round=r.round AND t.role='worker:'||r.task_id AND t.status='done')
                 FROM task_rounds r WHERE r.task_id=?1 AND r.round>0 AND r.round<=?2 ORDER BY r.round DESC",
            )?;
            statement
                .query_map(params![task, through], |row| {
                    Ok((
                        row.get::<_, u32>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, bool>(2)?,
                        row.get::<_, bool>(3)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        rows.into_iter()
            .map(|(round, encoded, started, completed)| {
                let reference = serde_json::from_str(&encoded)?;
                let snapshot: Value = serde_json::from_str(&self.content.read(&reference)?)?;
                let value = serde_json::json!({
                    "base":snapshot["base"], "result":snapshot["result"],
                    "report":snapshot["report"], "verdict":snapshot["verdict"],
                    "worker_started":started, "worker_completed":completed,
                });
                Ok((round, value))
            })
            .collect()
    }

    /// agent_final is durable before the scheduler updates its round snapshot.
    /// Recover that window without consulting tool or conversation history.
    pub(in crate::lean) fn completed_worker_report(
        &self,
        task: &str,
        round: u32,
    ) -> Result<Option<(u32, String)>> {
        let encoded: Option<(u32, String)> = self
            .db()
            .query_row(
                "SELECT s.round,e.payload FROM events e JOIN turns t ON t.id=e.turn_id
             JOIN turn_scopes s ON s.turn_id=t.id
             WHERE s.task_id=?1 AND s.round<=?2 AND t.role='worker:'||?1 AND e.kind='agent_final'
             AND NOT EXISTS(SELECT 1 FROM events start WHERE start.turn_id=t.id AND start.kind='turn_started' AND json_extract(start.payload,'$.mail_only')=1)
             ORDER BY e.seq DESC LIMIT 1",
                params![task, round],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        encoded
            .map(|(source, encoded)| {
                let payload = self.hydrate_event_payload("agent_final", &encoded)?;
                payload["text"]
                    .as_str()
                    .map(|text| (source, text.to_owned()))
                    .ok_or_else(|| Error::Invalid("worker final answer has no text".into()))
            })
            .transpose()
    }

    pub(in crate::lean) fn handoff_field(&self, value: &Value, json: bool) -> Result<Value> {
        let Some(reference) = value.get("session_content") else {
            return Ok(value.clone());
        };
        let text = self
            .content
            .read(&serde_json::from_value(reference.clone())?)?;
        Ok(if json {
            serde_json::from_str(&text)?
        } else {
            text.into()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orchestrator_mail_keeps_its_user_message_source_without_schema_changes() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(&directory.path().join("core.db")).unwrap();
        let conversation = store.create_conversation("project").unwrap();
        let other = store.create_conversation("other").unwrap();
        let mut pairs = vec![];
        for _ in 0..2 {
            let receipt = store
                .enqueue_agent_message(&AgentMessage {
                    conversation: conversation.id.clone(),
                    plan: None,
                    recipient: "orchestrator".into(),
                    operation_id: crate::id(),
                    expected_round: None,
                    expected_run: None,
                    text: "same correction 👩‍💻".into(),
                })
                .unwrap();
            let mail = receipt["mail"].as_u64().unwrap();
            let message = store
                .messages_by(&conversation.id, "user", 0)
                .unwrap()
                .last()
                .unwrap()
                .seq;
            assert_eq!(
                store
                    .agent_mail_user_message(&conversation.id, mail)
                    .unwrap(),
                Some(message)
            );
            assert_eq!(
                store.agent_mail_user_message(&other.id, mail).unwrap(),
                None
            );
            pairs.push((mail, message));
        }
        assert_ne!(pairs[0].1, pairs[1].1);
        // Schema 11 records made before source metadata was added retain
        // their atomic mail/message event pair and need no migration.
        store.db().execute("UPDATE events SET payload=json_remove(payload,'$.user_message') WHERE conversation_id=?1 AND kind='mail'",[&conversation.id]).unwrap();
        for (mail, message) in pairs {
            assert_eq!(
                store
                    .agent_mail_user_message(&conversation.id, mail)
                    .unwrap(),
                Some(message)
            );
        }
        let schema: u32 = store
            .db()
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(schema, 11);
    }

    #[test]
    fn session_events_page_references_and_enforce_ownership() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("core.db");
        let store = Store::open(&path).unwrap();
        let conversation = store.create_conversation("project").unwrap();
        let other = store.create_conversation("other").unwrap();
        let turn = store
            .start_turn(&conversation.id, "orchestrator", "codex", 0, None)
            .unwrap();
        let text = "é👩‍💻".repeat(600_000);
        let seq = store
            .append_event(
                &conversation.id,
                Some(&turn.id),
                None,
                "agent",
                &serde_json::json!({"type":"prompt","text":text}),
            )
            .unwrap();
        store
            .append_event(
                &conversation.id,
                Some(&turn.id),
                None,
                "agent_final",
                &serde_json::json!({"text":text}),
            )
            .unwrap();
        let page = store
            .session_events(&conversation.id, &turn.id, 0, 1)
            .unwrap();
        assert_eq!(page["has_more"], true);
        assert_eq!(page["events"][0]["seq"], seq);
        assert!(page.to_string().len() < 1000);
        assert!(store.session_events(&other.id, &turn.id, 0, 1).is_err());
        assert!(store.session_content(&other.id, seq, "text", 0).is_err());
        let mut restored = String::new();
        let mut offset = 0;
        loop {
            let chunk = store
                .session_content(&conversation.id, seq, "text", offset)
                .unwrap();
            restored.push_str(&chunk.text);
            if chunk.done {
                break;
            }
            offset = chunk.next_offset;
        }
        assert_eq!(restored, text);
        drop(store);
        let reopened = Store::open(&path).unwrap();
        let events = reopened.events_since(&conversation.id, seq - 1, 2).unwrap();
        assert_eq!(events[0].payload["text"], text);
        assert_eq!(events[1].payload["text"], text);
    }
}

#[cfg(test)]
mod round_tests {
    use super::*;

    #[test]
    fn rounds_keep_results_turn_scope_and_idempotent_messages() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("core.db");
        let store = Store::open(&path).unwrap();
        let conversation = store.create_conversation("project").unwrap();
        let mut task = store
            .add_plan(
                &conversation.id,
                &[serde_json::json!({"id":"t1","title":"task"})],
            )
            .unwrap()
            .remove(0);
        task.round = 1;
        task.status = "working".into();
        task.report = Some("report one".into());
        store.update_task(&task).unwrap();
        let role = format!("worker:{}", task.id);
        let turn = store
            .start_turn(&conversation.id, &role, "codex", 0, Some(&task.plan_id))
            .unwrap();
        task.status = "approved".into();
        task.base = Some("base-one".into());
        task.result = Some("result-one".into());
        store.update_task(&task).unwrap();
        let request = AgentMessage {
            conversation: conversation.id.clone(),
            plan: Some(task.plan_id.clone()),
            recipient: role,
            operation_id: uuid::Uuid::new_v4().to_string(),
            expected_round: Some(1),
            expected_run: None,
            text: "fix 👩‍💻".into(),
        };
        let receipt = store.enqueue_agent_message(&request).unwrap();
        assert_eq!(
            store.enqueue_agent_message(&request).unwrap()["mail"],
            receipt["mail"]
        );
        assert_eq!(
            store
                .pending_mail(&conversation.id, &request.recipient)
                .unwrap()
                .len(),
            1
        );
        let queued = store.task_by_id(&task.id).unwrap().unwrap();
        assert_eq!(queued.status, "queued");
        assert!(queued.result.is_none());
        let old = store.task_round(&task.id, 1).unwrap();
        assert_eq!(old["status"], "approved");
        assert_eq!(old["result"], "result-one");
        assert_eq!(old["turns"][0]["turn_id"], turn.id);
        let mut conflicting = request.clone();
        conflicting.text = "different".into();
        assert!(store.enqueue_agent_message(&conflicting).is_err());
        conflicting.operation_id = uuid::Uuid::new_v4().to_string();
        conflicting.expected_round = Some(0);
        assert!(store.enqueue_agent_message(&conflicting).is_err());
        task.round = 2;
        task.report = Some("report two".into());
        store.update_task(&task).unwrap();
        assert_eq!(store.task_rounds(&task.id).unwrap().len(), 2);
        assert_eq!(
            store.task_round(&task.id, 1).unwrap()["report"],
            "report one"
        );
        let new_turn = store
            .start_turn(
                &conversation.id,
                &request.recipient,
                "codex",
                0,
                Some(&task.plan_id),
            )
            .unwrap();
        store
            .mark_delivered(
                &conversation.id,
                &[receipt["mail"].as_u64().unwrap()],
                &new_turn.id,
            )
            .unwrap();
        assert!(
            store.task_round(&task.id, 1).unwrap()["messages"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            store.task_round(&task.id, 2).unwrap()["messages"][0]["id"],
            receipt["mail"]
        );
        let page = store
            .history_messages(
                &conversation.id,
                &HistoryScope::Round {
                    task: task.id.clone(),
                    round: 2,
                },
                0,
                1,
            )
            .unwrap();
        assert_eq!(page["messages"][0]["id"], receipt["mail"]);
        assert!(page["messages"][0]["content"]["session_content"].is_object());
        let foreign = store.create_conversation("foreign").unwrap();
        assert!(
            store
                .history_content(
                    &foreign.id,
                    &ContentOwner::Mail {
                        id: receipt["mail"].as_u64().unwrap()
                    },
                    0
                )
                .is_err()
        );
        assert!(
            store
                .history_messages(
                    &foreign.id,
                    &HistoryScope::Round {
                        task: task.id.clone(),
                        round: 2
                    },
                    0,
                    1
                )
                .is_err()
        );
        let refs = store.task_round_with_references(&task.id, 2, true).unwrap();
        assert!(refs["report"]["session_content"].is_object());
        assert_eq!(
            store
                .history_content(
                    &conversation.id,
                    &ContentOwner::Round {
                        task: task.id.clone(),
                        round: 2,
                        field: "report".into()
                    },
                    0
                )
                .unwrap()
                .text,
            "report two"
        );
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store.task_round(&task.id, 2).unwrap()["report"],
            "report two"
        );
        assert_eq!(
            store.enqueue_agent_message(&request).unwrap()["mail"],
            receipt["mail"]
        );
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;

    #[test]
    fn failed_round_attribution_rolls_back_the_turn() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("core.db")).unwrap();
        let conversation = store.create_conversation("project").unwrap();
        let task = store
            .add_plan(
                &conversation.id,
                &[serde_json::json!({"id":"t1","title":"task"})],
            )
            .unwrap()
            .remove(0);
        store.db().execute_batch("CREATE TRIGGER reject_scope BEFORE INSERT ON turn_scopes BEGIN SELECT RAISE(ABORT,'scope failure'); END;").unwrap();
        assert!(
            store
                .start_turn(
                    &conversation.id,
                    &format!("worker:{}", task.id),
                    "codex",
                    0,
                    Some(&task.plan_id)
                )
                .is_err()
        );
        let count: i64 = store
            .db()
            .query_row("SELECT count(*) FROM turns", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn restart_does_not_infer_missing_round_attribution() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("core.db");
        let store = Store::open(&path).unwrap();
        let conversation = store.create_conversation("project").unwrap();
        let mut task = store
            .add_plan(
                &conversation.id,
                &[serde_json::json!({"id":"t1","title":"task"})],
            )
            .unwrap()
            .remove(0);
        task.round = 7;
        task.status = "working".into();
        store.update_task(&task).unwrap();
        store
            .append_event(
                &conversation.id,
                None,
                Some(&task.plan_id),
                "task",
                &serde_json::json!({"task":task.id,"round":7,"status":"working"}),
            )
            .unwrap();
        let role = format!("worker:{}", task.id);
        let known = store
            .start_turn(&conversation.id, &role, "codex", 0, Some(&task.plan_id))
            .unwrap();
        store
            .append_event(
                &conversation.id,
                Some(&known.id),
                Some(&task.plan_id),
                "turn_started",
                &serde_json::json!({"role":role}),
            )
            .unwrap();
        store
            .append_event(
                &conversation.id,
                None,
                Some(&task.plan_id),
                "task",
                &serde_json::json!({"task":task.id,"round":7,"status":"approved"}),
            )
            .unwrap();
        let unknown = store
            .start_turn(&conversation.id, &role, "codex", 0, Some(&task.plan_id))
            .unwrap();
        store
            .append_event(
                &conversation.id,
                Some(&unknown.id),
                Some(&task.plan_id),
                "turn_started",
                &serde_json::json!({"role":role}),
            )
            .unwrap();
        task.round = 8;
        store.update_task(&task).unwrap();
        store
            .db()
            .execute_batch("DELETE FROM turn_scopes; DELETE FROM task_rounds WHERE round=7;")
            .unwrap();
        drop(store);
        let store = Store::open(&path).unwrap();
        assert!(store.task_round(&task.id, 7).is_err());
        let assigned: bool = store
            .db()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM turn_scopes WHERE turn_id=?1)",
                [unknown.id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!assigned);
    }
}

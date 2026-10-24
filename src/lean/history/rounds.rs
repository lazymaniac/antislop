use super::*;

impl Store {
    pub fn task_rounds(&self, task: &str) -> Result<Vec<Value>> {
        let db = self.db();
        let mut rounds = stored_rounds(&db, task)?;
        add_historical_rounds(&db, task, &mut rounds)?;
        add_current_round(&db, task, &mut rounds)?;
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
        let mut result = self.round_record_or_current(task, round)?;
        self.hydrate_round_fields(&mut result, references)?;
        self.attach_round_execution(task, round, references, &mut result)?;
        Ok(result)
    }

    fn round_record_or_current(&self, task: &str, round: u32) -> Result<Value> {
        let encoded: Option<(String, String)> = self
            .db()
            .query_row(
                "SELECT status,content FROM task_rounds WHERE task_id=?1 AND round=?2",
                params![task, round],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        match encoded {
            Some((status, encoded)) => self.decode_round_record(&status, &encoded),
            None => self.current_round_record(task, round),
        }
    }

    fn decode_round_record(&self, status: &str, encoded: &str) -> Result<Value> {
        let reference = serde_json::from_str(encoded)?;
        let mut record: Value = serde_json::from_str(&self.content.read(&reference)?)?;
        record["status"] = status.into();
        Ok(record)
    }

    fn current_round_record(&self, task: &str, round: u32) -> Result<Value> {
        let current = self
            .task_by_id(task)?
            .ok_or_else(|| Error::Invalid("task does not exist".into()))?;
        let mut value = if current.round == round {
            serde_json::to_value(current)?
        } else {
            self.unavailable_round_record(&current, round)?
        };
        value["history_unavailable"] = true.into();
        Ok(value)
    }

    fn unavailable_round_record(&self, current: &Task, round: u32) -> Result<Value> {
        let assigned: bool = self.db().query_row(
            "SELECT EXISTS(SELECT 1 FROM turn_scopes WHERE task_id=?1 AND round=?2)",
            params![current.id, round],
            |r| r.get(0),
        )?;
        crate::require(assigned, "round history is unavailable")?;
        Ok(
            serde_json::json!({"id":current.id,"plan_id":current.plan_id,"round":round,"status":"history","spec":{"id":current.spec["id"],"title":current.spec["title"]}}),
        )
    }

    fn hydrate_round_fields(&self, result: &mut Value, references: bool) -> Result<()> {
        if references {
            return Ok(());
        }
        for field in ["report", "verdict"] {
            if let Some(reference) = result[field].get("session_content") {
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
        Ok(())
    }

    fn attach_round_execution(
        &self,
        task: &str,
        round: u32,
        references: bool,
        result: &mut Value,
    ) -> Result<()> {
        result["checks"] = serde_json::to_value(self.checks(task, round)?)?;
        let db = self.db();
        result["turns"] = load_round_turns(&db, task, round)?.into();
        result["native_sessions"] = load_native_sessions(&db, task, round)?.into();
        drop(db);
        if !references {
            result["messages"] = self.load_round_messages(task, round)?.into();
        }
        Ok(())
    }

    fn load_round_messages(&self, task: &str, round: u32) -> Result<Vec<Value>> {
        let db = self.db();
        let mut statement = db.prepare("SELECT m.id,m.sender,m.recipient,m.text,m.delivered_turn FROM mail m LEFT JOIN mail_scopes s ON s.mail_id=m.id
            WHERE m.conversation_id=(SELECT conversation_id FROM tasks WHERE id=?1)
            AND ((s.task_id=?1 AND s.round=?2 AND m.delivered_turn IS NULL
                AND NOT EXISTS(SELECT 1 FROM mail_sources source WHERE source.mail_id=m.id)) OR m.delivered_turn IN (SELECT turn_id FROM turn_scopes WHERE task_id=?1 AND round=?2)
                OR m.id IN (SELECT mail_id FROM mail_sources WHERE turn_id IN (SELECT turn_id FROM turn_scopes WHERE task_id=?1 AND round=?2))) ORDER BY m.id")?;
        statement.query_map(params![task, round], |row| Ok(serde_json::json!({
            "id":row.get::<_,i64>(0)?,"sender":row.get::<_,String>(1)?,"recipient":row.get::<_,String>(2)?,"text":self.content.sql_text(row,3)?,"delivered_turn":row.get::<_,Option<String>>(4)?
        })))?.collect::<std::result::Result<Vec<_>,_>>().map_err(Into::into)
    }

    /// Read compact round snapshots without loading transcripts, mailbox contents
    /// or historical reports. Only selected handoff fields are resolved later.
    pub(in crate::lean) fn handoff_rounds(
        &self,
        task: &str,
        through: u32,
    ) -> Result<Vec<(u32, Value)>> {
        let rows = load_handoff_round_rows(&self.db(), task, through)?;
        rows.into_iter()
            .map(|row| self.shape_handoff_round(row))
            .collect()
    }

    fn shape_handoff_round(
        &self,
        (round, encoded, started, completed): HandoffRound,
    ) -> Result<(u32, Value)> {
        let reference = serde_json::from_str(&encoded)?;
        let snapshot: Value = serde_json::from_str(&self.content.read(&reference)?)?;
        Ok((
            round,
            serde_json::json!({
                "base":snapshot["base"], "result":snapshot["result"],
                "report":snapshot["report"], "verdict":snapshot["verdict"],
                "worker_started":started, "worker_completed":completed,
            }),
        ))
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

fn stored_rounds(db: &rusqlite::Connection, task: &str) -> Result<Vec<Value>> {
    let mut statement =
        db.prepare("SELECT round,status FROM task_rounds WHERE task_id=?1 ORDER BY round")?;
    statement.query_map([task], |row| {
        Ok(serde_json::json!({"round": row.get::<_, u32>(0)?, "status": row.get::<_, String>(1)?}))
    })?.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
}

fn add_historical_rounds(
    db: &rusqlite::Connection,
    task: &str,
    rounds: &mut Vec<Value>,
) -> Result<()> {
    let mut statement = db.prepare(
        "SELECT DISTINCT round FROM turn_scopes WHERE task_id=?1 AND round IS NOT NULL ORDER BY round",
    )?;
    for round in statement.query_map([task], |row| row.get::<_, u32>(0))? {
        let round = round?;
        if !round_exists(rounds, round) {
            rounds.push(
                serde_json::json!({"round":round,"status":"history","history_unavailable":true}),
            );
        }
    }
    rounds.sort_by_key(|value| value["round"].as_u64());
    Ok(())
}

fn add_current_round(db: &rusqlite::Connection, task: &str, rounds: &mut Vec<Value>) -> Result<()> {
    let current: Option<(u32, String)> = db
        .query_row(
            "SELECT round,status FROM tasks WHERE id=?1",
            [task],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((round, status)) = current
        && !round_exists(rounds, round)
    {
        rounds.push(serde_json::json!({"round":round,"status":status,"history_unavailable":true}));
        rounds.sort_by_key(|value| value["round"].as_u64());
    }
    Ok(())
}

fn round_exists(rounds: &[Value], round: u32) -> bool {
    rounds
        .iter()
        .any(|value| value["round"].as_u64() == Some(u64::from(round)))
}

type HandoffRound = (u32, String, bool, bool);

fn load_round_turns(db: &rusqlite::Connection, task: &str, round: u32) -> Result<Vec<Value>> {
    let mut statement = db.prepare(
        "SELECT t.id,t.role,t.status,(SELECT backend FROM run_contexts r WHERE r.turn_id=t.id),t.session_id FROM turns t JOIN turn_scopes s ON s.turn_id=t.id WHERE s.task_id=?1 AND s.round=?2 ORDER BY t.started_ms,t.rowid",
    )?;
    statement.query_map(params![task, round], |row| Ok(serde_json::json!({
        "turn_id":row.get::<_,String>(0)?,"role":row.get::<_,String>(1)?,"status":row.get::<_,String>(2)?,"backend":row.get::<_,Option<String>>(3)?,"session_id":row.get::<_,Option<String>>(4)?
    })))?.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
}

fn load_native_sessions(db: &rusqlite::Connection, task: &str, round: u32) -> Result<Vec<Value>> {
    let mut statement = db.prepare(
        "SELECT t.role,r.backend,COALESCE(r.provider_thread_id,r.provider_session_id,t.session_id) FROM tasks k JOIN turns t ON t.conversation_id=k.conversation_id AND t.role IN ('worker:'||k.id,'verifier:'||k.id) JOIN run_contexts r ON r.turn_id=t.id JOIN turn_scopes current_scope ON current_scope.turn_id=t.id AND current_scope.round=k.round WHERE k.id=?1 AND k.round=?2 AND t.id=(SELECT p.id FROM turns p WHERE p.conversation_id=t.conversation_id AND p.role=t.role ORDER BY p.started_ms DESC,p.rowid DESC LIMIT 1)",
    )?;
    statement.query_map(params![task, round], |row| Ok(serde_json::json!({
        "role":row.get::<_,String>(0)?,"backend":row.get::<_,String>(1)?,"session_id":row.get::<_,Option<String>>(2)?
    })))?.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
}

fn load_handoff_round_rows(
    db: &rusqlite::Connection,
    task: &str,
    through: u32,
) -> Result<Vec<HandoffRound>> {
    let mut statement = db.prepare(
        "SELECT r.round,r.content,EXISTS(SELECT 1 FROM turn_scopes s JOIN turns t ON t.id=s.turn_id WHERE s.task_id=r.task_id AND s.round=r.round AND t.role='worker:'||r.task_id),EXISTS(SELECT 1 FROM turn_scopes s JOIN turns t ON t.id=s.turn_id WHERE s.task_id=r.task_id AND s.round=r.round AND t.role='worker:'||r.task_id AND t.status='done') FROM task_rounds r WHERE r.task_id=?1 AND r.round>0 AND r.round<=?2 ORDER BY r.round DESC",
    )?;
    statement
        .query_map(params![task, through], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

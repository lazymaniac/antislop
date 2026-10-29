use super::*;

#[path = "mail/agent.rs"]
mod agent;

impl Store {
    /// Compare the expected scope and enqueue exactly once in one transaction.
    pub fn enqueue_agent_message(&self, request: &AgentMessage) -> Result<Value> {
        let signature = agent::agent_message_signature(request)?;
        let mut db = self.db();
        let tx = db.transaction()?;
        if let Some(existing) = agent::existing_agent_message(&tx, request, &signature)? {
            return Ok(existing);
        }
        let task = agent::validate_agent_recipient(&tx, request)?;
        let now = now_ms();
        let stored_text = self.content.encode(&request.text)?;
        let mail =
            agent::insert_agent_mail(&tx, request, &signature, task.as_deref(), &stored_text, now)?;
        agent::update_agent_task(&tx, request, task.as_deref(), now)?;
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
                SELECT id,json_extract(spec,'$.id') FROM tasks WHERE id=?1 AND status<>'superseded'
                UNION SELECT t.id,json_extract(t.spec,'$.id') FROM tasks t,affected a,json_each(t.spec,'$.depends_on') d
                WHERE t.plan_id=?2 AND t.status<>'superseded' AND (d.value=a.id OR d.value=a.local_id))
                SELECT id FROM affected WHERE id<>?1")?;
    let affected = statement
        .query_map(params![task, plan], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    drop(statement);
    for dependent in affected {
        invalidate_dependent(tx, &dependent, task, plan, conversation, operation, now)?;
    }
    Ok(())
}

fn invalidate_dependent(
    tx: &rusqlite::Transaction<'_>,
    dependent: &str,
    source: &str,
    plan: &str,
    conversation: &str,
    operation: &str,
    now: i64,
) -> Result<()> {
    // A task that never ran has nothing from an earlier input to discard.
    let untouched: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM tasks WHERE id=?1 AND status='queued' AND round=0 AND report IS NULL AND result IS NULL)",
        [dependent],
        |row| row.get(0),
    )?;
    if untouched {
        return Ok(());
    }
    invalidate_task_input(tx, dependent)?;
    record_invalidation(tx, dependent, source, plan, conversation, operation, now)
}

fn invalidate_task_input(tx: &rusqlite::Transaction<'_>, dependent: &str) -> Result<()> {
    tx.execute(
        "UPDATE task_inputs SET generation=generation+1 WHERE task_id=?1",
        [dependent],
    )?;
    tx.execute(
        "UPDATE tasks SET status='queued',verdict=NULL,feedback=NULL,
        base=NULL,result=NULL
        WHERE id=?1 AND status NOT IN ('proposed','stale','superseded')",
        [dependent],
    )?;
    Ok(())
}

fn record_invalidation(
    tx: &rusqlite::Transaction<'_>,
    dependent: &str,
    source: &str,
    plan: &str,
    conversation: &str,
    operation: &str,
    now: i64,
) -> Result<()> {
    tx.execute("INSERT INTO events(conversation_id,plan_id,kind,payload,created_ms) VALUES(?1,?2,'task_inputs_invalidated',?3,?4)",
        params![conversation, plan, serde_json::json!({"task":dependent,"source":source,"operation_id":operation}).to_string(), now])?;
    Ok(())
}

use super::*;

type RunningTurn = (String, String, Option<String>, String);

pub(super) fn running_orchestrators(tx: &rusqlite::Transaction<'_>) -> Result<Vec<String>> {
    let mut statement = tx.prepare(
        "SELECT DISTINCT conversation_id FROM turns
         WHERE status='running' AND role IN ('orchestrator','comment-orchestrator')",
    )?;
    Ok(statement
        .query_map([], |row| row.get(0))?
        .collect::<std::result::Result<_, _>>()?)
}

pub(super) fn record_turn_interruptions(tx: &rusqlite::Transaction<'_>) -> Result<()> {
    for (turn, conversation, plan, role) in running_turn_rows(tx)? {
        record_turn_interruption(tx, &turn, &conversation, plan.as_deref(), &role)?;
    }
    tx.execute(
        "UPDATE turns SET status='interrupted',finished_ms=?1 WHERE status='running'",
        [now_ms()],
    )?;
    Ok(())
}

fn running_turn_rows(tx: &rusqlite::Transaction<'_>) -> Result<Vec<RunningTurn>> {
    let mut statement =
        tx.prepare("SELECT id,conversation_id,plan_id,role FROM turns WHERE status='running'")?;
    Ok(statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<std::result::Result<_, _>>()?)
}

fn record_turn_interruption(
    tx: &rusqlite::Transaction<'_>,
    turn: &str,
    conversation: &str,
    plan: Option<&str>,
    role: &str,
) -> Result<()> {
    tx.execute(
        "INSERT INTO events(conversation_id,turn_id,plan_id,kind,payload,created_ms) VALUES(?1,?2,?3,'turn_finished',?4,?5)",
        params![conversation, turn, plan, serde_json::json!({"role": role, "status": "interrupted", "error": "the daemon stopped while this turn ran"}).to_string(), now_ms()],
    )?;
    Ok(())
}

pub(super) fn unfinished_plans(tx: &rusqlite::Transaction<'_>) -> Result<Vec<(String, String)>> {
    let mut statement = tx.prepare(
        "SELECT DISTINCT conversation_id, plan_id FROM tasks
         WHERE status IN ('queued','working','verifying')",
    )?;
    Ok(statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?)
}

impl Store {
    pub(super) fn mark_tasks_interrupted(&self, tx: &rusqlite::Transaction<'_>) -> Result<()> {
        for (id, report) in unfinished_task_rows(tx)? {
            self.mark_task_interrupted(tx, &id, report.as_deref())?;
        }
        Ok(())
    }

    fn mark_task_interrupted(
        &self,
        tx: &rusqlite::Transaction<'_>,
        id: &str,
        report: Option<&str>,
    ) -> Result<()> {
        let durable:Option<String>=tx.query_row("SELECT r.result FROM turns t JOIN turn_scopes s ON s.turn_id=t.id JOIN turn_inputs x ON x.turn_id=t.id LEFT JOIN agent_results r ON r.turn_id=t.id WHERE s.task_id=?1 AND t.role='worker:'||?1 AND s.round=(SELECT round FROM tasks WHERE id=?1) AND x.generation=coalesce((SELECT generation FROM task_inputs WHERE task_id=?1),0) ORDER BY t.started_ms DESC,t.rowid DESC LIMIT 1",[id],|r|r.get(0)).optional()?.flatten();
        let completed = durable
            .as_ref()
            .and_then(|text| serde_json::from_str::<Value>(text).ok())
            .is_some_and(|r| r["type"] == "completed");
        let correction:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM mail WHERE conversation_id=(SELECT conversation_id FROM tasks WHERE id=?1) AND recipient='worker:'||?1 AND delivered_turn IS NULL)",[id],|r|r.get(0))?;
        if completed && !correction {
            tx.execute("UPDATE tasks SET feedback=json_set(coalesce(feedback,'{}'),'$.resume_role','verifier') WHERE id=?1",[id])?;
        }
        let text = interrupted_report(self, report)?;
        tx.execute(
            "UPDATE tasks SET status='queued',report=?2 WHERE id=?1",
            params![id, self.content.encode(&text)?],
        )?;
        Ok(())
    }
}

fn unfinished_task_rows(tx: &rusqlite::Transaction<'_>) -> Result<Vec<(String, Option<String>)>> {
    let mut statement =
        tx.prepare("SELECT id,report FROM tasks WHERE status IN ('queued','working','verifying')")?;
    Ok(statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?)
}

fn interrupted_report(store: &Store, report: Option<&str>) -> Result<String> {
    let mut text = report
        .map(|report| store.content.decode(report))
        .transpose()?
        .unwrap_or_default();
    if !text.is_empty() {
        text.push_str("\n\n");
    }
    text.push_str("[interrupted] the daemon stopped while this task ran");
    Ok(text)
}

pub(super) fn insert_proposed_tasks(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    plan_id: &str,
    specs: &[Value],
    now: i64,
) -> Result<Vec<Task>> {
    specs
        .iter()
        .enumerate()
        .map(|(position, spec)| {
            insert_proposed_task(tx, conversation, plan_id, position, spec, now)
        })
        .collect()
}

fn insert_proposed_task(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    plan_id: &str,
    position: usize,
    spec: &Value,
    now: i64,
) -> Result<Task> {
    let task = proposed_task(plan_id, spec);
    tx.execute(
        "INSERT INTO tasks(id,conversation_id,plan_id,position,spec,status,created_ms)
         VALUES(?1,?2,?3,?4,?5,'proposed',?6)",
        params![
            task.id,
            conversation,
            plan_id,
            position as i64,
            spec.to_string(),
            now
        ],
    )?;
    Ok(task)
}

fn proposed_task(plan_id: &str, spec: &Value) -> Task {
    Task {
        input_generation: 0,
        id: crate::id(),
        plan_id: plan_id.into(),
        spec: spec.clone(),
        status: "proposed".into(),
        round: 0,
        report: None,
        verdict: None,
        base: None,
        result: None,
        feedback: None,
        freshness_token: None,
        checks: None,
    }
}

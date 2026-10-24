use super::*;

impl Store {
    pub fn set_check(
        &self,
        task: &str,
        round: u32,
        check: &str,
        state: &str,
        status: Option<&str>,
        note: &str,
    ) -> Result<bool> {
        let before = previous_check(self, task, round, check)?;
        let Some((state, status, note)) = merge_check(before, state, status, note) else {
            return Ok(false);
        };
        let changed = self.db().execute(
            "INSERT INTO checks(task_id,round,check_id,state,status,note,updated_ms)
             VALUES(?1,?2,?3,?4,?5,?6,?7)
             ON CONFLICT(task_id,round,check_id) DO UPDATE SET
               state=?4,status=?5,note=?6,updated_ms=?7
             WHERE state IS NOT ?4 OR status IS NOT ?5 OR note IS NOT ?6",
            params![task, round, check, state, status, note, now_ms()],
        )?;
        Ok(changed > 0)
    }

    pub fn update_task(&self, task: &Task) -> Result<()> {
        let stored = StoredTaskContent::encode(self, task)?;
        let snapshot = task_snapshot(self, task, &stored)?;
        let content = self.content.write(&serde_json::to_string(&snapshot)?)?;
        save_task_round(self, task, stored, serde_json::to_string(&content)?)
    }
}

fn previous_check(
    store: &Store,
    task: &str,
    round: u32,
    check: &str,
) -> Result<Option<(String, Option<String>, String)>> {
    Ok(store
        .db()
        .query_row(
            "SELECT state,status,note FROM checks WHERE task_id=?1 AND round=?2 AND check_id=?3",
            params![task, round, check],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?)
}

fn merge_check<'a>(
    previous: Option<(String, Option<String>, String)>,
    state: &'a str,
    status: Option<&'a str>,
    note: &str,
) -> Option<(&'a str, Option<String>, String)> {
    match previous {
        Some((old_state, ..)) if old_state == "settled" && state == "checking" => None,
        Some((_, old_status, old_note)) => Some((
            state,
            status.map(str::to_owned).or(old_status),
            if note.is_empty() {
                old_note
            } else {
                note.to_owned()
            },
        )),
        None => Some((state, status.map(str::to_owned), note.to_owned())),
    }
}

struct StoredTaskContent {
    report: Option<String>,
    verdict: Option<String>,
}

impl StoredTaskContent {
    fn encode(store: &Store, task: &Task) -> Result<Self> {
        Ok(Self {
            report: task
                .report
                .as_deref()
                .map(|text| store.content.encode(text))
                .transpose()?,
            verdict: task
                .verdict
                .as_ref()
                .map(|value| store.content.encode(&value.to_string()))
                .transpose()?,
        })
    }
}

fn task_snapshot(store: &Store, task: &Task, stored: &StoredTaskContent) -> Result<Value> {
    let mut snapshot = serde_json::to_value(task)?;
    encode_snapshot_field(&mut snapshot, "report", stored.report.as_deref())?;
    encode_snapshot_field(&mut snapshot, "verdict", stored.verdict.as_deref())?;
    retain_previous_result(store, task, &mut snapshot)?;
    Ok(snapshot)
}

fn encode_snapshot_field(snapshot: &mut Value, field: &str, encoded: Option<&str>) -> Result<()> {
    if let Some(encoded) = encoded {
        let reference: Value = serde_json::from_str(
            encoded
                .strip_prefix("\0antislop-content:")
                .expect("encoded content"),
        )?;
        snapshot[field] = serde_json::json!({"session_content":reference});
    }
    Ok(())
}

fn retain_previous_result(store: &Store, task: &Task, snapshot: &mut Value) -> Result<()> {
    let previous: Option<String> = store
        .db()
        .query_row(
            "SELECT content FROM task_rounds WHERE task_id=?1 AND round=?2",
            params![task.id, task.round],
            |row| row.get(0),
        )
        .optional()?;
    let Some(previous) = previous else {
        return Ok(());
    };
    let reference = serde_json::from_str(&previous)?;
    let previous: Value = serde_json::from_str(&store.content.read(&reference)?)?;
    for field in ["report", "verdict"] {
        if snapshot[field].is_null() {
            snapshot[field] = previous[field].clone();
        }
    }
    if snapshot["result"].is_null() && !previous["result"].is_null() {
        snapshot["base"] = previous["base"].clone();
        snapshot["result"] = previous["result"].clone();
    }
    Ok(())
}

fn save_task_round(
    store: &Store,
    task: &Task,
    stored: StoredTaskContent,
    content: String,
) -> Result<()> {
    let mut db = store.db();
    let transaction = db.transaction()?;
    continue_prior_round(&transaction, task)?;
    let changed = update_task_row(&transaction, task, stored)?;
    crate::require(
        changed == 1,
        "stale task generation; result retained in turn history",
    )?;
    insert_task_round(&transaction, task, &content)?;
    transaction.commit()?;
    Ok(())
}

fn continue_prior_round(transaction: &rusqlite::Transaction<'_>, task: &Task) -> Result<()> {
    transaction.execute(
        "UPDATE task_rounds SET status='continued' WHERE task_id=?1 AND round=(SELECT round FROM tasks WHERE id=?1) AND round<?2 AND status IN ('working','verifying','queued')",
        params![task.id, task.round],
    )?;
    Ok(())
}

fn update_task_row(
    transaction: &rusqlite::Transaction<'_>,
    task: &Task,
    stored: StoredTaskContent,
) -> Result<usize> {
    Ok(transaction.execute(
        "UPDATE tasks SET status=?2,round=?3,report=?4,verdict=?5,base=?6,result=?7,feedback=?8 WHERE id=?1 AND coalesce((SELECT generation FROM task_inputs WHERE task_id=?1),0)=?9",
        params![task.id, task.status, task.round as i64, stored.report, stored.verdict,
            task.base, task.result, task.feedback.as_ref().map(Value::to_string), task.input_generation],
    )?)
}

fn insert_task_round(
    transaction: &rusqlite::Transaction<'_>,
    task: &Task,
    content: &str,
) -> Result<()> {
    transaction.execute(
        "INSERT INTO task_rounds(task_id,round,status,content) VALUES(?1,?2,?3,?4)
         ON CONFLICT(task_id,round) DO UPDATE SET status=excluded.status,content=excluded.content",
        params![task.id, task.round, task.status, content],
    )?;
    Ok(())
}

use super::*;
use rusqlite::params;

pub(super) fn apply_specs(
    tx: &rusqlite::Transaction<'_>,
    context: &EditContext<'_>,
    baseline: &[Task],
    specs: &[TaskSpec],
) -> Result<()> {
    for (position, spec) in specs.iter().enumerate() {
        let old = baseline
            .iter()
            .find(|task| task.spec["id"].as_str() == Some(&spec.id));
        let Some((id, task_status, round)) = apply_spec(tx, context, old, position, spec)? else {
            continue;
        };
        record_graph_task(tx, context, &id, &task_status, round, spec, old)?;
    }
    Ok(())
}

fn apply_spec(
    tx: &rusqlite::Transaction<'_>,
    context: &EditContext<'_>,
    old: Option<&Task>,
    position: usize,
    spec: &TaskSpec,
) -> Result<Option<(String, String, u32)>> {
    if let Some(old) = old {
        update_existing_task(tx, old, spec, position)
    } else {
        insert_new_task(tx, context, spec, position).map(Some)
    }
}

fn update_existing_task(
    tx: &rusqlite::Transaction<'_>,
    old: &Task,
    spec: &TaskSpec,
    position: usize,
) -> Result<Option<(String, String, u32)>> {
    let value = serde_json::to_value(spec)?;
    let before: TaskSpec = serde_json::from_value(old.spec.clone())?;
    if before == *spec {
        tx.execute(
            "UPDATE tasks SET position=?2 WHERE id=?1",
            params![old.id, position as i64],
        )?;
        return Ok(None);
    }
    validate_dependency_only_change(&before, spec)?;
    let (state, round) = task_state(tx, old, spec)?;
    tx.execute(
        "UPDATE tasks SET spec=?2,position=?3 WHERE id=?1",
        params![old.id, value.to_string(), position as i64],
    )?;
    Ok(Some((old.id.clone(), state, round)))
}

fn validate_dependency_only_change(before: &TaskSpec, after: &TaskSpec) -> Result<()> {
    let mut comparable = before.clone();
    comparable.depends_on = after.depends_on.clone();
    require(
        comparable == *after,
        "running graph edits may change only dependencies of an existing unstarted task; add a new task for other changes",
    )
}

fn task_state(
    tx: &rusqlite::Transaction<'_>,
    old: &Task,
    spec: &TaskSpec,
) -> Result<(String, u32)> {
    let (state, round): (String, u32) = tx.query_row(
        "SELECT status,round FROM tasks WHERE id=?1",
        [&old.id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    require(
        matches!(state.as_str(), status::QUEUED | status::PROPOSED) && round == 0,
        &format!(
            "task {} has already started; its dependencies and history cannot be rewritten",
            spec.id
        ),
    )?;
    Ok((state, round))
}

fn insert_new_task(
    tx: &rusqlite::Transaction<'_>,
    context: &EditContext<'_>,
    spec: &TaskSpec,
    position: usize,
) -> Result<(String, String, u32)> {
    let id = crate::id();
    let value = serde_json::to_value(spec)?;
    tx.execute(
        "INSERT INTO tasks(id,conversation_id,plan_id,position,spec,status,created_ms) VALUES(?1,?2,?3,?4,?5,'queued',?6)",
        params![id, context.conversation, context.plan, position as i64, value.to_string(), now_ms()],
    )?;
    tx.execute(
        "INSERT INTO task_inputs(task_id,run_id) VALUES(?1,?2)",
        params![id, context.run],
    )?;
    insert_requirement_inputs(tx, &id, &context.run, &context.entries, spec)?;
    Ok((id, status::QUEUED.into(), 0))
}

fn insert_requirement_inputs(
    tx: &rusqlite::Transaction<'_>,
    task: &str,
    run: &str,
    entries: &[crate::lean::ledger::LedgerEntry],
    spec: &TaskSpec,
) -> Result<()> {
    let ids: std::collections::BTreeSet<_> = spec.requirement_ids.iter().collect();
    for id in ids {
        let entry = entries
            .iter()
            .find(|entry| entry.spec.id == *id)
            .expect("validated entry");
        let latest: i64 = tx.query_row(
            "SELECT max(version) FROM ledger_versions WHERE run_id=?1 AND entry_id=?2",
            params![run, id],
            |row| row.get(0),
        )?;
        require(
            latest == entry.version,
            "requirements changed during graph review; retry",
        )?;
        tx.execute(
            "INSERT INTO task_ledger_inputs VALUES(?1,?2,?3)",
            params![task, id, entry.version],
        )?;
    }
    Ok(())
}

fn record_graph_task(
    tx: &rusqlite::Transaction<'_>,
    context: &EditContext<'_>,
    id: &str,
    task_status: &str,
    round: u32,
    spec: &TaskSpec,
    old: Option<&Task>,
) -> Result<()> {
    let old_dependencies = old.map(|task| &task.spec["depends_on"]);
    tx.execute(
        "INSERT INTO events(conversation_id,plan_id,kind,payload,created_ms) VALUES(?1,?2,'graph_task',?3,?4)",
        params![context.conversation, context.plan, serde_json::json!({"plan":context.plan,"id":id,"spec":spec,"status":task_status,"round":round,"old_dependencies":old_dependencies}).to_string(), now_ms()],
    )?;
    Ok(())
}

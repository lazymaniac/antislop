use super::*;

impl RevisionApply<'_> {
    pub(super) fn verify_lease(
        &self,
        tx: &rusqlite::Transaction<'_>,
        lease: &RevisionLease<'_>,
    ) -> Result<()> {
        let current = tx.query_row(
            "SELECT spec_revision FROM runs WHERE id=?1",
            [&self.run],
            |row| row.get::<_, i64>(0),
        )?;
        crate::require(
            current == lease.requirement_revision,
            "requirements changed during revision review",
        )?;
        let changed = tx.execute(
            "UPDATE plan_heads SET revision=revision+1 WHERE plan_id=?1 AND revision=?2",
            params![self.revision.plan, self.revision.base_revision],
        )?;
        crate::require(changed == 1, "stale plan revision")
    }

    pub(super) fn update_removed_tasks(&self, tx: &rusqlite::Transaction<'_>) -> Result<()> {
        for old in self.baseline {
            verify_task_snapshot(tx, old)?;
            if self.affected.contains(&old.id) {
                invalidate_task_questions(tx, &old.id)?;
            }
            if !self.specs.iter().any(|spec| old.spec["id"] == spec.id) {
                retire_task(tx, &old.id)?;
            }
        }
        Ok(())
    }

    pub(super) fn save_task(
        &self,
        tx: &rusqlite::Transaction<'_>,
        position: usize,
        spec: &TaskContract,
    ) -> Result<()> {
        let old = self.baseline.iter().find(|task| task.spec["id"] == spec.id);
        let id = old.map_or_else(crate::id, |task| task.id.clone());
        let encoded = serde_json::to_string(spec)?;
        let reworded =
            old.is_some_and(|task| !self.affected.contains(&id) && !same_contract(task, spec));
        save_task_row(tx, self, old, &id, position as i64, &encoded, reworded)?;
        save_task_inputs(tx, self, old, spec, &id)?;
        save_task_contract(tx, self, old, &id, &encoded, reworded)
    }

    pub(super) fn record_graph(&self, tx: &rusqlite::Transaction<'_>) -> Result<()> {
        for spec in self.specs {
            let preserved = self.baseline.iter().any(|old| {
                old.spec["id"] == spec.id
                    && !self.affected.contains(&old.id)
                    && same_contract(old, spec)
            });
            if !preserved {
                record_graph_task(tx, self, spec)?;
            }
        }
        tx.execute(
            "INSERT INTO events(conversation_id,plan_id,kind,payload,created_ms) VALUES(?1,?2,'graph_updated',?3,?4)",
            params![self.conversation, self.revision.plan,
                serde_json::json!({"plan":self.revision.plan,"text":self.revision.reason,"order":self.specs.iter().map(|spec|&spec.id).collect::<Vec<_>>(),"revision":self.revision.base_revision+1}).to_string(), now_ms()],
        )?;
        Ok(())
    }

    pub(super) fn finish(
        &self,
        tx: &rusqlite::Transaction<'_>,
        lease: &RevisionLease<'_>,
    ) -> Result<()> {
        tx.execute("UPDATE runs SET state='active' WHERE id=?1", [&self.run])?;
        let changed = tx.execute(
            "UPDATE plan_revision_jobs SET status='applied',error=NULL WHERE id=?1 AND status='reviewing'",
            [lease.request],
        )?;
        crate::require(changed == 1, "revision receipt is no longer active")
    }
}

fn save_task_row(
    tx: &rusqlite::Transaction<'_>,
    apply: &RevisionApply<'_>,
    old: Option<&Task>,
    id: &str,
    position: i64,
    encoded: &str,
    reworded: bool,
) -> Result<()> {
    match old {
        Some(_) if apply.affected.contains(id) => reset_affected_task(tx, id, position, encoded)?,
        Some(_) => update_preserved_task(tx, id, position, encoded, reworded)?,
        None => insert_revision_task(tx, apply, id, position, encoded)?,
    }
    Ok(())
}

fn save_task_inputs(
    tx: &rusqlite::Transaction<'_>,
    apply: &RevisionApply<'_>,
    old: Option<&Task>,
    spec: &TaskContract,
    id: &str,
) -> Result<()> {
    if old.is_none() || apply.affected.contains(id) {
        tx.execute("DELETE FROM task_ledger_inputs WHERE task_id=?1", [id])?;
        insert_revision_ledger_links(tx, id, spec, &apply.entries)?;
    }
    Ok(())
}

fn save_task_contract(
    tx: &rusqlite::Transaction<'_>,
    apply: &RevisionApply<'_>,
    old: Option<&Task>,
    id: &str,
    encoded: &str,
    reworded: bool,
) -> Result<()> {
    if old.is_none() || apply.affected.contains(id) || reworded {
        let version = apply.revision.base_revision + 1;
        tx.execute(
            "INSERT INTO task_contracts(task_id,version,spec) VALUES(?1,?2,?3)",
            params![id, version, encoded],
        )?;
        record_contract_event(tx, apply, id, version)?;
    }
    Ok(())
}

pub(super) fn validate_revision_coverage(
    specs: &[TaskContract],
    entries: &[crate::lean::ledger::LedgerEntry],
) -> Result<()> {
    let mut covered = std::collections::HashSet::new();
    for spec in specs {
        crate::lean::ledger::validate_task_links(spec, entries)?;
        covered.extend(spec.criterion_ids.iter().cloned());
    }
    crate::lean::ledger::validate_criterion_coverage(entries, &covered)
}

fn verify_task_snapshot(tx: &rusqlite::Transaction<'_>, old: &Task) -> Result<()> {
    let generation = tx.query_row(
        "SELECT coalesce((SELECT generation FROM task_inputs WHERE task_id=?1),0)",
        [&old.id],
        |row| row.get::<_, i64>(0),
    )?;
    crate::require(
        generation == old.input_generation,
        "task inputs changed during review",
    )?;
    let stored = tx.query_row("SELECT spec FROM tasks WHERE id=?1", [&old.id], |row| {
        row.get::<_, String>(0)
    })?;
    crate::require(
        serde_json::from_str::<Value>(&stored)? == old.spec,
        "contract changed during review",
    )
}

fn invalidate_task_questions(tx: &rusqlite::Transaction<'_>, task_id: &str) -> Result<()> {
    tx.execute("UPDATE agent_questions SET state='invalidated' WHERE turn_id IN (SELECT turn_id FROM turn_scopes WHERE task_id=?1) AND state IN ('pending','awaiting_user')", [task_id])?;
    Ok(())
}

fn retire_task(tx: &rusqlite::Transaction<'_>, task_id: &str) -> Result<()> {
    tx.execute(
        "UPDATE task_inputs SET generation=generation+1 WHERE task_id=?1",
        [task_id],
    )?;
    tx.execute(
        "UPDATE tasks SET status='superseded' WHERE id=?1",
        [task_id],
    )?;
    Ok(())
}

fn reset_affected_task(
    tx: &rusqlite::Transaction<'_>,
    id: &str,
    position: i64,
    encoded: &str,
) -> Result<()> {
    tx.execute("UPDATE tasks SET spec=?2,position=?3,status='queued',verdict=NULL,feedback=NULL,base=NULL,result=NULL WHERE id=?1", params![id, encoded, position])?;
    tx.execute(
        "UPDATE task_inputs SET generation=generation+1 WHERE task_id=?1",
        [id],
    )?;
    Ok(())
}

fn update_preserved_task(
    tx: &rusqlite::Transaction<'_>,
    id: &str,
    position: i64,
    encoded: &str,
    reworded: bool,
) -> Result<()> {
    tx.execute(
        "UPDATE tasks SET spec=CASE WHEN ?3 THEN ?4 ELSE spec END,position=?2 WHERE id=?1",
        params![id, position, reworded, encoded],
    )?;
    Ok(())
}

fn insert_revision_task(
    tx: &rusqlite::Transaction<'_>,
    apply: &RevisionApply<'_>,
    id: &str,
    position: i64,
    encoded: &str,
) -> Result<()> {
    tx.execute("INSERT INTO tasks(id,conversation_id,plan_id,position,spec,status,created_ms) VALUES(?1,?2,?3,?4,?5,'queued',?6)", params![id, apply.conversation, apply.revision.plan, position, encoded, now_ms()])?;
    tx.execute(
        "INSERT INTO task_inputs(task_id,run_id) VALUES(?1,?2)",
        params![id, apply.run],
    )?;
    Ok(())
}

fn insert_revision_ledger_links(
    tx: &rusqlite::Transaction<'_>,
    task_id: &str,
    spec: &TaskContract,
    entries: &[crate::lean::ledger::LedgerEntry],
) -> Result<()> {
    for entry in entries
        .iter()
        .filter(|entry| spec.requirement_ids.contains(&entry.spec.id))
    {
        tx.execute(
            "INSERT INTO task_ledger_inputs VALUES(?1,?2,?3)",
            params![task_id, entry.spec.id, entry.version],
        )?;
    }
    Ok(())
}

fn record_contract_event(
    tx: &rusqlite::Transaction<'_>,
    apply: &RevisionApply<'_>,
    task_id: &str,
    version: i64,
) -> Result<()> {
    tx.execute("INSERT INTO events(conversation_id,plan_id,kind,payload,created_ms) VALUES(?1,?2,'task_contract_changed',?3,?4)",
        params![apply.conversation, apply.revision.plan, serde_json::json!({"task":task_id,"version":version}).to_string(), now_ms()])?;
    Ok(())
}

fn record_graph_task(
    tx: &rusqlite::Transaction<'_>,
    apply: &RevisionApply<'_>,
    spec: &TaskContract,
) -> Result<()> {
    let (id, status, round): (String, String, i64) = tx.query_row(
        "SELECT id,status,round FROM tasks WHERE plan_id=?1 AND json_extract(spec,'$.id')=?2 AND status<>'superseded'",
        params![apply.revision.plan, spec.id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    tx.execute("INSERT INTO events(conversation_id,plan_id,kind,payload,created_ms) VALUES(?1,?2,'graph_task',?3,?4)",
        params![apply.conversation, apply.revision.plan, serde_json::json!({"plan":apply.revision.plan,"id":id,"status":status,"round":round,"spec":spec}).to_string(), now_ms()])?;
    Ok(())
}

//! Inserting a task before a blocked one. The blocked task keeps its rounds
//! and history; it waits for the new task, then goes on in a fresh round.
use super::{Store, Task, now_ms};
use crate::lean::{
    blocker,
    tasks::{TaskSpec, status},
};
use crate::{Result, require};
use rusqlite::params;

impl Store {
    /// Inserts `spec` before `blocked`, makes `blocked` depend on it and
    /// queues `blocked` again. Returns the new task's id. The caller holds
    /// the graph lock; every rule lives in [`blocker::check_unblocker`].
    pub fn insert_unblocker(
        &self,
        conversation: &str,
        blocked: &Task,
        mut spec: TaskSpec,
    ) -> Result<String> {
        require(
            blocked.status == status::NEEDS_USER,
            "only a task waiting for the user has a blocker to clear",
        )?;
        let before: TaskSpec = serde_json::from_value(blocked.spec.clone())?;
        spec.resolves = Some(before.id.clone());
        if spec.requirement_ids.is_empty() {
            spec.requirement_ids = before.requirement_ids.clone();
        }
        if spec.criterion_ids.is_empty() {
            spec.criterion_ids = before.criterion_ids.clone();
        }
        let plan = self
            .plan(&blocked.plan_id)?
            .into_iter()
            .map(|task| Ok((serde_json::from_value(task.spec)?, task.status)))
            .collect::<Result<Vec<(TaskSpec, String)>>>()?;
        blocker::check_unblocker(&spec, &before, &plan)?;
        let mut after = before.clone();
        after.depends_on.push(spec.id.clone());
        let uuid = crate::id();
        let mut db = self.db();
        let tx = db.transaction()?;
        let position: i64 = tx.query_row(
            "SELECT position FROM tasks WHERE id=?1",
            [&blocked.id],
            |row| row.get(0),
        )?;
        tx.execute(
            "UPDATE tasks SET position=position+1 WHERE plan_id=?1 AND position>=?2",
            params![blocked.plan_id, position],
        )?;
        tx.execute(
            "INSERT INTO tasks(id,conversation_id,plan_id,position,spec,status,created_ms) VALUES(?1,?2,?3,?4,?5,'queued',?6)",
            params![uuid, conversation, blocked.plan_id, position, serde_json::to_string(&spec)?, now_ms()],
        )?;
        tx.execute(
            "INSERT INTO task_inputs(task_id,run_id) SELECT ?1,run_id FROM task_inputs WHERE task_id=?2",
            params![uuid, blocked.id],
        )?;
        for id in &spec.requirement_ids {
            tx.execute(
                "INSERT INTO task_ledger_inputs SELECT ?1,entry_id,version FROM task_ledger_inputs WHERE task_id=?2 AND entry_id=?3",
                params![uuid, blocked.id, id],
            )?;
        }
        tx.execute(
            "UPDATE tasks SET spec=?2,status='queued',verdict=NULL,feedback=NULL WHERE id=?1",
            params![blocked.id, serde_json::to_string(&after)?],
        )?;
        for (id, spec, round, old) in [
            (&uuid, &spec, 0, None),
            (&blocked.id, &after, blocked.round, Some(&before.depends_on)),
        ] {
            let payload = serde_json::json!({"plan": blocked.plan_id, "id": id, "spec": spec,
                "status": "queued", "round": round, "old_dependencies": old});
            tx.execute(
                "INSERT INTO events(conversation_id,plan_id,kind,payload,created_ms) VALUES(?1,?2,'graph_task',?3,?4)",
                params![conversation, blocked.plan_id, payload.to_string(), now_ms()],
            )?;
        }
        tx.commit()?;
        Ok(uuid)
    }
}

//! Inserting tasks before a blocked one: one that clears its blocker, or the
//! parts it was cut into. The blocked task keeps its rounds and history; it
//! waits for what was inserted, then goes on in a fresh round.
use super::{Store, Task, now_ms};
use crate::lean::{
    blocker, split,
    tasks::{TaskSpec, status},
};
use crate::{Result, require};
use rusqlite::{Transaction, params};

/// A new task in the blocked task's plan, serving the same requirements:
/// its row, its ledger inputs, and its `graph_task` event.
fn insert_task(
    tx: &Transaction,
    conversation: &str,
    blocked: &Task,
    position: i64,
    spec: &TaskSpec,
) -> Result<String> {
    let uuid = crate::id();
    tx.execute(
        "INSERT INTO tasks(id,conversation_id,plan_id,position,spec,status,created_ms) VALUES(?1,?2,?3,?4,?5,'queued',?6)",
        params![uuid, conversation, blocked.plan_id, position, serde_json::to_string(spec)?, now_ms()],
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
    graph_event(tx, conversation, blocked, &uuid, spec, 0, None)?;
    Ok(uuid)
}

fn graph_event(
    tx: &Transaction,
    conversation: &str,
    blocked: &Task,
    id: &str,
    spec: &TaskSpec,
    round: u32,
    old_dependencies: Option<&Vec<String>>,
) -> Result<()> {
    let payload = serde_json::json!({"plan": blocked.plan_id, "id": id, "spec": spec,
        "status": "queued", "round": round, "old_dependencies": old_dependencies});
    tx.execute(
        "INSERT INTO events(conversation_id,plan_id,kind,payload,created_ms) VALUES(?1,?2,'graph_task',?3,?4)",
        params![conversation, blocked.plan_id, payload.to_string(), now_ms()],
    )?;
    Ok(())
}

fn clarification_count(db: &rusqlite::Connection, task: &str) -> Result<usize> {
    Ok(db.query_row(
        "SELECT count(*) FROM events WHERE kind='task_clarified' AND json_extract(payload,'$.task')=?1",
        [task], |row| row.get::<_, i64>(0).map(|count| count as usize),
    )?)
}

impl Store {
    pub(in crate::lean) fn clarification_count(&self, task: &str) -> Result<usize> {
        clarification_count(&self.db(), task)
    }

    /// Preserve old rounds and record both contracts atomically. A new input
    /// generation prevents an old worker/verifier result from overwriting it.
    pub(in crate::lean) fn clarify_task(
        &self,
        conversation: &str,
        blocked: &Task,
        spec: &TaskSpec,
        review: &crate::lean::tasks::Verdict,
        reason: &str,
    ) -> Result<()> {
        let before: TaskSpec = serde_json::from_value(blocked.spec.clone())?;
        blocker::check_clarification(&before, spec)?;
        require(
            review.approved,
            "task clarification was not independently approved",
        )?;
        require(
            blocked.status == status::NEEDS_USER,
            "only a blocked task can be clarified",
        )?;
        let mut db = self.db();
        let tx = db.transaction()?;
        require(
            clarification_count(&tx, &blocked.id)? < blocker::CLARIFICATIONS,
            "task was already clarified twice; the user decides",
        )?;
        let current: String =
            tx.query_row("SELECT spec FROM tasks WHERE id=?1", [&blocked.id], |row| {
                row.get(0)
            })?;
        require(
            serde_json::from_str::<TaskSpec>(&current)? == before,
            "task changed during clarification review",
        )?;
        let changed = tx.execute(
            "UPDATE tasks SET spec=?2,status='queued',verdict=NULL,feedback=NULL WHERE id=?1 AND conversation_id=?3 AND status='needs_user' AND round=?4 AND spec=?5 AND coalesce((SELECT generation FROM task_inputs WHERE task_id=?1),0)=?6",
            params![blocked.id, serde_json::to_string(spec)?, conversation, blocked.round, current, blocked.input_generation],
        )?;
        require(changed == 1, "task changed during clarification review")?;
        let bumped = tx.execute(
            "UPDATE task_inputs SET generation=generation+1 WHERE task_id=?1",
            [&blocked.id],
        )?;
        require(bumped == 1, "task has no input binding")?;
        graph_event(
            &tx,
            conversation,
            blocked,
            &blocked.id,
            spec,
            blocked.round,
            None,
        )?;
        let payload = serde_json::json!({"task":blocked.id,"before":before,"after":spec,"review":review,"reason":reason});
        tx.execute(
            "INSERT INTO events(conversation_id,plan_id,kind,payload,created_ms) VALUES(?1,?2,'task_clarified',?3,?4)",
            params![conversation, blocked.plan_id, payload.to_string(), now_ms()],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// The plan's tasks with their statuses, in order.
    fn plan_with_status(&self, blocked: &Task) -> Result<Vec<(TaskSpec, String)>> {
        self.plan(&blocked.plan_id)?
            .into_iter()
            .map(|task| Ok((serde_json::from_value(task.spec)?, task.status)))
            .collect()
    }

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
        if spec.scope.is_empty() {
            spec.scope = before.scope.clone();
        }
        for dependency in &before.depends_on {
            if !spec.depends_on.contains(dependency) {
                spec.depends_on.push(dependency.clone());
            }
        }
        blocker::check_unblocker(&spec, &before, &self.plan_with_status(blocked)?)?;
        let mut after = before.clone();
        after.depends_on.push(spec.id.clone());
        let mut db = self.db();
        let tx = db.transaction()?;
        let position = shift_from(&tx, blocked, 1)?;
        let uuid = insert_task(&tx, conversation, blocked, position, &spec)?;
        requeue(&tx, conversation, blocked, &before, &after, false)?;
        tx.commit()?;
        Ok(uuid)
    }

    /// Whether `parts` may be cut from `blocked`, before anything is touched.
    pub fn check_parts(&self, blocked: &Task, parts: &[TaskSpec]) -> Result<()> {
        let before: TaskSpec = serde_json::from_value(blocked.spec.clone())?;
        split::prepare(parts, &before, &self.plan_with_status(blocked)?).map(|_| ())
    }

    /// Cuts `blocked` into `parts` that come before it. `blocked` becomes the
    /// task that puts them together: it waits for all of them, and a writer
    /// starts over from a base that holds their results. The caller holds the
    /// graph lock; the rules live in [`split::prepare`].
    pub fn insert_parts(
        &self,
        conversation: &str,
        blocked: &Task,
        parts: &[TaskSpec],
    ) -> Result<Vec<String>> {
        require(
            blocked.status == status::NEEDS_USER,
            "only a task waiting for the user can be cut up",
        )?;
        let before: TaskSpec = serde_json::from_value(blocked.spec.clone())?;
        let ready = split::prepare(parts, &before, &self.plan_with_status(blocked)?)?;
        let mut after = before.clone();
        after
            .depends_on
            .extend(ready.iter().map(|part| part.id.clone()));
        let mut db = self.db();
        let tx = db.transaction()?;
        let position = shift_from(&tx, blocked, ready.len() as i64)?;
        let ids = ready
            .iter()
            .enumerate()
            .map(|(offset, part)| {
                insert_task(&tx, conversation, blocked, position + offset as i64, part)
            })
            .collect::<Result<Vec<_>>>()?;
        requeue(&tx, conversation, blocked, &before, &after, true)?;
        tx.commit()?;
        Ok(ids)
    }
}

/// Makes room for `count` tasks where `blocked` stands; returns that place.
fn shift_from(tx: &Transaction, blocked: &Task, count: i64) -> Result<i64> {
    let position: i64 = tx.query_row(
        "SELECT position FROM tasks WHERE id=?1",
        [&blocked.id],
        |row| row.get(0),
    )?;
    tx.execute(
        "UPDATE tasks SET position=position+?3 WHERE plan_id=?1 AND position>=?2",
        params![blocked.plan_id, position, count],
    )?;
    Ok(position)
}

/// The blocked task waits for what was inserted. With `restart`, its
/// writer base and result are dropped, so its next round starts from a base
/// that holds the new tasks' results.
fn requeue(
    tx: &Transaction,
    conversation: &str,
    blocked: &Task,
    before: &TaskSpec,
    after: &TaskSpec,
    restart: bool,
) -> Result<()> {
    let reset = if restart {
        ",base=NULL,result=NULL"
    } else {
        ""
    };
    tx.execute(
        &format!(
            "UPDATE tasks SET spec=?2,status='queued',verdict=NULL,feedback=NULL{reset} WHERE id=?1"
        ),
        params![blocked.id, serde_json::to_string(after)?],
    )?;
    graph_event(
        tx,
        conversation,
        blocked,
        &blocked.id,
        after,
        blocked.round,
        Some(&before.depends_on),
    )
}

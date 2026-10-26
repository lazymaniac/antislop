//! Reviewed task contract corrections and their durable progress history.
use super::unblocker::graph_event;
use super::{Store, Task, now_ms};
use crate::lean::{
    blocker,
    tasks::{TaskSpec, status},
};
use crate::{Result, require};
use rusqlite::{Transaction, params};

impl Store {
    /// Record the proposal before review; identical retries with unchanged
    /// evidence remain detectable after failures, cancellation and restart.
    pub(in crate::lean) fn reserve_clarification(
        &self,
        conversation: &str,
        blocked: &Task,
        spec: &TaskSpec,
        observation: &str,
        reason: &str,
    ) -> Result<String> {
        blocker::check_clarification(&serde_json::from_value(blocked.spec.clone())?, spec)?;
        let mut db = self.db();
        let tx = db.transaction()?;
        let current: (String, String, u32, i64) = tx.query_row(
            "SELECT spec,status,round,coalesce((SELECT generation FROM task_inputs WHERE task_id=?1),0) FROM tasks WHERE id=?1 AND conversation_id=?2",
            params![blocked.id, conversation], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
        )?;
        require(
            serde_json::from_str::<serde_json::Value>(&current.0)? == blocked.spec
                && current.1 == status::NEEDS_USER
                && current.2 == blocked.round
                && current.3 == blocked.input_generation,
            "task changed before clarification review",
        )?;
        let proposal = crate::lean::recovery_progress::proposal(
            "clarify_task",
            Some(&serde_json::to_value(spec)?),
            Some(reason),
            &[],
        );
        let attempt = super::recovery_progress::reserve(
            &tx,
            conversation,
            blocked,
            "clarify_task",
            observation,
            &proposal,
        )?;
        tx.commit()?;
        Ok(attempt)
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
        attempt: Option<&str>,
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
        if let Some(attempt) = attempt {
            check_attempt(&tx, conversation, blocked, spec, reason, attempt)?;
        }
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
        let payload = serde_json::json!({"task":blocked.id,"before":before,"after":spec,"review":review,"reason":reason,"review_attempt":attempt});
        tx.execute(
            "INSERT INTO events(conversation_id,plan_id,kind,payload,created_ms) VALUES(?1,?2,'task_clarified',?3,?4)",
            params![conversation, blocked.plan_id, payload.to_string(), now_ms()],
        )?;
        tx.commit()?;
        Ok(())
    }
}

fn check_attempt(
    tx: &Transaction<'_>,
    conversation: &str,
    blocked: &Task,
    spec: &TaskSpec,
    reason: &str,
    attempt: &str,
) -> Result<()> {
    let payload: String = tx.query_row(
        "SELECT payload FROM events WHERE conversation_id=?1 AND kind='clarification_attempt' AND json_extract(payload,'$.attempt')=?2",
        params![conversation,attempt], |row| row.get(0),
    )?;
    let payload: serde_json::Value = serde_json::from_str(&payload)?;
    let used: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE kind='task_clarified' AND json_extract(payload,'$.review_attempt')=?1)",
        [attempt], |row| row.get(0))?;
    require(
        !used
            && payload["task"] == blocked.id
            && payload["round"] == blocked.round
            && payload["generation"] == blocked.input_generation
            && payload["proposal"]
                == crate::lean::recovery_progress::proposal(
                    "clarify_task",
                    Some(&serde_json::to_value(spec)?),
                    Some(reason),
                    &[],
                ),
        "stale or mismatched clarification review attempt",
    )
}

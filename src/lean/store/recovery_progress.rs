use super::*;
use crate::lean::recovery_progress;

impl Store {
    pub(in crate::lean) fn recovery_user_message(&self, conversation: &str) -> Result<u64> {
        Ok(self.db().query_row(
            "SELECT coalesce(max(seq),0) FROM messages WHERE conversation_id=?1 AND author='user'",
            [conversation],
            |row| row.get::<_, i64>(0),
        )? as u64)
    }

    pub(in crate::lean) fn recovery_history(&self, task: &str, action: &str) -> Result<Vec<Value>> {
        let db = self.db();
        let mut query = db.prepare(
            "SELECT a.payload,(SELECT r.payload FROM events r WHERE r.kind IN ('clarification_reviewed','recovery_progress_reviewed') AND json_extract(r.payload,'$.attempt')=json_extract(a.payload,'$.attempt') ORDER BY r.seq DESC LIMIT 1)
             FROM events a WHERE a.kind IN ('recovery_attempt','clarification_attempt') AND json_extract(a.payload,'$.task')=?1
             AND (json_extract(a.payload,'$.action')=?2 OR (a.kind='clarification_attempt' AND json_extract(a.payload,'$.action') IS NULL AND ?2='clarify_task')) ORDER BY a.seq")?;
        let rows = query
            .query_map(params![task, recovery_progress::family(action)], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(attempt, review)| {
                Ok(serde_json::json!({
                    "attempt":serde_json::from_str::<Value>(&attempt)?,
                    "review":review.map(|value| serde_json::from_str::<Value>(&value)).transpose()?
                }))
            })
            .collect()
    }

    pub(in crate::lean) fn reserve_recovery_attempt(
        &self,
        conversation: &str,
        task: &Task,
        action: &str,
        observation: &str,
        proposal: &Value,
    ) -> Result<String> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let attempt = reserve(&tx, conversation, task, action, observation, proposal)?;
        tx.commit()?;
        Ok(attempt)
    }
}

pub(super) fn reserve(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    task: &Task,
    action: &str,
    observation: &str,
    proposal: &Value,
) -> Result<String> {
    let key = crate::hash(serde_json::to_string(proposal)?.as_bytes());
    let repeated: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM events WHERE kind IN ('recovery_attempt','clarification_attempt') AND conversation_id=?1
         AND json_extract(payload,'$.task')=?2 AND json_extract(payload,'$.action')=?3
         AND json_extract(payload,'$.observation')=?4 AND json_extract(payload,'$.proposal_key')=?5)",
        params![conversation, task.id, recovery_progress::family(action), observation, key], |row| row.get(0))?;
    crate::require(
        !repeated,
        "recovery made no progress: this proposal was already tried with unchanged inputs and outcome; change the approach or supply new evidence",
    )?;
    let attempt = crate::id();
    let kind = if action == "clarify_task" {
        "clarification_attempt"
    } else {
        "recovery_attempt"
    };
    let payload = serde_json::json!({"task":task.id,"attempt":attempt,"action":recovery_progress::family(action),
        "observation":observation,"proposal_key":key,"outcome":crate::lean::tasks::bound(&task.report.as_deref().and_then(crate::lean::blocker::parse).map(|b|b.summary()).unwrap_or_else(|| recovery_progress::outcome(task.report.as_deref().unwrap_or_default().rsplit("\n\n").next().unwrap_or_default())) ,1600),"worker_review":recovery_progress::review_evidence(task.verdict.as_ref()),"proposal":proposal,"round":task.round,"generation":task.input_generation});
    tx.execute("INSERT INTO events(conversation_id,plan_id,kind,payload,created_ms) VALUES(?1,?2,?3,?4,?5)",
        params![conversation,task.plan_id,kind,payload.to_string(),super::now_ms()])?;
    Ok(attempt)
}

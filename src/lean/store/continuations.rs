//! Revisions retain work and request assessment before any executor continuation.
use super::*;

impl Store {
    pub(in crate::lean) fn retain_work(
        tx: &rusqlite::Transaction<'_>,
        source: &Task,
        target: &str,
        generation: i64,
        reason: &str,
    ) -> Result<()> {
        Self::retain_work_from_ids(
            tx,
            &source.id,
            source.input_generation,
            target,
            generation,
            reason,
        )
    }

    pub(in crate::lean) fn retain_work_from_ids(
        tx: &rusqlite::Transaction<'_>,
        source: &str,
        source_generation: i64,
        target: &str,
        generation: i64,
        reason: &str,
    ) -> Result<()> {
        let owner: bool = tx.query_row("SELECT a.conversation_id=b.conversation_id FROM tasks a,tasks b WHERE a.id=?1 AND b.id=?2",params![source,target],|r|r.get(0))?;
        crate::require(owner, "continuation belongs to another conversation")?;
        let result: Option<String> = tx.query_row("SELECT r.turn_id FROM agent_results r JOIN turns t ON t.id=r.turn_id JOIN turn_scopes s ON s.turn_id=t.id JOIN turn_inputs x ON x.turn_id=t.id WHERE s.task_id=?1 AND x.generation=?2 AND t.role='worker:'||?1 AND json_extract(r.result,'$.type')='completed' ORDER BY s.round DESC,t.started_ms DESC,t.rowid DESC LIMIT 1",params![source,source_generation],|r|r.get(0)).optional()?;
        let result = match result {
            Some(result)=>Some(result),
            None=>tx.query_row("SELECT result_turn FROM task_resume_results WHERE task_id=?1 AND generation=?2",params![source,source_generation],|r|r.get::<_,Option<String>>(0)).optional()?.flatten(),
        };
        tx.execute(
            "INSERT OR REPLACE INTO task_resume_results VALUES(?1,?2,?3,?4)",
            params![target, generation, source, result],
        )?;
        tx.execute("UPDATE tasks SET round=(SELECT round FROM tasks WHERE id=?2),report=coalesce((SELECT report FROM tasks WHERE id=?2),(SELECT result FROM agent_results WHERE turn_id=?3)),base=(SELECT base FROM tasks WHERE id=?2),result=(SELECT result FROM tasks WHERE id=?2),verdict=NULL,feedback=json_set(coalesce(feedback,'{}'),'$.continuation_review',json('true'),'$.resume_role',CASE WHEN ?3 IS NOT NULL OR (SELECT report IS NOT NULL FROM tasks WHERE id=?2) THEN 'reviewer' ELSE 'worker' END,'$.continuation_reason',?4) WHERE id=?1",params![target,source,result,reason])?;
        tx.execute("INSERT INTO events(conversation_id,plan_id,kind,payload,created_ms) SELECT conversation_id,plan_id,'task_work_retained',?2,?3 FROM tasks WHERE id=?1",params![target,serde_json::json!({"task":target,"source":source,"generation":generation,"result_ref":result,"reason":reason}).to_string(),now_ms()])?;
        Ok(())
    }

    pub(in crate::lean) fn retained_result_ref(&self, task: &Task) -> Result<Option<String>> {
        Ok(self
            .db()
            .query_row(
                "SELECT result_turn FROM task_resume_results WHERE task_id=?1 AND generation=?2",
                params![task.id, task.input_generation],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten())
    }
}

use super::*;

type EncodedPlanTaskRow = (
    String,
    String,
    String,
    String,
    i64,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

fn task_generation(db: &rusqlite::Connection, task: &str) -> Result<i64> {
    Ok(db
        .query_row(
            "SELECT generation FROM task_inputs WHERE task_id=?1",
            [task],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(0))
}

#[path = "plan_tasks/checks.rs"]
mod checks;

impl Store {
    /// The latest plan of the conversation other than `except` that was
    /// accepted and ran (not only proposed or sent back).
    pub fn last_plan_that_ran(
        &self,
        conversation: &str,
        except: &str,
    ) -> Result<Option<Vec<Task>>> {
        let plan_id: Option<String> = self
            .db()
            .query_row(
                "SELECT plan_id FROM tasks WHERE conversation_id=?1 AND plan_id<>?2
                   AND status NOT IN ('proposed','revised')
                 ORDER BY created_ms DESC, rowid DESC LIMIT 1",
                params![conversation, except],
                |row| row.get(0),
            )
            .optional()?;
        plan_id.map(|plan_id| self.plan(&plan_id)).transpose()
    }

    /// Compact task instructions, without opening report or verdict contents.
    pub(in crate::lean) fn plan_specs(&self, plan: &str) -> Result<Vec<Value>> {
        let db = self.db();
        let mut query = db.prepare("SELECT spec FROM tasks WHERE plan_id=?1 ORDER BY position")?;
        let records = query
            .query_map([plan], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        records
            .into_iter()
            .map(|record| serde_json::from_str(&record).map_err(Into::into))
            .collect()
    }

    /// Only the approved reports requested by this task; unrelated content is
    /// neither decoded nor needed for a fresh session's dependency handoff.
    pub(in crate::lean) fn dependency_reports(
        &self,
        plan: &str,
        dependencies: &[String],
    ) -> Result<Vec<crate::lean::tasks::DependencyReport>> {
        if dependencies.is_empty() {
            return Ok(vec![]);
        }
        let encoded = self.dependency_report_rows(plan, dependencies)?;
        encoded
            .into_iter()
            .map(|row| self.decode_dependency_report(row))
            .collect()
    }

    /// Tasks of one plan, in the orchestrator's order.
    pub fn plan(&self, plan_id: &str) -> Result<Vec<Task>> {
        let db = self.db();
        self.plan_task_rows(&db, plan_id)?
            .into_iter()
            .map(|row| self.decode_plan_task(&db, row))
            .collect()
    }

    fn dependency_report_rows(
        &self,
        plan: &str,
        dependencies: &[String],
    ) -> Result<Vec<(String, String, String)>> {
        let db = self.db();
        let mut query = db.prepare(
            "SELECT json_extract(spec,'$.id'),coalesce(json_extract(spec,'$.title'),''),report
             FROM tasks WHERE plan_id=?1 AND status='approved' AND report IS NOT NULL
             AND json_extract(spec,'$.id') IN (SELECT value FROM json_each(?2)) ORDER BY position",
        )?;
        Ok(query
            .query_map(params![plan, serde_json::to_string(dependencies)?], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .collect::<std::result::Result<_, _>>()?)
    }

    fn decode_dependency_report(
        &self,
        (id, title, report): (String, String, String),
    ) -> Result<crate::lean::tasks::DependencyReport> {
        Ok(crate::lean::tasks::DependencyReport {
            id,
            title,
            report: self.content.decode(&report)?,
        })
    }

    fn plan_task_rows(
        &self,
        db: &rusqlite::Connection,
        plan_id: &str,
    ) -> Result<Vec<EncodedPlanTaskRow>> {
        let mut statement = db.prepare(
            "SELECT id,plan_id,spec,status,round,report,verdict,base,result,feedback FROM tasks
             WHERE plan_id=?1 ORDER BY position",
        )?;
        Ok(statement
            .query_map([plan_id], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                ))
            })?
            .collect::<std::result::Result<_, _>>()?)
    }

    fn decode_plan_task(&self, db: &rusqlite::Connection, row: EncodedPlanTaskRow) -> Result<Task> {
        let (id, plan_id, spec, status, round, report, verdict, base, result, feedback) = row;
        Ok(Task {
            input_generation: task_generation(db, &id)?,
            id,
            plan_id,
            spec: serde_json::from_str(&spec)?,
            status,
            round: round as u32,
            report: self.decode_optional_text(report)?,
            verdict: self.decode_optional_json(verdict)?,
            base,
            result,
            feedback: feedback
                .map(|value| serde_json::from_str(&value))
                .transpose()?,
            freshness_token: None,
            checks: None,
        })
    }

    fn decode_optional_text(&self, text: Option<String>) -> Result<Option<String>> {
        text.map(|value| self.content.decode(&value)).transpose()
    }

    fn decode_optional_json(&self, value: Option<String>) -> Result<Option<Value>> {
        value
            .map(|encoded| self.content.decode(&encoded))
            .transpose()?
            .map(|decoded| serde_json::from_str(&decoded).map_err(Into::into))
            .transpose()
    }

    /// Records how far a verifier is with one check of a round: `checking`
    /// while it works on it, `settled` with its status and evidence. A check
    /// already settled does not go back to `checking`; a note stays unless a
    /// new one replaces it. Returns whether the row changed.
    /// The checks of one round of a task, as JSON entries.
    pub fn checks(&self, task: &str, round: u32) -> Result<Vec<Value>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT check_id,state,status,note FROM checks
             WHERE task_id=?1 AND round=?2 ORDER BY updated_ms, check_id",
        )?;
        let rows = statement.query_map(params![task, round], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, String>(0)?, "state": row.get::<_, String>(1)?,
                "status": row.get::<_, Option<String>>(2)?, "note": row.get::<_, String>(3)?,
            }))
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }
}

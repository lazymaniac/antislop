use super::*;

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
        let rows = {
            let db = self.db();
            let mut query = db.prepare(
                "SELECT json_extract(spec,'$.id'),coalesce(json_extract(spec,'$.title'),''),report
                 FROM tasks WHERE plan_id=?1 AND status='approved' AND report IS NOT NULL
                 AND json_extract(spec,'$.id') IN (SELECT value FROM json_each(?2)) ORDER BY position")?;
            query
                .query_map(params![plan, serde_json::to_string(dependencies)?], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        rows.into_iter()
            .map(|(id, title, report)| {
                Ok(crate::lean::tasks::DependencyReport {
                    id,
                    title,
                    report: self.content.decode(&report)?,
                })
            })
            .collect()
    }

    /// Tasks of one plan, in the orchestrator's order.
    pub fn plan(&self, plan_id: &str) -> Result<Vec<Task>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT id,plan_id,spec,status,round,report,verdict,base,result,feedback FROM tasks
             WHERE plan_id=?1 ORDER BY position",
        )?;
        let rows = statement.query_map([plan_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<String>>(9)?,
            ))
        })?;
        rows.map(|row| {
            let (id, plan_id, spec, status, round, report, verdict, base, result, feedback) = row?;
            Ok(Task {
                input_generation: db
                    .query_row(
                        "SELECT generation FROM task_inputs WHERE task_id=?1",
                        [&id],
                        |r| r.get(0),
                    )
                    .optional()?
                    .unwrap_or(0),
                id,
                plan_id,
                spec: serde_json::from_str(&spec)?,
                status,
                round: round as u32,
                report: report
                    .as_deref()
                    .map(|text| self.content.decode(text))
                    .transpose()?,
                verdict: verdict
                    .map(|v| {
                        self.content
                            .decode(&v)
                            .and_then(|text| Ok(serde_json::from_str(&text)?))
                    })
                    .transpose()?,
                base,
                result,
                feedback: feedback.map(|v| serde_json::from_str(&v)).transpose()?,
                freshness_token: None,
                checks: None,
            })
        })
        .collect()
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

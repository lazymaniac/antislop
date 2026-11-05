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
#[path = "plan_tasks/dependencies.rs"]
mod dependencies;

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

    /// Every task of the conversation's latest plan with approved work,
    /// other than `except`; empty when none has any.
    pub fn last_plan_with_approvals(
        &self,
        conversation: &str,
        except: Option<&str>,
    ) -> Result<Vec<Task>> {
        let plan_id: Option<String> = self
            .db()
            .query_row(
                "SELECT plan_id FROM tasks WHERE conversation_id=?1 AND plan_id<>?2
                   AND status='approved' ORDER BY created_ms DESC, rowid DESC LIMIT 1",
                params![conversation, except.unwrap_or_default()],
                |row| row.get(0),
            )
            .optional()?;
        plan_id.map_or_else(|| Ok(Vec::new()), |plan_id| self.plan(&plan_id))
    }

    /// Latest approved result per authored id across this conversation's runs.
    pub(in crate::lean) fn approved_task_history(&self, conversation: &str) -> Result<Vec<Task>> {
        let ids = {
            let db = self.db();
            let mut query = db.prepare("SELECT id FROM (SELECT id,created_ms,position,row_number() OVER(PARTITION BY json_extract(spec,'$.id') ORDER BY created_ms DESC,rowid DESC) AS rank FROM tasks WHERE conversation_id=?1 AND (status='approved' OR (status='stale' AND report IS NOT NULL))) WHERE rank=1 ORDER BY created_ms DESC,position")?;
            query
                .query_map([conversation], |r| r.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        ids.iter()
            .map(|id| {
                self.task_by_id(id)?
                    .ok_or_else(|| Error::Invalid("approved result disappeared".into()))
            })
            .collect()
    }

    /// Compact task instructions, without opening report or verdict contents.
    pub(in crate::lean) fn plan_specs(&self, plan: &str) -> Result<Vec<Value>> {
        let db = self.db();
        let mut query = db.prepare(
            "SELECT spec FROM tasks WHERE plan_id=?1 AND status<>'superseded' ORDER BY position",
        )?;
        let records = query
            .query_map([plan], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        records
            .into_iter()
            .map(|record| serde_json::from_str(&record).map_err(Into::into))
            .collect()
    }

    /// What a dependency delivered, in brief: its worker's summary, or the
    /// opening line of its report when it was approved without one (a plan
    /// stored before summaries, or an approval recorded by hand).
    /// A short account of what the task's accepted execution did.
    pub(in crate::lean) fn task_summary(&self, task: &Task) -> Result<String> {
        let summary = self
            .completed_task_output(task)
            .map(Some)
            .or_else(|error| match error {
                Error::Sql(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                error => Err(error),
            })?
            .and_then(|result| result["summary"].as_str().map(str::to_owned));
        let summary = summary.unwrap_or_else(|| {
            task.report
                .as_deref()
                .and_then(|report| report.lines().find(|line| !line.trim().is_empty()))
                .unwrap_or_default()
                .to_owned()
        });
        Ok(crate::lean::tasks::bound(&summary, 2_000))
    }

    pub(in crate::lean) fn completed_task_output(&self, task: &Task) -> Result<Value> {
        self.completed_task_result(task).map(|(_, result)| result)
    }

    pub(in crate::lean) fn completed_task_result(&self, task: &Task) -> Result<(String, Value)> {
        let conversation = self.plan_conversation(&task.plan_id)?;
        let mut current = task.clone();
        let mut visited = std::collections::HashSet::new();
        loop {
            crate::require(
                visited.insert(current.id.clone()),
                "carried result contains a cycle",
            )?;
            let text: Option<String> = self.db().query_row("SELECT r.result FROM agent_results r JOIN turns t ON t.id=r.turn_id JOIN turn_scopes s ON s.turn_id=t.id JOIN turn_inputs x ON x.turn_id=t.id WHERE s.task_id=?1 AND s.round<=?2 AND x.generation=?3 AND json_extract(r.result,'$.type')='completed' AND t.role='worker:'||?1 ORDER BY t.started_ms DESC,t.rowid DESC LIMIT 1",params![current.id,current.round,current.input_generation], |r|r.get(0)).optional()?;
            if let Some(text) = text {
                return Ok((current.id, serde_json::from_str(&text)?));
            }
            if let Some(reference) = self.retained_result_ref(&current)? {
                let (text,producer):(String,String) = self.db().query_row(
                    "SELECT r.result,s.task_id FROM agent_results r JOIN turn_scopes s ON s.turn_id=r.turn_id WHERE r.turn_id=?1",
                    [reference],|r|Ok((r.get(0)?,r.get(1)?)),
                )?;
                return Ok((producer, serde_json::from_str(&text)?));
            }
            let recorded: Option<String> = self.db().query_row("SELECT producer_task_id FROM task_result_sources WHERE task_id=?1 AND generation=?2",params![current.id,current.input_generation],|r|r.get(0)).optional()?;
            if let Some(source) = recorded.as_deref().or_else(|| {
                current
                    .feedback
                    .as_ref()
                    .and_then(|f| f["carried_from"].as_str())
            }) {
                let source = self.task_by_id(source)?.ok_or_else(|| {
                    Error::Invalid("carried result producer no longer exists".into())
                })?;
                crate::require(
                    self.plan_conversation(&source.plan_id)? == conversation,
                    "carried result belongs to another conversation",
                )?;
                current = source;
                continue;
            }
            // Legacy text is a result only when no structured worker report exists.
            // A blocked/needs-input checkpoint or an older input generation must not become
            // a synthetic completion when recovering or starting another round.
            let structured: bool = self.db().query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_results r JOIN turns t ON t.id=r.turn_id JOIN turn_scopes s ON s.turn_id=t.id WHERE s.task_id=?1 AND t.role='worker:'||?1)",
                [&current.id], |row| row.get(0),
            )?;
            if let Some(report) = current.report.filter(|_| !structured) {
                return Ok((
                    current.id,
                    serde_json::json!({"type":"completed","summary":report.lines().find(|line| !line.trim().is_empty()).unwrap_or_default(),"report":report}),
                ));
            }
            return Err(Error::Sql(rusqlite::Error::QueryReturnedNoRows));
        }
    }

    /// Tasks of one plan, in the orchestrator's order.
    pub fn plan(&self, plan_id: &str) -> Result<Vec<Task>> {
        let db = self.db();
        self.plan_task_rows(&db, plan_id)?
            .into_iter()
            .map(|row| self.decode_plan_task(&db, row))
            .collect()
    }

    fn plan_task_rows(
        &self,
        db: &rusqlite::Connection,
        plan_id: &str,
    ) -> Result<Vec<EncodedPlanTaskRow>> {
        let mut statement = db.prepare(
            "SELECT id,plan_id,spec,status,round,report,verdict,base,result,feedback FROM tasks
             WHERE plan_id=?1 AND status<>'superseded' ORDER BY position",
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

    /// Records how far a reviewer is with one check of a round: `checking`
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

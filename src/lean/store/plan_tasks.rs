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

    pub(in crate::lean) fn dependency_inputs(
        &self,
        plan: &str,
        spec: &crate::lean::tasks::TaskContract,
    ) -> Result<Vec<crate::lean::tasks::DependencyReport>> {
        let mut inputs = vec![];
        for binding in &spec.inputs {
            let task = self
                .task_in_plan_by_id(plan, &binding.task)?
                .ok_or_else(|| Error::Invalid(format!("missing input task {}", binding.task)))?;
            crate::require(task.status == "approved", "input task is not verified")?;
            let result = self.completed_task_output(&task)?;
            let value = result["outputs"][&binding.output].as_str().ok_or_else(|| {
                Error::Invalid(format!(
                    "missing output {}.{}",
                    binding.task, binding.output
                ))
            })?;
            inputs.push(crate::lean::tasks::DependencyReport {
                id: binding.name.clone(),
                title: binding.name.clone(),
                report: value.into(),
            });
        }
        let choices: Vec<String> = self.db().prepare("SELECT id FROM tasks WHERE plan_id=?1 AND json_extract(spec,'$.options_for')=?2 AND status<>'superseded'")?.query_map(params![plan,spec.id],|r|r.get(0))?.collect::<std::result::Result<_,_>>()?;
        for id in choices {
            let task = self
                .task_by_id(&id)?
                .ok_or_else(|| Error::Invalid("options task disappeared".into()))?;
            crate::require(task.status == "approved", "local options are not verified")?;
            let result = self.completed_task_output(&task)?;
            let selection = &result["selection"];
            let variant = selection["alternatives"]
                .as_array()
                .and_then(|all| all.iter().find(|a| a["id"] == selection["selected"]))
                .ok_or_else(|| Error::Invalid("local options have no selected variant".into()))?;
            inputs.push(crate::lean::tasks::DependencyReport{id:"selected_approach".into(),title:"Selected approach for this task".into(),report:serde_json::json!({"variant":variant,"rationale":selection["rationale"],"reconsider_when":selection["reconsider_when"]}).to_string()});
        }
        Ok(inputs)
    }

    pub(in crate::lean) fn completed_task_output(&self, task: &Task) -> Result<Value> {
        let text: String = self.db().query_row("SELECT r.result FROM agent_results r JOIN turns t ON t.id=r.turn_id JOIN turn_scopes s ON s.turn_id=t.id JOIN turn_inputs x ON x.turn_id=t.id WHERE s.task_id=?1 AND s.round<=?2 AND x.generation=?3 AND json_extract(r.result,'$.type')='completed' AND t.role='worker:'||?1 ORDER BY t.started_ms DESC,t.rowid DESC LIMIT 1",params![task.id,task.round,task.input_generation], |r|r.get(0))?;
        Ok(serde_json::from_str(&text)?)
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

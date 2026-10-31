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

/// How much of a dependency's summary a dependent task is shown.
const DEPENDENCY_SUMMARY_LIMIT: usize = 2_000;
/// How much of an earlier check's summary a closing task is shown.
const CHECK_SUMMARY_LIMIT: usize = 800;

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
        for dependency in &spec.depends_on {
            if let Some(input) = self.dependency_report(plan, spec, dependency)? {
                inputs.push(input);
            }
        }
        inputs.extend(self.earlier_check_reports(plan, spec)?);
        let choices: Vec<String> = self.db().prepare("SELECT id FROM tasks WHERE plan_id=?1 AND json_extract(spec,'$.options_for')=?2 AND status<>'superseded'")?.query_map(params![plan,spec.id],|r|r.get(0))?.collect::<std::result::Result<_,_>>()?;
        for id in choices {
            inputs.push(self.selected_approach_report(&id)?);
        }
        Ok(inputs)
    }

    fn dependency_report(
        &self,
        plan: &str,
        spec: &crate::lean::tasks::TaskContract,
        dependency: &str,
    ) -> Result<Option<crate::lean::tasks::DependencyReport>> {
        let task = self
            .task_in_plan_by_id(plan, dependency)?
            .ok_or_else(|| Error::Invalid(format!("missing dependency {dependency}")))?;
        if task.spec["options_for"].as_str() == Some(spec.id.as_str()) {
            return Ok(None);
        }
        crate::require(task.status == "approved", "dependency is not verified")?;
        Ok(Some(crate::lean::tasks::DependencyReport {
            id: dependency.to_owned(),
            title: task.spec["title"].as_str().unwrap_or(dependency).to_owned(),
            report: crate::lean::tasks::bound(
                &self.dependency_summary(&task)?,
                DEPENDENCY_SUMMARY_LIMIT,
            ),
        }))
    }

    /// A task that closes the work (delivery, or one following the
    /// integration gate) judges evidence the gates before it recorded only in
    /// the store, so it also gets the short summaries of the approved checks
    /// and audits it depends on through other tasks.
    fn earlier_check_reports(
        &self,
        plan: &str,
        spec: &crate::lean::tasks::TaskContract,
    ) -> Result<Vec<crate::lean::tasks::DependencyReport>> {
        use crate::config::WorkKind;
        let tasks = self.plan(plan)?;
        let contract = |task: &Task| {
            serde_json::from_value::<crate::lean::tasks::TaskContract>(task.spec.clone()).ok()
        };
        let by_id: std::collections::HashMap<String, (&Task, crate::lean::tasks::TaskContract)> =
            tasks
                .iter()
                .filter_map(|task| contract(task).map(|spec| (spec.id.clone(), (task, spec))))
                .collect();
        let closing = spec.kind == WorkKind::Delivery
            || spec.depends_on.iter().any(|id| {
                by_id
                    .get(id)
                    .is_some_and(|(_, dependency)| dependency.kind == WorkKind::Integration)
            });
        if !closing {
            return Ok(vec![]);
        }
        let mut seen: std::collections::BTreeSet<String> =
            spec.depends_on.iter().cloned().collect();
        let mut pending: Vec<String> = spec.depends_on.clone();
        let mut reports = vec![];
        while let Some(id) = pending.pop() {
            let Some((_, dependency)) = by_id.get(&id) else {
                continue;
            };
            for next in &dependency.depends_on {
                if !seen.insert(next.clone()) {
                    continue;
                }
                pending.push(next.clone());
                let Some((task, check)) = by_id.get(next) else {
                    continue;
                };
                let evidence = matches!(
                    check.kind,
                    WorkKind::StaticAnalysis
                        | WorkKind::Testing
                        | WorkKind::Security
                        | WorkKind::Performance
                        | WorkKind::Integration
                );
                if evidence && check.options_for.is_none() && task.status == "approved" {
                    reports.push(crate::lean::tasks::DependencyReport {
                        id: check.id.clone(),
                        title: format!("{} (earlier check)", check.title),
                        report: crate::lean::tasks::bound(
                            &self.dependency_summary(task)?,
                            CHECK_SUMMARY_LIMIT,
                        ),
                    });
                }
            }
        }
        reports.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(reports)
    }

    fn selected_approach_report(
        &self,
        task_id: &str,
    ) -> Result<crate::lean::tasks::DependencyReport> {
        let task = self
            .task_by_id(task_id)?
            .ok_or_else(|| Error::Invalid("options task disappeared".into()))?;
        crate::require(task.status == "approved", "local options are not verified")?;
        let result = self.completed_task_output(&task)?;
        let selection = &result["selection"];
        let variant = selection["alternatives"]
            .as_array()
            .and_then(|all| all.iter().find(|item| item["id"] == selection["selected"]))
            .ok_or_else(|| Error::Invalid("local options have no selected variant".into()))?;
        Ok(crate::lean::tasks::DependencyReport {
            id: "selected_approach".into(),
            title: "Selected approach for this task".into(),
            report: serde_json::json!({"variant":variant,"rationale":selection["rationale"]})
                .to_string(),
        })
    }

    /// What a dependency delivered, in brief: its worker's summary, or the
    /// opening line of its report when it was approved without one (a plan
    /// stored before summaries, or an approval recorded by hand).
    fn dependency_summary(&self, task: &Task) -> Result<String> {
        let summary = self
            .completed_task_output(task)
            .map(Some)
            .or_else(|error| match error {
                Error::Sql(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                error => Err(error),
            })?
            .and_then(|result| result["summary"].as_str().map(str::to_owned));
        Ok(summary.unwrap_or_else(|| {
            task.report
                .as_deref()
                .and_then(|report| report.lines().find(|line| !line.trim().is_empty()))
                .unwrap_or_default()
                .to_owned()
        }))
    }

    pub(in crate::lean) fn completed_task_output(&self, task: &Task) -> Result<Value> {
        // A kept result was produced by the turns of the task it came from.
        let carried = task
            .feedback
            .as_ref()
            .and_then(|f| f["carried_from"].as_str());
        if let Some(source) = carried.and_then(|id| self.task_by_id(id).transpose()) {
            return self.completed_task_output(&source?);
        }
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

//! Turn-bound access to approved results and their same-input worker rounds.
use super::*;

impl Store {
    pub(in crate::lean) fn dependency_plan(
        &self,
        conversation: &str,
        role: &str,
        turn: &str,
    ) -> Result<String> {
        let db = self.db();
        let plan: Option<String> = db.query_row(
            "SELECT coalesce(t.plan_id,
                (SELECT p.id FROM plans p JOIN plan_activations a ON a.plan_id=p.id WHERE p.conversation_id=t.conversation_id ORDER BY p.created_ms DESC,p.rowid DESC LIMIT 1),
                (SELECT p.id FROM plans p WHERE p.conversation_id=t.conversation_id ORDER BY p.created_ms DESC,p.rowid DESC LIMIT 1))
             FROM turns t WHERE t.id=?1 AND t.conversation_id=?2 AND t.role=?3 AND t.status='running'",
            params![turn,conversation,role], |r| r.get(0))?;
        let plan = plan.ok_or_else(|| Error::Invalid("no plan bound to this turn".into()))?;
        crate::require(
            db.query_row(
                "SELECT EXISTS(SELECT 1 FROM plans WHERE id=?1 AND conversation_id=?2)",
                params![plan, conversation],
                |r| r.get::<_, bool>(0),
            )?,
            "turn plan belongs to another conversation",
        )?;
        // Legacy/unbound turns freeze the same accepted-plan choice on first use.
        db.execute(
            "UPDATE turns SET plan_id=?1 WHERE id=?2 AND plan_id IS NULL",
            params![plan, turn],
        )?;
        Ok(plan)
    }

    /// Compliance roles can read approved text results of the exact run bound at launch.
    /// They cannot inherit access from a newer draft or a different conversation.
    pub(in crate::lean) fn audit_dependency_scope(
        &self,
        conversation: &str,
        role: &str,
        turn: &str,
    ) -> Result<(Vec<Task>, i64)> {
        let bound: Option<(String, i64)> = self.db().query_row(
            "SELECT p.run_id,r.spec_revision FROM turns t JOIN plans q ON q.id=t.plan_id JOIN plan_runs p ON p.plan_id=q.id JOIN runs r ON r.id=p.run_id WHERE t.id=?1 AND t.conversation_id=?2 AND q.conversation_id=?2 AND r.conversation_id=?2 AND t.role=?3 AND t.status='running'",
            params![turn, conversation, role], |row| Ok((row.get(0)?, row.get(1)?))
        ).optional()?;
        let (run, revision) = bound.ok_or_else(|| {
            Error::Invalid("audit turn has no plan bound to this conversation and run".into())
        })?;
        let tasks = self
            .run_tasks(conversation, &run)?
            .into_iter()
            .filter(|task| task.status == "approved")
            .collect();
        Ok((tasks, revision))
    }

    /// Immutable worker reports of this scoped producer, including older inputs.
    /// Historical reports are discoverable, never substituted for current evidence.
    pub(in crate::lean) fn dependency_reports(&self, task: &Task) -> Result<DependencyReports> {
        let current = match self.completed_task_result(task) {
            Ok(value) => Some(value),
            Err(Error::Sql(rusqlite::Error::QueryReturnedNoRows)) => None,
            Err(error) => return Err(error),
        };
        let producer = current.as_ref().map_or(task.id.as_str(), |(id, _)| id);
        let source = self
            .task_by_id(producer)?
            .ok_or_else(|| Error::Invalid("result producer disappeared".into()))?;
        let conversation = self.plan_conversation(&task.plan_id)?;
        let mut pending = vec![task.clone()];
        let mut visited = std::collections::HashSet::new();
        let mut reports = Vec::new();
        while let Some(ancestor) = pending.pop() {
            if !visited.insert(ancestor.id.clone()) {
                continue;
            }
            crate::require(
                self.plan_conversation(&ancestor.plan_id)? == conversation,
                "historical result belongs to another conversation",
            )?;
            reports.extend(self.worker_report_references(&ancestor, &source)?);
            let sources = {
                let db = self.db();
                let mut query = db.prepare("SELECT producer_task_id FROM task_result_sources WHERE task_id=?1 AND generation<=?2 UNION SELECT source_task FROM task_resume_results WHERE task_id=?1 AND generation<=?2 AND source_task<>?1")?;
                query
                    .query_map(params![ancestor.id, ancestor.input_generation], |row| {
                        row.get::<_, String>(0)
                    })?
                    .collect::<std::result::Result<Vec<_>, _>>()?
            };
            for id in sources {
                pending.push(self.task_by_id(&id)?.ok_or_else(|| {
                    Error::Invalid("historical result producer disappeared".into())
                })?);
            }
        }
        reports.sort_by_key(|r| {
            (
                !r.current_inputs,
                std::cmp::Reverse(r.input_generation),
                std::cmp::Reverse(r.round),
            )
        });
        let retained = self.retained_result_ref(task)?;
        Ok(DependencyReports {
            producer: source,
            current: current.map(|(_, output)| output).filter(|_| {
                retained.is_some()
                    || reports.is_empty()
                    || reports
                        .iter()
                        .any(|r| r.current_inputs && r.result_type == "completed")
            }),
            retained,
            reports,
        })
    }

    fn worker_report_references(
        &self,
        ancestor: &Task,
        current: &Task,
    ) -> Result<Vec<ReportReference>> {
        let db = self.db();
        let mut query = db.prepare("SELECT t.id,s.round,x.generation,length(cast(json_extract(r.result,'$.report') as blob)),json_extract(r.result,'$.type') FROM agent_results r JOIN turns t ON t.id=r.turn_id JOIN turn_scopes s ON s.turn_id=t.id JOIN turn_inputs x ON x.turn_id=t.id WHERE s.task_id=?1 AND s.round<=?2 AND x.generation<=?3 AND t.status='done' AND t.role='worker:'||?1 AND json_extract(r.result,'$.type') IN ('completed','blocked') ORDER BY x.generation DESC,s.round DESC,t.started_ms DESC,t.rowid DESC")?;
        Ok(query
            .query_map(
                params![ancestor.id, ancestor.round, ancestor.input_generation],
                |row| {
                    let generation = row.get(2)?;
                    Ok(ReportReference {
                        result_ref: row.get(0)?,
                        round: row.get(1)?,
                        input_generation: generation,
                        bytes: row.get::<_, i64>(3)? as u64,
                        result_type: row.get(4)?,
                        producer_task: ancestor.id.clone(),
                        current_inputs: ancestor.id == current.id
                            && generation == current.input_generation,
                    })
                },
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub(in crate::lean) fn referenced_worker_report(
        &self,
        reference: &ReportReference,
    ) -> Result<Value> {
        let text: String = self.db().query_row(
            "SELECT result FROM agent_results WHERE turn_id=?1",
            [&reference.result_ref],
            |row| row.get(0),
        )?;
        Ok(serde_json::from_str(&text)?)
    }
}

#[derive(Debug, Clone, Serialize)]
pub(in crate::lean) struct ReportReference {
    pub result_ref: String,
    pub round: u32,
    pub input_generation: i64,
    pub bytes: u64,
    pub result_type: String,
    pub producer_task: String,
    pub current_inputs: bool,
}

pub(in crate::lean) struct DependencyReports {
    pub retained: Option<String>,
    pub producer: Task,
    pub current: Option<Value>,
    pub reports: Vec<ReportReference>,
}

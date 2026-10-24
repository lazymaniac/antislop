use super::*;

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
    pub fn set_check(
        &self,
        task: &str,
        round: u32,
        check: &str,
        state: &str,
        status: Option<&str>,
        note: &str,
    ) -> Result<bool> {
        let db = self.db();
        let before: Option<(String, Option<String>, String)> = db
            .query_row(
                "SELECT state,status,note FROM checks WHERE task_id=?1 AND round=?2 AND check_id=?3",
                params![task, round, check],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let (state, status, note) = match before {
            Some((old_state, ..)) if old_state == "settled" && state == "checking" => {
                return Ok(false);
            }
            Some((_, old_status, old_note)) => (
                state,
                status.map(str::to_owned).or(old_status),
                if note.is_empty() {
                    old_note
                } else {
                    note.to_owned()
                },
            ),
            None => (state, status.map(str::to_owned), note.to_owned()),
        };
        let changed = db.execute(
            "INSERT INTO checks(task_id,round,check_id,state,status,note,updated_ms)
             VALUES(?1,?2,?3,?4,?5,?6,?7)
             ON CONFLICT(task_id,round,check_id) DO UPDATE SET
               state=?4,status=?5,note=?6,updated_ms=?7
             WHERE state IS NOT ?4 OR status IS NOT ?5 OR note IS NOT ?6",
            params![task, round, check, state, status, note, now_ms()],
        )?;
        Ok(changed > 0)
    }

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

    pub fn update_task(&self, task: &Task) -> Result<()> {
        let stored_report = task
            .report
            .as_deref()
            .map(|text| self.content.encode(text))
            .transpose()?;
        let stored_verdict = task
            .verdict
            .as_ref()
            .map(|value| self.content.encode(&value.to_string()))
            .transpose()?;
        let mut snapshot = serde_json::to_value(task)?;
        for (field, encoded) in [("report", &stored_report), ("verdict", &stored_verdict)] {
            if let Some(encoded) = encoded {
                let reference: Value = serde_json::from_str(
                    encoded
                        .strip_prefix("\0antislop-content:")
                        .expect("encoded content"),
                )?;
                snapshot[field] = serde_json::json!({"session_content":reference});
            }
        }
        // A requeue may clear the task's mutable result before its next round
        // starts. Keep the last durable report, verdict and Git range in that
        // round's history, including when setup fails before another turn.
        let previous: Option<String> = self
            .db()
            .query_row(
                "SELECT content FROM task_rounds WHERE task_id=?1 AND round=?2",
                params![task.id, task.round],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(previous) = previous {
            let reference = serde_json::from_str(&previous)?;
            let previous: Value = serde_json::from_str(&self.content.read(&reference)?)?;
            for field in ["report", "verdict"] {
                if snapshot[field].is_null() {
                    snapshot[field] = previous[field].clone();
                }
            }
            if snapshot["result"].is_null() && !previous["result"].is_null() {
                snapshot["base"] = previous["base"].clone();
                snapshot["result"] = previous["result"].clone();
            }
        }
        let content = self.content.write(&serde_json::to_string(&snapshot)?)?;
        let mut db = self.db();
        let transaction = db.transaction()?;
        transaction.execute("UPDATE task_rounds SET status='continued' WHERE task_id=?1 AND round=(SELECT round FROM tasks WHERE id=?1) AND round<?2 AND status IN ('working','verifying','queued')",params![task.id,task.round])?;
        let changed = transaction.execute(
            "UPDATE tasks SET status=?2,round=?3,report=?4,verdict=?5,base=?6,result=?7,feedback=?8 WHERE id=?1 AND coalesce((SELECT generation FROM task_inputs WHERE task_id=?1),0)=?9",
            params![
                task.id,
                task.status,
                task.round as i64,
                stored_report,
                stored_verdict,
                task.base,
                task.result,
                task.feedback.as_ref().map(Value::to_string),
                task.input_generation
            ],
        )?;
        crate::require(
            changed == 1,
            "stale task generation; result retained in turn history",
        )?;
        transaction.execute("INSERT INTO task_rounds(task_id,round,status,content) VALUES(?1,?2,?3,?4)
            ON CONFLICT(task_id,round) DO UPDATE SET status=excluded.status,content=excluded.content",
            params![task.id, task.round, task.status, serde_json::to_string(&content)?])?;
        transaction.commit()?;
        Ok(())
    }
}

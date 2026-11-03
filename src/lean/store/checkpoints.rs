//! Explicit recovery of unintegrated code from older plans, never approval.
use super::*;

impl Store {
    pub(in crate::lean) fn unfinished_checkpoints(
        &self,
        conversation: &str,
        plan: &str,
    ) -> Result<Vec<Value>> {
        let db = self.db();
        let mut query = db.prepare(
            "SELECT id FROM tasks WHERE conversation_id=?1 AND plan_id<>?2
             AND status IN ('coordinating','failed','cancelled','needs_user','waiting','stale')
             AND base IS NOT NULL AND result IS NOT NULL AND base<>result
             AND json_extract(spec,'$.writes')=1
             AND coalesce(json_extract(spec,'$.change_kind'),'code')='code'
             AND NOT EXISTS(SELECT 1 FROM task_resume_results r JOIN tasks t ON t.id=r.task_id
                 WHERE r.source_task=tasks.id AND t.id<>tasks.id AND t.status IN ('queued','working','reviewing','approved'))
             ORDER BY created_ms DESC,rowid DESC LIMIT 32",
        )?;
        let ids = query
            .query_map(params![conversation, plan], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ids.iter()
            .map(|id| checkpoint_source(&db, conversation, plan, id).map(|(_, value)| value))
            .collect()
    }

    pub(in crate::lean) fn revision_checkpoints(
        &self,
        conversation: &str,
        revision: &PlanRevision,
    ) -> Result<Vec<Value>> {
        let mut sources = Vec::new();
        for (target, source) in &revision.replacements {
            if self.task_in_plan_by_id(&revision.plan, source)?.is_some() {
                continue;
            }
            let (_, mut value) =
                checkpoint_source(&self.db(), conversation, &revision.plan, source)?;
            value["target"] = target.as_str().into();
            sources.push(value);
        }
        Ok(sources)
    }

    pub(in crate::lean) fn inherit_revision_checkpoint(
        tx: &rusqlite::Transaction<'_>,
        conversation: &str,
        plan: &str,
        source: &str,
        target: &str,
    ) -> Result<()> {
        let (generation, _) = checkpoint_source(tx, conversation, plan, source)?;
        Self::retain_work_from_ids(
            tx,
            source,
            generation,
            target,
            0,
            "recover unfinished code from an earlier plan",
        )
    }
}

fn checkpoint_source(
    db: &rusqlite::Connection,
    conversation: &str,
    plan: &str,
    source: &str,
) -> Result<(i64, Value)> {
    let (source_plan, status, spec, base, result, generation): (
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        i64,
    ) = db
        .query_row(
            "SELECT plan_id,status,spec,base,result,
         coalesce((SELECT generation FROM task_inputs WHERE task_id=tasks.id),0)
         FROM tasks WHERE id=?1 AND conversation_id=?2",
            params![source, conversation],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| {
            Error::Invalid("checkpoint source is unavailable in this conversation".into())
        })?;
    crate::require(
        source_plan != plan,
        "use a retired local task name for a current-plan replacement",
    )?;
    crate::require(
        matches!(
            status.as_str(),
            "coordinating" | "failed" | "cancelled" | "needs_user" | "waiting" | "stale"
        ),
        "checkpoint source must be unfinished; integrated or active work cannot be adopted",
    )?;
    let continued: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM task_resume_results r JOIN tasks t ON t.id=r.task_id
         WHERE r.source_task=?1 AND t.id<>?1 AND t.status IN ('queued','working','reviewing','approved'))",
        [source], |r| r.get(0),
    )?;
    crate::require(
        !continued,
        "checkpoint source already has an active or approved successor; continue that task instead",
    )?;
    let spec: crate::lean::tasks::TaskContract = serde_json::from_str(&spec)?;
    crate::require(
        spec.writes && spec.changes_code(),
        "checkpoint source must contain code work",
    )?;
    let (base, result) = base
        .zip(result)
        .filter(|(a, b)| a != b)
        .ok_or_else(|| Error::Invalid("checkpoint source has no unintegrated change".into()))?;
    Ok((
        generation,
        serde_json::json!({
            "source_task":source,"task":spec.id,"plan":source_plan,"status":status,
            "title":spec.title,"base":base,"result":result,"input_generation":generation,
        }),
    ))
}

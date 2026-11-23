//! Explicit `keep` binds a proposal to an already approved result. Unlike an
//! automatic same-name carry, it accepts that historical result in the new scope.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

use carry::Row;

impl Store {
    pub(in crate::lean) fn retain_plan_tasks(
        &self,
        conversation: &str,
        plan: &str,
        keep: &[String],
    ) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        for name in keep {
            let source: Option<String> = tx.query_row(
                "SELECT id FROM tasks WHERE conversation_id=?1 AND plan_id<>?2 AND CASE WHEN json_valid(spec) THEN json_extract(spec,'$.id') END=?3 AND (status IN ('approved','stale') OR (status IN ('coordinating','failed','cancelled','needs_user','waiting','queued') AND (report IS NOT NULL OR result IS NOT NULL))) AND (status='approved' OR (status='stale' AND report IS NOT NULL)) ORDER BY created_ms DESC,rowid DESC LIMIT 1",
                params![conversation, plan, name], |r| r.get(0),
            ).optional()?;
            let Some(source) = source else { continue };
            let Some(approval) = approval(&tx, &source)? else {
                continue;
            };
            tx.execute(
                "UPDATE tasks SET feedback=json_set(coalesce(feedback,'{}'),'$.kept_approval',json(?4)) WHERE conversation_id=?1 AND plan_id=?2 AND status='proposed' AND CASE WHEN json_valid(spec) THEN json_extract(spec,'$.id') END=?3 AND status<>'superseded'",
                params![conversation, plan, name, approval.to_string()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Compact preview only; reports and evidence stay with their result producer.
    pub(in crate::lean) fn retained_plan_preview(
        &self,
        conversation: &str,
        plan: &str,
    ) -> Result<Value> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let captured = carry::capture(&tx, conversation, plan)?;
        let preview = captured.retained_preview();
        tx.commit()?;
        Ok(preview)
    }
}

fn approval(tx: &rusqlite::Transaction<'_>, source: &str) -> Result<Option<Value>> {
    let encoded: Option<String> = tx.query_row(
        "SELECT json_object('source',t.id,'generation',coalesce(i.generation,0),'version',coalesce((SELECT max(version) FROM task_contracts WHERE task_id=t.id),0),'round',t.round) FROM tasks t LEFT JOIN task_inputs i ON i.task_id=t.id WHERE t.id=?1 AND t.status='approved'",
        [source], |r| r.get(0),
    ).optional()?;
    encoded
        .map(|s| {
            let mut pin: Value = serde_json::from_str(&s)?;
            pin["result_turn"] = serde_json::to_value(carry::result_turn(tx, source)?)?;
            Ok(pin)
        })
        .transpose()
}

/// Generated edges are runtime prerequisites for fresh execution. A kept writer
/// already satisfied them; its authored contract and authored dependencies remain.
fn authored_contract(
    tx: &rusqlite::Transaction<'_>,
    row: &Row,
) -> Result<crate::lean::tasks::TaskContract> {
    let mut spec: crate::lean::tasks::TaskContract = serde_json::from_value(row.1.clone())?;
    let mut authored = Vec::new();
    for dependency in &spec.depends_on {
        let generated: bool = tx.query_row(
            "SELECT json_extract(spec,'$.quality_gate')=1 OR json_extract(spec,'$.options_for') IS NOT NULL FROM tasks WHERE plan_id=(SELECT plan_id FROM tasks WHERE id=?1) AND CASE WHEN json_valid(spec) THEN json_extract(spec,'$.id') END=?2 AND status<>'superseded'",
            params![row.0, dependency], |r| Ok(r.get::<_, Option<bool>>(0)?.unwrap_or(false)),
        ).optional()?.unwrap_or(false);
        if !generated {
            authored.push(dependency.clone());
        }
    }
    spec.depends_on = authored;
    spec.depends_on.sort();
    Ok(spec)
}

fn dependency_result(
    tx: &rusqlite::Transaction<'_>,
    owner: &str,
    name: &str,
) -> Result<Option<(String, i64)>> {
    let id: Option<String> = tx.query_row(
        "SELECT id FROM tasks WHERE plan_id=(SELECT plan_id FROM tasks WHERE id=?1) AND CASE WHEN json_valid(spec) THEN json_extract(spec,'$.id') END=?2 AND status<>'superseded' AND status='approved'",
        params![owner, name], |r| r.get(0),
    ).optional()?;
    id.map(|id| result_identity(tx, &id))
        .transpose()
        .map(Option::flatten)
}

fn result_identity(tx: &rusqlite::Transaction<'_>, task: &str) -> Result<Option<(String, i64)>> {
    Ok(tx.query_row(
        "SELECT coalesce(s.producer_task_id,json_extract(t.feedback,'$.carried_from'),t.id),t.round FROM tasks t LEFT JOIN task_result_sources s ON s.task_id=t.id AND s.generation=coalesce((SELECT generation FROM task_inputs WHERE task_id=t.id),0) WHERE t.id=?1 AND t.status='approved'",
        [task], |r| Ok((r.get(0)?, r.get(1)?)),
    ).optional()?)
}

pub(super) fn valid_sources(
    tx: &rusqlite::Transaction<'_>,
    targets: &[Row],
    sources: &BTreeMap<String, Row>,
) -> Result<BTreeSet<String>> {
    let mut candidates = BTreeMap::new();
    for target in targets {
        let Some(pin) = target.2.as_ref().and_then(|v| v.get("kept_approval")) else {
            continue;
        };
        let name = target.1["id"].as_str().unwrap_or_default();
        let Some(source) = sources.get(name) else {
            continue;
        };
        if source.1["quality_gate"] == true
            || source.1["options_for"].is_string()
            || approval(tx, &source.0)?.as_ref() != Some(pin)
        {
            continue;
        }
        let spec = authored_contract(tx, target)?;
        if spec == authored_contract(tx, source)? {
            candidates.insert(name.to_owned(), (target, source, spec));
        }
    }
    let mut valid = BTreeSet::new();
    loop {
        let before = valid.len();
        for (name, (_, source, spec)) in &candidates {
            if valid.contains(name) {
                continue;
            }
            let mut dependencies_kept = true;
            for dependency in &spec.depends_on {
                let Some((_, producer, _)) = candidates
                    .get(dependency)
                    .filter(|_| valid.contains(dependency))
                else {
                    dependencies_kept = false;
                    break;
                };
                let previous = dependency_result(tx, &source.0, dependency)?;
                let current = result_identity(tx, &producer.0)?;
                dependencies_kept &= previous.is_some() && previous == current;
            }
            if dependencies_kept {
                valid.insert(name.clone());
            }
        }
        if valid.len() == before {
            break;
        }
    }
    Ok(candidates
        .into_iter()
        .filter(|(name, _)| valid.contains(name))
        .map(|(_, (target, _, _))| target.0.clone())
        .collect())
}

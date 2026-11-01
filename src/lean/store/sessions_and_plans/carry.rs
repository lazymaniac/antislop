//! Approved work a new plan keeps instead of repeating.
use super::*;

type Row = (String, Value, Option<String>);

/// Each task of `plan_id` whose contract matches an approved task of the
/// conversation's approved history takes over its original report. The same
/// requirements retain approval; new requirements queue a verifier-only review. A writer's
/// change is already in the project, so no commits come along.
/// `feedback.carried_from` names the task whose turns produced the result.
/// Returns what to tell the user, if anything was matched by id.
pub(super) fn carry_approved(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    plan_id: &str,
) -> Result<Option<String>> {
    let Some((_source, same_requirements)) = source_plan(tx, conversation, plan_id)? else {
        return Ok(None);
    };
    let approved = rows(
        tx,
        "SELECT id,spec,feedback FROM tasks WHERE conversation_id=?1 AND status='approved' ORDER BY created_ms DESC,rowid DESC",
        conversation,
    )?;
    let project: String = tx.query_row(
        "SELECT project FROM conversations WHERE id=?1",
        [conversation],
        |r| r.get(0),
    )?;
    let (kept, repeated) =
        carry_plan_tasks(tx, plan_id, &approved, std::path::Path::new(&project))?;
    let why = if same_requirements {
        "their contracts changed since approval"
    } else {
        "the requirements changed since their approval"
    };
    Ok(carry_notice(&kept, &repeated, why))
}

fn carry_plan_tasks(
    tx: &rusqlite::Transaction<'_>,
    plan_id: &str,
    approved: &[Row],
    project: &std::path::Path,
) -> Result<(Vec<String>, Vec<String>)> {
    let (mut kept, mut repeated) = (vec![], vec![]);
    let mut current_source = None;
    for (id, spec, _) in rows(
        tx,
        "SELECT id,spec,NULL FROM tasks WHERE plan_id=?1 ORDER BY position",
        plan_id,
    )? {
        let name = spec["id"].as_str().unwrap_or_default().to_owned();
        let same = approved.iter().find(|old| same_contract(&old.1, &spec));
        let same = match same {
            Some(old)
                if source_is_current(tx, old, project, &mut current_source)?
                    && dependencies_are_current(tx, plan_id, old)? =>
            {
                Some(old)
            }
            _ => None,
        };
        let same_requirements = if let Some(old) = same {
            tx.query_row("SELECT coalesce(a.run_id=b.run_id,0) FROM tasks t LEFT JOIN plan_runs a ON a.plan_id=t.plan_id LEFT JOIN plan_runs b ON b.plan_id=?2 WHERE t.id=?1",params![old.0,plan_id],|r|r.get::<_,bool>(0))?
        } else {
            false
        };
        let reusable = match same {
            Some(old) if same_requirements => Some(old),
            Some(old)
                if tx.query_row(
                    "SELECT report IS NOT NULL FROM tasks WHERE id=?1",
                    [&old.0],
                    |r| r.get::<_, bool>(0),
                )? =>
            {
                Some(old)
            }
            _ => None,
        };
        if let Some(old) = reusable {
            carry(tx, &id, old, same_requirements)?;
            kept.push(name);
        } else if approved.iter().any(|old| old.1["id"] == spec["id"]) {
            repeated.push(name);
        }
    }
    Ok((kept, repeated))
}

fn dependencies_are_current(
    tx: &rusqlite::Transaction<'_>,
    plan: &str,
    source: &Row,
) -> Result<bool> {
    let spec: crate::lean::tasks::TaskContract = serde_json::from_value(source.1.clone())?;
    let old_plan: String =
        tx.query_row("SELECT plan_id FROM tasks WHERE id=?1", [&source.0], |r| {
            r.get(0)
        })?;
    for dependency in &spec.depends_on {
        let origin = |plan: &str| -> Result<Option<String>> {
            Ok(tx.query_row("SELECT CASE WHEN EXISTS(SELECT 1 FROM agent_results r JOIN turns u ON u.id=r.turn_id JOIN turn_scopes x ON x.turn_id=u.id JOIN turn_inputs i ON i.turn_id=u.id WHERE x.task_id=t.id AND u.role='worker:'||t.id AND x.round<=t.round AND i.generation=coalesce((SELECT generation FROM task_inputs WHERE task_id=t.id),0) AND json_extract(r.result,'$.type')='completed') THEN t.id ELSE coalesce(s.producer_task_id,json_extract(t.feedback,'$.carried_from'),t.id) END FROM tasks t LEFT JOIN task_result_sources s ON s.task_id=t.id AND s.generation=coalesce((SELECT generation FROM task_inputs WHERE task_id=t.id),0) WHERE t.plan_id=?1 AND json_extract(t.spec,'$.id')=?2 AND t.status<>'superseded'",params![plan,dependency],|r|r.get(0)).optional()?)
        };
        let before = origin(&old_plan)?;
        let after = origin(plan)?;
        if before.is_none() || before != after {
            return Ok(false);
        }
    }
    Ok(true)
}

fn source_is_current(
    tx: &rusqlite::Transaction<'_>,
    source: &Row,
    project: &std::path::Path,
    snapshot: &mut Option<String>,
) -> Result<bool> {
    let revision: Option<String> = tx.query_row("SELECT revision FROM task_source_revisions WHERE task_id=?1 AND generation=coalesce((SELECT generation FROM task_inputs WHERE task_id=?1),0)",[&source.0],|r|r.get(0)).optional()?;
    let Some(revision) = revision else {
        return Ok(true);
    };
    if !crate::lean::git::is_repository(project) {
        return Ok(false);
    }
    let spec: crate::lean::tasks::TaskContract = serde_json::from_value(source.1.clone())?;
    let current = if spec.change_kind() == crate::lean::tasks::ChangeKind::Artifact {
        crate::lean::git::snapshot_with_artifacts(
            project,
            &spec.outputs,
            "Artifact inputs before reusing approved work",
        )?
    } else {
        match snapshot {
            Some(current) => current.clone(),
            None => {
                let current = crate::lean::git::snapshot(
                    project,
                    "Source inputs before reusing approved work",
                )?;
                *snapshot = Some(current.clone());
                current
            }
        }
    };
    let paths = crate::lean::git::changed_files(project, &revision, &current)?;
    Ok(!paths
        .iter()
        .any(|path| crate::lean::tasks::scopes_overlap(&spec.scope, std::slice::from_ref(path))))
}

/// The last plan with approved work, and whether it answered the same
/// requirements; only then does its work count for this plan.
fn source_plan(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    plan_id: &str,
) -> Result<Option<(String, bool)>> {
    Ok(tx
        .query_row(
            "SELECT t.plan_id, coalesce(a.run_id=b.run_id, 0) FROM tasks t
               LEFT JOIN plan_runs a ON a.plan_id=t.plan_id
               LEFT JOIN plan_runs b ON b.plan_id=?2
             WHERE t.conversation_id=?1 AND t.plan_id<>?2 AND t.status='approved'
             ORDER BY t.created_ms DESC, t.rowid DESC LIMIT 1",
            params![conversation, plan_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?)
}

fn rows(tx: &rusqlite::Transaction<'_>, sql: &str, plan: &str) -> Result<Vec<Row>> {
    let mut statement = tx.prepare(sql)?;
    let encoded = statement
        .query_map([plan], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    encoded
        .into_iter()
        .map(|(id, spec, feedback)| Ok((id, serde_json::from_str(&spec)?, feedback)))
        .collect()
}

/// Dependency order is the planner's; everything else must be identical.
fn same_contract(old: &Value, new: &Value) -> bool {
    let normal = |spec: &Value| {
        let mut spec = spec.clone();
        if let Some(object) = spec.as_object_mut() {
            for field in ["requirement_ids", "criterion_ids", "covers"] {
                object.remove(field);
            }
        }
        if let Some(dependencies) = spec["depends_on"].as_array_mut() {
            dependencies.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
        }
        spec
    };
    normal(old) == normal(new)
}

fn carry(
    tx: &rusqlite::Transaction<'_>,
    id: &str,
    (source, _, feedback): &Row,
    same_requirements: bool,
) -> Result<()> {
    let mut feedback = feedback
        .as_deref()
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    // A result kept twice still names the task that produced it.
    if feedback["carried_from"].as_str().is_none() {
        feedback["carried_from"] = source.as_str().into();
    }
    if !same_requirements {
        feedback["resume_role"] = "verifier".into();
        feedback["compatibility_review"] = true.into();
    }
    tx.execute("INSERT OR IGNORE INTO task_result_sources(task_id,generation,producer_task_id) VALUES(?1,coalesce((SELECT generation FROM task_inputs WHERE task_id=?1),0),?2)", params![id, feedback["carried_from"].as_str().unwrap_or(source)])?;
    tx.execute("INSERT OR IGNORE INTO task_source_revisions(task_id,generation,revision) SELECT ?1,coalesce((SELECT generation FROM task_inputs WHERE task_id=?1),0),revision FROM task_source_revisions WHERE task_id=?2 AND generation=coalesce((SELECT generation FROM task_inputs WHERE task_id=?2),0)",params![id,source])?;
    tx.execute(
        "UPDATE tasks SET status=?4,
           round=(SELECT round FROM tasks WHERE id=?2),
           report=(SELECT report FROM tasks WHERE id=?2),
           verdict=CASE WHEN ?4='approved' THEN (SELECT verdict FROM tasks WHERE id=?2) ELSE NULL END,
           feedback=?3, base=NULL, result=NULL
         WHERE id=?1",
        params![id, source, feedback.to_string(), if same_requirements { "approved" } else { "queued" }],
    )?;
    Ok(())
}

fn carry_notice(kept: &[String], repeated: &[String], why: &str) -> Option<String> {
    let mut parts = vec![];
    if !kept.is_empty() {
        parts.push(format!(
            "kept the approved results of {} from the previous plan",
            kept.join(", ")
        ));
    }
    if !repeated.is_empty() {
        parts.push(format!("{} run again: {why}", repeated.join(", ")));
    }
    (!parts.is_empty()).then(|| parts.join("; "))
}

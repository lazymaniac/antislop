//! Approved work a new plan keeps instead of repeating.
use super::*;

type Row = (String, Value, Option<String>);

/// Each task of `plan_id` whose contract matches an approved task of the
/// conversation's last plan with approved work, under the same requirements,
/// takes over that approval with its round, report and verdict. A writer's
/// change is already in the project, so no commits come along.
/// `feedback.carried_from` names the task whose turns produced the result.
/// Returns what to tell the user, if anything was matched by id.
pub(super) fn carry_approved(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    plan_id: &str,
) -> Result<Option<String>> {
    let Some((source, same_requirements)) = source_plan(tx, conversation, plan_id)? else {
        return Ok(None);
    };
    let approved = rows(
        tx,
        "SELECT id,spec,feedback FROM tasks WHERE plan_id=?1 AND status='approved'",
        &source,
    )?;
    let (mut kept, mut repeated) = (vec![], vec![]);
    for (id, spec, _) in rows(
        tx,
        "SELECT id,spec,NULL FROM tasks WHERE plan_id=?1",
        plan_id,
    )? {
        let name = spec["id"].as_str().unwrap_or_default().to_owned();
        let same = approved.iter().find(|old| same_contract(&old.1, &spec));
        if let Some(old) = same.filter(|_| same_requirements) {
            carry(tx, &id, old)?;
            kept.push(name);
        } else if approved.iter().any(|old| old.1["id"] == spec["id"]) {
            repeated.push(name);
        }
    }
    let why = if same_requirements {
        "their contracts changed since approval"
    } else {
        "the requirements changed since their approval"
    };
    Ok(carry_notice(&kept, &repeated, why))
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
        if let Some(dependencies) = spec["depends_on"].as_array_mut() {
            dependencies.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
        }
        spec
    };
    normal(old) == normal(new)
}

fn carry(tx: &rusqlite::Transaction<'_>, id: &str, (source, _, feedback): &Row) -> Result<()> {
    let mut feedback = feedback
        .as_deref()
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    // A result kept twice still names the task that produced it.
    if feedback["carried_from"].as_str().is_none() {
        feedback["carried_from"] = source.as_str().into();
    }
    tx.execute(
        "UPDATE tasks SET status='approved',
           round=(SELECT round FROM tasks WHERE id=?2),
           report=(SELECT report FROM tasks WHERE id=?2),
           verdict=(SELECT verdict FROM tasks WHERE id=?2),
           feedback=?3, base=NULL, result=NULL
         WHERE id=?1",
        params![id, source, feedback.to_string()],
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
        parts.push(format!(
            "{} run again: {why}",
            repeated.join(", ")
        ));
    }
    (!parts.is_empty()).then(|| parts.join("; "))
}

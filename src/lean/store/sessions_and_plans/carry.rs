//! Approved work a new plan keeps instead of repeating.
use super::*;

pub(super) type Row = (String, Value, Option<Value>);
type Records = Vec<Vec<rusqlite::types::Value>>;

#[derive(PartialEq)]
struct TaskInputs {
    task: Records,
    scope_binding: Records,
    scope: Records,
    scope_sources: Records,
    contract: Records,
    result_turn: Option<String>,
    fallback_producer: Option<String>,
    result: Records,
    result_context: Records,
    evidence: Records,
}

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

const ELIGIBLE: &str = "(status IN ('approved','stale') OR (status IN ('coordinating','failed','cancelled','needs_user','waiting','queued') AND (report IS NOT NULL OR result IS NOT NULL)))";

struct ProjectedProducer {
    id: String,
    round: i64,
}

#[derive(PartialEq)]
pub(super) struct Capture {
    project: String,
    source_scope: Option<bool>,
    targets: Vec<Row>,
    sources: BTreeMap<String, Row>,
    tokens: BTreeMap<String, TaskInputs>,
    dependencies: BTreeMap<String, bool>,
    origins: BTreeMap<(String, String), Option<String>>,
    scopes: BTreeMap<String, bool>,
    revisions: BTreeMap<String, Option<String>>,
    workflow: Records,
    retained: BTreeSet<String>,
}

/// Capture only demanded producers and the plan-local dependencies they read.
/// Reselecting the same inputs detects newly eligible producers as well as edits.
pub(super) fn capture(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    plan: &str,
) -> Result<Capture> {
    let targets = rows(
        tx,
        "SELECT id,spec,feedback FROM tasks WHERE plan_id=?1 AND status<>'superseded' ORDER BY position,rowid",
        plan,
    )?;
    let project = one(
        tx,
        "SELECT project FROM conversations WHERE id=?1",
        [conversation],
        |r| r.get(0),
    )?;
    let mut captured = Capture {
        project,
        source_scope: source_plan(tx, conversation, plan)?.map(|(_, same)| same),
        targets,
        sources: BTreeMap::new(),
        tokens: BTreeMap::new(),
        dependencies: BTreeMap::new(),
        origins: BTreeMap::new(),
        scopes: BTreeMap::new(),
        revisions: BTreeMap::new(),
        retained: BTreeSet::new(),
        workflow: query_values(
            tx,
            "SELECT w.*,h.revision,a.plan_id FROM session_workflow w JOIN plan_heads h ON h.plan_id=?2 LEFT JOIN plan_activations a ON a.plan_id=h.plan_id WHERE w.conversation_id=?1",
            params![conversation, plan],
        )?,
    };
    let target_scope = scope(tx, plan)?;
    for (id, spec, feedback) in &captured.targets {
        captured.tokens.insert(id.clone(), task_token(tx, id)?);
        if captured.source_scope.is_none() {
            continue;
        }
        let name = spec["id"].as_str().unwrap_or_default();
        if !captured.sources.contains_key(name) {
            let pinned = feedback
                .as_ref()
                .and_then(|f| f["kept_approval"]["source"].as_str());
            let sql = format!(
                "SELECT id,spec,feedback FROM tasks WHERE conversation_id=?1 AND CASE WHEN json_valid(spec) THEN json_extract(spec,'$.id') END=?2 AND plan_id<>?3 AND {ELIGIBLE} AND (?4 IS NULL OR id=?4) ORDER BY created_ms DESC,rowid DESC LIMIT 1"
            );
            let source: Option<(String, String, Option<String>)> =
                one(tx, &sql, params![conversation, name, plan, pinned], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })
                .optional()?;
            if let Some(row) = source {
                captured.sources.insert(name.to_owned(), decode_row(row)?);
            }
        }
    }
    captured.retained = super::retained::valid_sources(tx, &captured.targets, &captured.sources)?;
    let projected_producers = captured
        .sources
        .iter()
        .map(|(name, source)| {
            let producer = source
                .2
                .as_ref()
                .and_then(|v| v["carried_from"].as_str())
                .unwrap_or(&source.0)
                .to_owned();
            let round = one(
                tx,
                "SELECT round FROM tasks WHERE id=?1",
                [&source.0],
                |r| r.get(0),
            )?;
            Ok((
                name.clone(),
                ProjectedProducer {
                    id: producer,
                    round,
                },
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    for source in captured.sources.values() {
        if let std::collections::btree_map::Entry::Vacant(entry) =
            captured.tokens.entry(source.0.clone())
        {
            entry.insert(task_token(tx, &source.0)?);
        }
        let old_plan: String = one(
            tx,
            "SELECT plan_id FROM tasks WHERE id=?1",
            [&source.0],
            |r| r.get(0),
        )?;
        let contract: crate::lean::tasks::TaskContract = serde_json::from_value(source.1.clone())?;
        let mut unchanged_dependencies = true;
        for dependency in &contract.depends_on {
            for p in [old_plan.as_str(), plan] {
                let key = (p.to_owned(), dependency.clone());
                if let std::collections::btree_map::Entry::Vacant(entry) =
                    captured.origins.entry(key)
                {
                    let id: Option<String> = one(tx, "SELECT id FROM tasks WHERE plan_id=?1 AND CASE WHEN json_valid(spec) THEN json_extract(spec,'$.id') END=?2 AND status<>'superseded'", params![p,dependency], |r| r.get(0)).optional()?;
                    if let Some(id) = id
                        && let std::collections::btree_map::Entry::Vacant(entry) =
                            captured.tokens.entry(id.clone())
                    {
                        entry.insert(task_token(tx, &id)?);
                    }
                    entry.insert(dependency_origin(
                        tx,
                        p,
                        dependency,
                        if p == plan {
                            projected_producers.get(dependency)
                        } else {
                            None
                        },
                    )?);
                }
            }
            let before = &captured.origins[&(old_plan.clone(), dependency.clone())];
            let after = &captured.origins[&(plan.to_owned(), dependency.clone())];
            unchanged_dependencies &= before.is_some() && before == after;
        }
        captured
            .dependencies
            .insert(source.0.clone(), unchanged_dependencies);
        let approved: bool = one(
            tx,
            "SELECT status='approved' FROM tasks WHERE id=?1",
            [&source.0],
            |r| r.get(0),
        )?;
        captured.scopes.insert(
            source.0.clone(),
            approved
                && target_scope.is_some()
                && scope(tx, &old_plan)?
                    .as_ref()
                    .zip(target_scope.as_ref())
                    .is_some_and(|(a, b)| a.1 == b.1),
        );
        captured.revisions.insert(source.0.clone(), one(tx, "SELECT revision FROM task_source_revisions WHERE task_id=?1 AND generation=coalesce((SELECT generation FROM task_inputs WHERE task_id=?1),0)", [&source.0], |r| r.get(0)).optional()?);
    }
    // Older results without a pinned turn still resolve through producer rows.
    // Follow only that actual lineage, never the conversation's task history.
    let mut pending = captured
        .tokens
        .keys()
        .chain(projected_producers.values().map(|producer| &producer.id))
        .chain(captured.origins.values().flatten())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut visited = BTreeSet::new();
    while let Some(id) = pending.pop_first() {
        if !visited.insert(id.clone()) {
            continue;
        }
        if let std::collections::btree_map::Entry::Vacant(entry) = captured.tokens.entry(id.clone())
        {
            entry.insert(task_token(tx, &id)?);
        }
        if let Some(producer) = &captured.tokens[&id].fallback_producer {
            pending.insert(producer.clone());
        }
    }
    Ok(captured)
}

fn scope(tx: &rusqlite::Transaction<'_>, plan: &str) -> Result<Option<(i64, String)>> {
    Ok(one(
        tx,
        "SELECT revision,scope_id FROM plan_scopes WHERE plan_id=?1 ORDER BY revision DESC LIMIT 1",
        [plan],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()?)
}

pub(super) fn one<T>(
    tx: &rusqlite::Transaction<'_>,
    sql: &str,
    params: impl rusqlite::Params,
    f: impl FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
) -> rusqlite::Result<T> {
    let mut stmt = tx.prepare(sql)?;
    let result = stmt.query_row(params, f);
    #[cfg(test)]
    {
        measurements::statement(&stmt, usize::from(result.is_ok()));
        measurements::update(|m| {
            m.producer_lookups += usize::from(
                sql.starts_with("SELECT id,spec,feedback FROM tasks WHERE conversation_id="),
            );
            m.dependency_lookups += usize::from(
                sql.starts_with("SELECT id FROM tasks WHERE plan_id=")
                    || sql.starts_with("SELECT CASE WHEN EXISTS(SELECT 1 FROM agent_results"),
            );
        });
    }
    result
}

pub(super) fn execute(
    tx: &rusqlite::Transaction<'_>,
    sql: &str,
    params: impl rusqlite::Params,
) -> rusqlite::Result<usize> {
    let mut stmt = tx.prepare(sql)?;
    let result = stmt.execute(params);
    #[cfg(test)]
    measurements::statement(&stmt, 0);
    result
}

fn query_values(
    tx: &rusqlite::Transaction<'_>,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Records> {
    let mut stmt = tx.prepare(sql)?;
    let columns = stmt.column_count();
    let mut rows = stmt.query(params)?;
    let mut values = Vec::new();
    while let Some(row) = rows.next()? {
        values.push(
            (0..columns)
                .map(|column| row.get(column))
                .collect::<rusqlite::Result<Vec<_>>>()?,
        );
    }
    drop(rows);
    #[cfg(test)]
    {
        measurements::statement(&stmt, values.len());
        measurements::update(|m| {
            if sql.starts_with("SELECT t.*") && !values.is_empty() {
                m.spec_cells += 1;
                m.feedback_cells += usize::from(values[0][11] != rusqlite::types::Value::Null);
            }
            if sql.starts_with("SELECT version,spec") && !values.is_empty() {
                m.spec_cells += 1;
            }
        });
    }
    Ok(values)
}

fn task_token(tx: &rusqlite::Transaction<'_>, task: &str) -> Result<TaskInputs> {
    let result_turn = result_turn(tx, task)?;
    let fallback_producer = if result_turn.is_none() {
        one(tx, "SELECT producer_task_id FROM task_result_sources WHERE task_id=?1 AND generation=coalesce((SELECT generation FROM task_inputs WHERE task_id=?1),0)", [task], |r| r.get(0)).optional()?
    } else {
        None
    };
    let (result, result_context, evidence) = if let Some(turn) = &result_turn {
        (
            query_values(
                tx,
                "SELECT result FROM agent_results WHERE turn_id=?1",
                [turn],
            )?,
            query_values(
                tx,
                "SELECT u.*,s.task_id,s.round,i.generation FROM turns u LEFT JOIN turn_scopes s ON s.turn_id=u.id LEFT JOIN turn_inputs i ON i.turn_id=u.id WHERE u.id=?1",
                [turn],
            )?,
            query_values(
                tx,
                "SELECT e.* FROM result_evidence r JOIN agent_evidence e ON e.id=r.evidence_id WHERE r.turn_id=?1 ORDER BY e.id",
                [turn],
            )?,
        )
    } else {
        (Vec::new(), Vec::new(), Vec::new())
    };
    Ok(TaskInputs {
        task: query_values(
            tx,
            "SELECT t.*,i.generation,s.producer_task_id,v.revision,r.source_task,r.result_turn,h.revision FROM tasks t LEFT JOIN task_inputs i ON i.task_id=t.id LEFT JOIN task_result_sources s ON s.task_id=t.id AND s.generation=coalesce(i.generation,0) LEFT JOIN task_source_revisions v ON v.task_id=t.id AND v.generation=coalesce(i.generation,0) LEFT JOIN task_resume_results r ON r.task_id=t.id AND r.generation=coalesce(i.generation,0) LEFT JOIN plan_heads h ON h.plan_id=t.plan_id WHERE t.id=?1",
            [task],
        )?,
        scope_binding: query_values(
            tx,
            "SELECT revision,scope_id FROM plan_scopes WHERE plan_id=(SELECT plan_id FROM tasks WHERE id=?1) ORDER BY revision DESC LIMIT 1",
            [task],
        )?,
        scope: query_values(
            tx,
            "SELECT s.* FROM scopes s WHERE s.id=(SELECT scope_id FROM plan_scopes WHERE plan_id=(SELECT plan_id FROM tasks WHERE id=?1) ORDER BY revision DESC LIMIT 1)",
            [task],
        )?,
        scope_sources: query_values(
            tx,
            "SELECT c.* FROM scope_documents d JOIN conversation_sources c ON c.id=d.source_id WHERE d.scope_id=(SELECT scope_id FROM plan_scopes WHERE plan_id=(SELECT plan_id FROM tasks WHERE id=?1) ORDER BY revision DESC LIMIT 1) ORDER BY c.id",
            [task],
        )?,
        contract: query_values(
            tx,
            "SELECT version,spec FROM task_contracts WHERE task_id=?1 ORDER BY version DESC LIMIT 1",
            [task],
        )?,
        result_turn,
        fallback_producer,
        result,
        result_context,
        evidence,
    })
}

pub(super) fn result_turn(tx: &rusqlite::Transaction<'_>, task: &str) -> Result<Option<String>> {
    Ok(one(
        tx,
        "SELECT coalesce((SELECT r.turn_id FROM agent_results r JOIN turn_scopes s ON s.turn_id=r.turn_id JOIN turn_inputs x ON x.turn_id=r.turn_id JOIN turns u ON u.id=r.turn_id WHERE s.task_id=?1 AND x.generation=coalesce((SELECT generation FROM task_inputs WHERE task_id=?1),0) AND u.role='worker:'||?1 AND json_extract(r.result,'$.type')='completed' ORDER BY s.round DESC,u.started_ms DESC LIMIT 1),(SELECT result_turn FROM task_resume_results WHERE task_id=?1 AND generation=coalesce((SELECT generation FROM task_inputs WHERE task_id=?1),0)))",
        [task],
        |r| r.get(0),
    )?)
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum SnapshotInputs {
    Source,
    Artifacts(Vec<String>),
}

/// Caches belong to this capture's single repository and immutable comparison
/// inputs; they are never shared across calls against a mutable worktree.
pub(super) struct Prepared<'a> {
    captured: &'a Capture,
    changes: BTreeMap<String, Option<Arc<Vec<String>>>>,
    snapshots: BTreeMap<SnapshotInputs, String>,
    revisions: BTreeMap<String, String>,
}

impl Capture {
    pub(super) fn retained_preview(&self) -> Value {
        Value::Array(self.targets.iter().filter(|row| self.retained.contains(&row.0)).map(|row| {
            let pin = &row.2.as_ref().unwrap()["kept_approval"];
            serde_json::json!({"id":row.1["id"],"task":row.0,"source":pin["source"],"round":pin["round"]})
        }).collect())
    }
    /// Git runs with neither a Store guard nor a SQLite transaction alive.
    pub(super) fn compare_sources(&self) -> Result<Prepared<'_>> {
        let project = std::path::Path::new(&self.project);
        let mut prepared = Prepared {
            captured: self,
            changes: BTreeMap::new(),
            snapshots: BTreeMap::new(),
            revisions: BTreeMap::new(),
        };
        let mut diffs = BTreeMap::new();

        let repository = if self.revisions.values().any(Option::is_some) {
            crate::lean::git::repository_probe(project)?
        } else {
            false
        };
        for source in self.sources.values() {
            let Some(revision) = self.revisions[&source.0].as_ref().filter(|_| repository) else {
                prepared.changes.insert(source.0.clone(), None);
                continue;
            };
            let outputs = snapshot_outputs(source)?;
            if !prepared.snapshots.contains_key(&outputs) {
                prepared
                    .snapshots
                    .insert(outputs.clone(), input_snapshot(project, &outputs)?);
            }
            if !prepared.revisions.contains_key(revision) {
                prepared.revisions.insert(
                    revision.clone(),
                    crate::lean::git::ok(
                        project,
                        &["rev-parse", "--verify", &format!("{revision}^{{commit}}")],
                    )?,
                );
            }
            let key = (
                prepared.revisions[revision].clone(),
                prepared.snapshots[&outputs].clone(),
            );
            if !diffs.contains_key(&key) {
                #[cfg(test)]
                measurements::update(|m| m.comparisons += 1);
                diffs.insert(
                    key.clone(),
                    Arc::new(crate::lean::git::changed_files(project, &key.0, &key.1)?),
                );
            }
            prepared
                .changes
                .insert(source.0.clone(), Some(diffs[&key].clone()));
        }
        Ok(prepared)
    }
}

fn snapshot_outputs(source: &Row) -> Result<SnapshotInputs> {
    let spec: crate::lean::tasks::TaskContract = serde_json::from_value(source.1.clone())?;
    if spec.change_kind() != crate::lean::tasks::ChangeKind::Artifact {
        return Ok(SnapshotInputs::Source);
    }
    let mut outputs = spec.outputs;
    outputs.sort();
    outputs.dedup();
    Ok(SnapshotInputs::Artifacts(outputs))
}

fn input_snapshot(project: &std::path::Path, inputs: &SnapshotInputs) -> Result<String> {
    #[cfg(test)]
    measurements::update(|m| m.snapshots += 1);
    let (outputs, purpose) = match inputs {
        SnapshotInputs::Source => (
            &[][..],
            crate::lean::git::CommitPurpose::SourceInputSnapshot,
        ),
        SnapshotInputs::Artifacts(outputs) => (
            outputs.as_slice(),
            crate::lean::git::CommitPurpose::ArtifactInputSnapshot,
        ),
    };
    crate::lean::git::snapshot_with_artifacts(project, outputs, purpose)
}

impl Prepared<'_> {
    pub(super) fn validate_sources(&self) -> Result<()> {
        let project = std::path::Path::new(&self.captured.project);
        for (revision, resolved) in &self.revisions {
            crate::require(
                crate::lean::git::ok(
                    project,
                    &["rev-parse", "--verify", &format!("{revision}^{{commit}}")],
                )? == *resolved,
                "carry revision changed during preparation; approve the plan again",
            )?;
        }
        for (outputs, before) in &self.snapshots {
            let after = input_snapshot(project, outputs)?;
            crate::require(
                crate::lean::git::changed_files(project, before, &after)?.is_empty(),
                "carry inputs changed during preparation; approve the plan again",
            )?;
        }
        Ok(())
    }
}

pub(super) fn apply(
    tx: &rusqlite::Transaction<'_>,
    prepared: &Prepared<'_>,
) -> Result<Option<String>> {
    let captured = prepared.captured;
    let Some(same_scope) = captured.source_scope else {
        return Ok(None);
    };
    let (kept, repeated) = carry_plan_tasks(tx, captured, prepared)?;
    let why = if same_scope {
        "their contracts changed since approval"
    } else {
        "the scope changed since their approval"
    };
    Ok(carry_notice(&kept, &repeated, why))
}

fn carry_plan_tasks(
    tx: &rusqlite::Transaction<'_>,
    captured: &Capture,
    prepared: &Prepared,
) -> Result<(Vec<String>, Vec<String>)> {
    let (mut kept, mut repeated) = (vec![], vec![]);
    for (id, spec, _) in &captured.targets {
        let name = spec["id"].as_str().unwrap_or_default().to_owned();
        let Some(old) = captured.sources.get(&name) else {
            continue;
        };
        let plan: String = one(tx, "SELECT plan_id FROM tasks WHERE id=?1", [id], |r| {
            r.get(0)
        })?;
        let unchanged_contract =
            same_contract(&old.1, spec) && options_owner_current(tx, &old.0, &plan, spec)?;
        let source_changes = &prepared.changes[&old.0];
        let unchanged_sources = source_changes
            .as_ref()
            .is_some_and(|paths| paths.is_empty());
        let unchanged_dependencies = captured.dependencies[&old.0];
        let unchanged_scope = captured.scopes[&old.0];
        let compatible = captured.retained.contains(id)
            || (unchanged_contract
                && unchanged_sources
                && unchanged_dependencies
                && unchanged_scope);
        if !compatible {
            let fields = spec
                .as_object()
                .into_iter()
                .flat_map(|o| o.keys())
                .filter(|key| old.1[*key] != spec[*key])
                .cloned()
                .collect::<Vec<_>>();
            execute(tx, "INSERT INTO events(conversation_id,plan_id,kind,payload,created_ms) SELECT conversation_id,plan_id,'task_reassessment_required',?2,?3 FROM tasks WHERE id=?1",params![id,serde_json::json!({"task":id,"source":old.0,"contract_fields":fields,"source_comparison_unknown":source_changes.is_none(),"changed_paths":source_changes.as_ref().map(|paths| paths.iter().take(12).collect::<Vec<_>>()),"changed_path_count":source_changes.as_ref().map(|paths| paths.len()),"dependency_producer_changed":!unchanged_dependencies,"scope_changed":!unchanged_scope}).to_string(),now_ms()])?;
        }
        let revision = captured.revisions[&old.0].as_ref().map(|revision| {
            prepared
                .revisions
                .get(revision)
                .unwrap_or(revision)
                .as_str()
        });
        carry(tx, id, old, compatible, revision)?;
        if compatible {
            kept.push(name);
        } else {
            let mut reasons = Vec::new();
            if !unchanged_contract {
                reasons.push("contract fields changed");
            }
            if !unchanged_sources {
                reasons.push(if source_changes.is_none() {
                    "source comparison unavailable"
                } else {
                    "project inputs changed"
                });
            }
            if !unchanged_dependencies {
                reasons.push("dependency producer changed");
            }
            if !unchanged_scope {
                reasons.push("scope needs reassessment");
            }
            repeated.push(format!("{name} ({})", reasons.join(", ")));
        }
    }
    Ok((kept, repeated))
}

fn dependency_origin(
    tx: &rusqlite::Transaction<'_>,
    plan: &str,
    dependency: &str,
    carried_producer: Option<&ProjectedProducer>,
) -> Result<Option<String>> {
    Ok(one(tx, "SELECT CASE WHEN EXISTS(SELECT 1 FROM agent_results r JOIN turns u ON u.id=r.turn_id JOIN turn_scopes x ON x.turn_id=u.id JOIN turn_inputs i ON i.turn_id=u.id WHERE x.task_id=t.id AND u.role='worker:'||t.id AND x.round<=coalesce(?4,t.round) AND i.generation=coalesce((SELECT generation FROM task_inputs WHERE task_id=t.id),0) AND json_extract(r.result,'$.type')='completed') THEN t.id ELSE coalesce(s.producer_task_id,?3,json_extract(t.feedback,'$.carried_from'),t.id) END FROM tasks t LEFT JOIN task_result_sources s ON s.task_id=t.id AND s.generation=coalesce((SELECT generation FROM task_inputs WHERE task_id=t.id),0) WHERE t.plan_id=?1 AND CASE WHEN json_valid(t.spec) THEN json_extract(t.spec,'$.id') END=?2 AND t.status<>'superseded'", params![plan,dependency,carried_producer.map(|p| p.id.as_str()),carried_producer.map(|p| p.round)], |r| r.get(0)).optional()?)
}

/// Latest eligible history is an indexed seek; scope lookup touches one row.
fn source_plan(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    plan_id: &str,
) -> Result<Option<(String, bool)>> {
    let sql = format!(
        "SELECT plan_id FROM tasks WHERE conversation_id=?1 AND plan_id<>?2 AND {ELIGIBLE} ORDER BY created_ms DESC,rowid DESC LIMIT 1"
    );
    let source: Option<String> =
        one(tx, &sql, params![conversation, plan_id], |r| r.get(0)).optional()?;
    source
        .map(|source| {
            let a = scope(tx, &source)?;
            let b = scope(tx, plan_id)?;
            let same = a.as_ref().zip(b.as_ref()).is_some_and(|(a, b)| a.1 == b.1);
            Ok((source, same))
        })
        .transpose()
}

fn rows(tx: &rusqlite::Transaction<'_>, sql: &str, plan: &str) -> Result<Vec<Row>> {
    let mut statement = tx.prepare(sql)?;
    let encoded = statement
        .query_map([plan], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    #[cfg(test)]
    measurements::statement(&statement, encoded.len());
    encoded.into_iter().map(decode_row).collect()
}

fn decode_row((id, spec, feedback): (String, String, Option<String>)) -> Result<Row> {
    #[cfg(test)]
    measurements::update(|m| {
        m.spec_decodes += 1;
        m.spec_cells += 1;
        m.feedback_cells += usize::from(feedback.is_some());
        m.feedback_decodes += usize::from(feedback.is_some());
    });
    Ok((
        id,
        serde_json::from_str(&spec)?,
        feedback
            .as_deref()
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
            .filter(Value::is_object),
    ))
}

/// Dependency order is the planner's; everything else must be identical.
/// Generated options carry no owner snapshot in their goal. Compare the bound
/// owner contracts before retaining a selection across plans.
fn options_owner_current(
    tx: &rusqlite::Transaction<'_>,
    old: &str,
    plan: &str,
    spec: &Value,
) -> Result<bool> {
    let Some(owner) = spec["options_for"].as_str() else {
        return Ok(true);
    };
    let previous: Option<String> = tx.query_row("SELECT spec FROM tasks WHERE plan_id=(SELECT plan_id FROM tasks WHERE id=?1) AND json_extract(spec,'$.id')=?2", params![old, owner], |row| row.get(0)).optional()?;
    let current: Option<String> = tx
        .query_row(
            "SELECT spec FROM tasks WHERE plan_id=?1 AND json_extract(spec,'$.id')=?2",
            params![plan, owner],
            |row| row.get(0),
        )
        .optional()?;
    let (Some(previous), Some(current)) = (previous, current) else {
        return Ok(false);
    };
    let mut previous: crate::lean::tasks::TaskContract = serde_json::from_str(&previous)?;
    let current: crate::lean::tasks::TaskContract = serde_json::from_str(&current)?;
    previous.title = current.title.clone();
    previous.summary = current.summary.clone();
    previous.section = current.section.clone();
    previous.reviewer = current.reviewer.clone();
    Ok(crate::lean::tasks::same_contract(&previous, &current))
}

fn same_contract(old: &Value, new: &Value) -> bool {
    let normal = |spec: &Value| {
        let mut spec = spec.clone();
        if let Ok(mut typed) =
            serde_json::from_value::<crate::lean::tasks::TaskContract>(spec.clone())
        {
            crate::lean::tasks::normalize_options_presentation(&mut typed);
            spec = serde_json::to_value(typed).expect("contract serializes");
        }
        if let Some(object) = spec.as_object_mut() {
            for field in [
                "acceptance_criteria",
                "scope",
                "title",
                "summary",
                "section",
                "reviewer",
            ] {
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
    compatible: bool,
    revision: Option<&str>,
) -> Result<()> {
    let integrated: bool = one(
        tx,
        "SELECT status='approved' FROM tasks WHERE id=?1",
        [source],
        |r| r.get(0),
    )?;
    let mut feedback = feedback.clone().unwrap_or_else(|| serde_json::json!({}));
    feedback.as_object_mut().unwrap().remove("kept_approval");
    // A result kept twice still names the task that produced it.
    if feedback["carried_from"].as_str().is_none() {
        feedback["carried_from"] = source.as_str().into();
    }
    if !integrated {
        feedback["continuation_review"] = true.into();
    }
    if !compatible {
        feedback["resume_role"] = "reviewer".into();
        feedback["compatibility_review"] = true.into();
    }
    execute(
        tx,
        "INSERT OR IGNORE INTO task_resume_results VALUES(?1,coalesce((SELECT generation FROM task_inputs WHERE task_id=?1),0),?2,?3)",
        params![id, source, result_turn(tx, source)?],
    )?;
    execute(
        tx,
        "INSERT OR IGNORE INTO task_result_sources(task_id,generation,producer_task_id) VALUES(?1,coalesce((SELECT generation FROM task_inputs WHERE task_id=?1),0),?2)",
        params![id, feedback["carried_from"].as_str().unwrap_or(source)],
    )?;
    if let Some(revision) = revision {
        execute(
            tx,
            "INSERT OR IGNORE INTO task_source_revisions(task_id,generation,revision) VALUES(?1,coalesce((SELECT generation FROM task_inputs WHERE task_id=?1),0),?2)",
            params![id, revision],
        )?;
    }
    execute(tx,
        "UPDATE tasks SET status=?4,
           round=(SELECT round FROM tasks WHERE id=?2),
           report=(SELECT report FROM tasks WHERE id=?2),
           verdict=CASE WHEN ?4='approved' THEN (SELECT verdict FROM tasks WHERE id=?2) ELSE NULL END,
           feedback=?3, base=CASE WHEN ?5 THEN NULL ELSE (SELECT base FROM tasks WHERE id=?2) END, result=CASE WHEN ?5 THEN NULL ELSE (SELECT result FROM tasks WHERE id=?2) END
         WHERE id=?1",
        params![id, source, feedback.to_string(), if compatible { "approved" } else { "queued" }, integrated],
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
            "{} retained for compatibility review; executor work is not restarted ({why})",
            repeated.join(", ")
        ));
    }
    (!parts.is_empty()).then(|| parts.join("; "))
}

#[cfg(test)]
pub(in crate::lean::store) mod measurements {
    use std::cell::RefCell;
    #[derive(Debug, Default, Clone, PartialEq, Eq)]
    pub struct Work {
        pub rows: usize,
        pub spec_decodes: usize,
        pub spec_cells: usize,
        pub feedback_decodes: usize,
        pub feedback_cells: usize,
        pub statements: usize,
        pub producer_lookups: usize,
        pub dependency_lookups: usize,
        pub vm_steps: i64,
        pub fullscan_steps: i64,
        pub sorts: i64,
        pub comparisons: usize,
        pub snapshots: usize,
    }
    thread_local! { static WORK: RefCell<Work> = RefCell::default(); }
    pub fn update(f: impl FnOnce(&mut Work)) {
        WORK.with(|w| f(&mut w.borrow_mut()));
    }
    pub fn take() -> Work {
        WORK.with(|w| std::mem::take(&mut *w.borrow_mut()))
    }
    pub fn statement(stmt: &rusqlite::Statement<'_>, rows: usize) {
        use rusqlite::StatementStatus::*;
        update(|m| {
            m.rows += rows;
            m.statements += 1;
            m.vm_steps += i64::from(stmt.get_status(VmStep));
            m.fullscan_steps += i64::from(stmt.get_status(FullscanStep));
            m.sorts += i64::from(stmt.get_status(Sort));
        });
    }
}

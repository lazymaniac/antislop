use super::super::now_ms;
use super::*;
use crate::lean::{ledger::validate_task_links, tasks::status};
use crate::{Result, require};
use apply::apply_specs;
use rusqlite::params;

#[path = "helpers/apply.rs"]
mod apply;

pub(super) struct EditContext<'a> {
    conversation: &'a str,
    plan: String,
    run: String,
    entries: Vec<crate::lean::ledger::LedgerEntry>,
}

pub(super) fn prepare_graph_edit<'a>(
    store: &Store,
    conversation: &'a str,
    baseline: &[Task],
    specs: &[TaskSpec],
) -> Result<EditContext<'a>> {
    require(!baseline.is_empty(), "no graph to edit")?;
    crate::lean::tasks::validate(specs)?;
    let plan = baseline[0].plan_id.clone();
    let run = store
        .plan_run(&plan)?
        .ok_or_else(|| crate::Error::Invalid("graph has no requirements run".into()))?;
    require(
        store.extraction_current(conversation, &run)?,
        "requirements need independent verification before editing the graph",
    )?;
    let entries = store.ledger_entries(conversation, &run, None)?;
    validate_candidate_coverage(store, conversation, &plan, &run, baseline, specs, &entries)?;
    Ok(EditContext {
        conversation,
        plan,
        run,
        entries,
    })
}

pub(super) fn persist_graph_edit(
    store: &Store,
    context: &EditContext<'_>,
    baseline: &[Task],
    specs: &[TaskSpec],
) -> Result<()> {
    let mut db = store.db();
    let tx = db.transaction()?;
    verify_graph_snapshot(&tx, context.conversation, &context.plan, baseline.len())?;
    verify_baseline_specs(&tx, context.conversation, &context.plan, baseline, specs)?;
    apply_specs(&tx, context, baseline, specs)?;
    tx.commit()?;
    Ok(())
}

pub(super) fn validate_candidate_coverage(
    store: &Store,
    conversation: &str,
    plan: &str,
    run: &str,
    baseline: &[Task],
    specs: &[TaskSpec],
    entries: &[crate::lean::ledger::LedgerEntry],
) -> Result<()> {
    for task in baseline {
        store.assert_task_inputs(task)?;
    }
    let mut covered = std::collections::HashSet::new();
    for spec in specs {
        validate_task_links(spec, entries)?;
        covered.extend(spec.criterion_ids.iter().cloned());
    }
    include_approved_criteria(store, conversation, plan, run, &mut covered)?;
    crate::lean::ledger::validate_criterion_coverage(entries, &covered)
}

fn include_approved_criteria(
    store: &Store,
    conversation: &str,
    plan: &str,
    run: &str,
    covered: &mut std::collections::HashSet<String>,
) -> Result<()> {
    for task in store
        .run_tasks(conversation, run)?
        .iter()
        .filter(|task| task.plan_id != plan && task.status == "approved")
    {
        if store.assert_task_inputs(task).is_ok() {
            let spec: TaskSpec = serde_json::from_value(task.spec.clone())?;
            covered.extend(spec.criterion_ids);
        }
    }
    Ok(())
}

pub(super) fn verify_graph_snapshot(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    plan: &str,
    baseline_count: usize,
) -> Result<()> {
    verify_task_count(tx, conversation, plan, baseline_count)?;
    verify_graph_running(tx, plan)?;
    verify_requirements_current(tx, plan)
}

fn verify_task_count(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    plan: &str,
    baseline_count: usize,
) -> Result<()> {
    let count: i64 = tx.query_row(
        "SELECT count(*) FROM tasks WHERE conversation_id=?1 AND plan_id=?2",
        params![conversation, plan],
        |row| row.get(0),
    )?;
    require(
        count == baseline_count as i64,
        "graph changed during review; retry against its latest version",
    )
}

fn verify_graph_running(tx: &rusqlite::Transaction<'_>, plan: &str) -> Result<()> {
    let unfinished: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM tasks WHERE plan_id=?1 AND status IN ('queued','working','verifying'))",
        [plan],
        |row| row.get(0),
    )?;
    require(
        unfinished,
        "the graph finished during review; start a new interview for further work",
    )
}

fn verify_requirements_current(tx: &rusqlite::Transaction<'_>, plan: &str) -> Result<()> {
    let stale: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM tasks t JOIN task_inputs i ON i.task_id=t.id JOIN task_ledger_inputs l ON l.task_id=t.id WHERE t.plan_id=?1 AND l.version<>(SELECT max(v.version) FROM ledger_versions v WHERE v.run_id=i.run_id AND v.entry_id=l.entry_id))",
        [plan],
        |row| row.get(0),
    )?;
    require(
        !stale,
        "graph requirements changed during review; replan first",
    )
}

pub(super) fn verify_baseline_specs(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    plan: &str,
    baseline: &[Task],
    specs: &[TaskSpec],
) -> Result<()> {
    for old in baseline {
        verify_baseline_spec(tx, conversation, plan, old)?;
        require(
            specs
                .iter()
                .any(|spec| old.spec["id"].as_str() == Some(&spec.id)),
            "graph edits cannot remove existing tasks",
        )?;
    }
    Ok(())
}

fn verify_baseline_spec(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    plan: &str,
    old: &Task,
) -> Result<()> {
    let stored: String = tx.query_row(
        "SELECT spec FROM tasks WHERE id=?1 AND plan_id=?2 AND conversation_id=?3",
        params![old.id, plan, conversation],
        |row| row.get(0),
    )?;
    require(
        serde_json::from_str::<serde_json::Value>(&stored)? == old.spec,
        "graph changed during review; retry against its latest version",
    )
}

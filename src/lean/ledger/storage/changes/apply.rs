use super::*;

pub(super) fn saved_decision(
    tx: &rusqlite::Transaction<'_>,
    change: &LedgerChange,
    request_hash: &str,
) -> Result<Option<ChangeResult>> {
    let saved = tx
        .query_row(
            "SELECT request_hash,result FROM ledger_decisions WHERE operation_id=?1",
            [&change.operation_id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()?;
    saved
        .map(|(hash, result)| {
            require(
                hash == request_hash,
                "operation identity reused with a different payload",
            )?;
            Ok(serde_json::from_str(&result)?)
        })
        .transpose()
}

/// Stores a new version of every submitted entry except one resubmitted
/// exactly as it stands: its version, and the evidence recorded against it,
/// stay current.
pub(super) fn persist_entries(
    tx: &rusqlite::Transaction<'_>,
    change: &LedgerChange,
    before: &[LedgerEntry],
    state: &std::path::Path,
    revision: i64,
) -> Result<Vec<serde_json::Value>> {
    change
        .entries
        .iter()
        .filter(|entry| !before.iter().any(|old| &old.spec == *entry))
        .map(|entry| persist_entry(tx, change, entry, state, revision))
        .collect()
}

fn persist_entry(
    tx: &rusqlite::Transaction<'_>,
    change: &LedgerChange,
    entry: &EntrySpec,
    state: &std::path::Path,
    revision: i64,
) -> Result<serde_json::Value> {
    let text = source_text(tx, &change.run_id, &entry.source.source_id, state)?;
    entry.source.text(&text)?;
    tx.execute(
        "INSERT OR IGNORE INTO ledger_entries(run_id,id) VALUES(?1,?2)",
        params![change.run_id, entry.id],
    )?;
    let version = next_entry_version(tx, &change.run_id, &entry.id)?;
    insert_entry_version(tx, change, entry, revision, version)?;
    insert_entry_relations(tx, change, entry, version)?;
    Ok(serde_json::json!({"id":entry.id,"version":version}))
}

fn next_entry_version(tx: &rusqlite::Transaction<'_>, run: &str, id: &str) -> Result<i64> {
    Ok(tx.query_row(
        "SELECT coalesce(max(version),0)+1 FROM ledger_versions WHERE run_id=?1 AND entry_id=?2",
        params![run, id],
        |row| row.get(0),
    )?)
}

fn insert_entry_version(
    tx: &rusqlite::Transaction<'_>,
    change: &LedgerChange,
    entry: &EntrySpec,
    revision: i64,
    version: i64,
) -> Result<()> {
    let disposition = serde_json::to_value(entry.disposition)?
        .as_str()
        .unwrap()
        .to_owned();
    insert_version_row(tx, change, entry, revision, version, &disposition)?;
    insert_entry_criteria(tx, change, entry, version)
}

fn insert_version_row(
    tx: &rusqlite::Transaction<'_>,
    change: &LedgerChange,
    entry: &EntrySpec,
    revision: i64,
    version: i64,
    disposition: &str,
) -> Result<()> {
    // Source artifacts own the bytes; version rows preserve only the span.
    tx.execute(
        "INSERT INTO ledger_versions VALUES(?1,?2,?3,?4,?5,?6,?7,'',?8,?9,?10,?11)",
        params![
            change.run_id,
            entry.id,
            version,
            revision,
            entry.source.source_id,
            entry.source.start as i64,
            entry.source.end as i64,
            entry.interpretation,
            entry.mandatory,
            entry.position,
            disposition
        ],
    )?;
    Ok(())
}

fn insert_entry_criteria(
    tx: &rusqlite::Transaction<'_>,
    change: &LedgerChange,
    entry: &EntrySpec,
    version: i64,
) -> Result<()> {
    for criterion in &entry.criteria {
        insert_criterion(tx, change, entry, version, criterion)?;
    }
    Ok(())
}

fn insert_criterion(
    tx: &rusqlite::Transaction<'_>,
    change: &LedgerChange,
    entry: &EntrySpec,
    version: i64,
    criterion: &Criterion,
) -> Result<()> {
    tx.execute(
        "INSERT INTO ledger_criteria VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![
            change.run_id,
            entry.id,
            version,
            criterion.id,
            criterion.text,
            criterion.mandatory,
            criterion.integration
        ],
    )?;
    Ok(())
}

fn insert_entry_relations(
    tx: &rusqlite::Transaction<'_>,
    change: &LedgerChange,
    entry: &EntrySpec,
    version: i64,
) -> Result<()> {
    let relations = [
        ("parent", entry.parent.iter().cloned().collect()),
        ("dependency", entry.depends_on.clone()),
        ("task", entry.task_ids.clone()),
        ("qualifier", entry.qualifiers.clone()),
        ("implementer", entry.implementation_owners.clone()),
        ("verifier", entry.verification_owners.clone()),
    ];
    for (kind, values) in relations {
        for value in values {
            tx.execute(
                "INSERT INTO ledger_relations VALUES(?1,?2,?3,?4,?5)",
                params![change.run_id, entry.id, version, kind, value],
            )?;
        }
    }
    Ok(())
}

pub(super) fn mark_affected_tasks(tx: &rusqlite::Transaction<'_>, tasks: &[String]) -> Result<()> {
    for task in tasks {
        tx.execute(
            "UPDATE task_inputs SET generation=generation+1 WHERE task_id=?1",
            [task],
        )?;
        tx.execute("UPDATE tasks SET status='stale' WHERE id=?1", [task])?;
    }
    Ok(())
}

/// A task the change does not reach was built against the same meaning of
/// its entries; it moves to their new versions instead of failing its input
/// check for a resubmission that changed nothing it relies on.
pub(super) fn carry_unaffected_inputs(
    tx: &rusqlite::Transaction<'_>,
    change: &LedgerChange,
    versions: &[serde_json::Value],
    affected: &[String],
) -> Result<()> {
    for version in versions {
        let (Some(entry), Some(number)) = (version["id"].as_str(), version["version"].as_i64())
        else {
            continue;
        };
        let tasks: Vec<String> = tx
            .prepare("SELECT l.task_id FROM task_ledger_inputs l JOIN task_inputs i ON i.task_id=l.task_id WHERE i.run_id=?1 AND l.entry_id=?2")?
            .query_map(params![change.run_id, entry], |row| row.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        for task in tasks.iter().filter(|task| !affected.contains(task)) {
            tx.execute(
                "UPDATE task_ledger_inputs SET version=?3 WHERE task_id=?1 AND entry_id=?2",
                params![task, entry, number],
            )?;
        }
    }
    Ok(())
}

pub(super) fn save_decision(
    tx: &rusqlite::Transaction<'_>,
    change: &LedgerChange,
    result: &ChangeResult,
    author: &str,
    request_hash: &str,
) -> Result<()> {
    tx.execute(
        "UPDATE runs SET spec_revision=?2,state='active' WHERE id=?1",
        params![change.run_id, result.spec_revision],
    )?;
    tx.execute(
        "INSERT INTO ledger_decisions VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            change.operation_id,
            change.run_id,
            result.spec_revision,
            author,
            change.reason,
            request_hash,
            serde_json::to_string(result)?,
            now_ms()
        ],
    )?;
    Ok(())
}

pub(super) fn emit_change(
    tx: &rusqlite::Transaction<'_>,
    change: &LedgerChange,
    result: &ChangeResult,
    versions: &[serde_json::Value],
) -> Result<()> {
    event(
        tx,
        &change.conversation,
        "ledger_changed",
        &serde_json::json!({"run":result.run_id,"revision":result.spec_revision,"entries":versions,"affected_tasks":result.affected_tasks}),
    )
}

pub(super) fn resume_if_approved(
    tx: &rusqlite::Transaction<'_>,
    change: &LedgerChange,
    user: bool,
) -> Result<()> {
    if !user {
        return Ok(());
    }
    let proposal: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM ledger_proposals WHERE run_id=?1 AND json_extract(request,'$.operation_id')=?2 AND author LIKE 'turn:%')", params![change.run_id,change.operation_id], |row| row.get(0))?;
    if proposal {
        enqueue_requirements_resume(tx, change)?;
    }
    Ok(())
}

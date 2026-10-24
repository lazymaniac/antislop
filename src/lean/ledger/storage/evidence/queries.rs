use super::*;

pub(super) fn linked_entry_ids(
    store: &Store,
    query: &LedgerQuery,
) -> Result<Option<HashSet<String>>> {
    let Some(task) = &query.task else {
        return Ok(None);
    };
    let db = store.db();
    let ids = db.prepare("SELECT l.entry_id FROM task_ledger_inputs l JOIN task_inputs i ON i.task_id=l.task_id JOIN tasks t ON t.id=i.task_id WHERE i.run_id=?1 AND (t.id=?2 OR json_extract(t.spec,'$.id')=?2)")?
        .query_map(params![query.run_id,task], |row| row.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    Ok(Some(ids))
}

pub(super) fn filter_ledger_entries(
    entries: Vec<LedgerEntry>,
    query: &LedgerQuery,
    linked: Option<HashSet<String>>,
) -> Vec<LedgerEntry> {
    entries
        .into_iter()
        .filter(|entry| matches_ledger_filter(entry, query, linked.as_ref()))
        .collect()
}

fn matches_ledger_filter(
    entry: &LedgerEntry,
    query: &LedgerQuery,
    linked: Option<&HashSet<String>>,
) -> bool {
    query
        .status
        .as_ref()
        .is_none_or(|status| *status == entry.status)
        && (!query.missing_coverage
            || !matches!(entry.status.as_str(), "verified" | "superseded_by_user"))
        && linked.is_none_or(|ids| ids.contains(&entry.spec.id))
}

pub(super) fn build_ledger_page(
    run: Run,
    entries: Vec<LedgerEntry>,
    offset: usize,
    requested: Option<usize>,
) -> LedgerPage {
    let total = entries.len();
    let limit = requested.unwrap_or(50).clamp(1, 200);
    LedgerPage {
        run,
        entries: entries.into_iter().skip(offset).take(limit).collect(),
        total,
        offset,
        has_more: offset.saturating_add(limit) < total,
    }
}

pub(super) fn extraction_gap(store: &Store, conversation: &str, run: &str) -> Result<Vec<String>> {
    if store.extraction_current(conversation, run)? {
        return Ok(Vec::new());
    }
    Ok(vec![
        "source coverage has not been independently verified for this revision".into(),
    ])
}

pub(super) fn entry_gaps(entries: &[LedgerEntry]) -> Vec<String> {
    let mut gaps = Vec::new();
    if entries.is_empty() {
        gaps.push("ledger is empty".into());
    }
    for entry in entries.iter().filter(|e| {
        e.spec.disposition == Disposition::Active && e.spec.mandatory && e.status != "verified"
    }) {
        gaps.push(format!(
            "{}: {} (current integrated evidence required)",
            entry.spec.id, entry.status
        ));
    }
    gaps
}

pub(super) fn audit_missing(
    store: &Store,
    run: &str,
    revision: i64,
    fingerprint: &str,
) -> Result<bool> {
    let approved: Option<bool> = store.db().query_row("SELECT approved FROM ledger_reviews WHERE run_id=?1 AND spec_revision=?2 AND kind='audit' AND fingerprint=?3 ORDER BY created_ms DESC,rowid DESC LIMIT 1", params![run,revision,fingerprint], |row| row.get(0)).optional()?;
    Ok(!approved.unwrap_or(false))
}

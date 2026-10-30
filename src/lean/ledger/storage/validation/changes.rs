use super::super::*;
use super::{checked_graph, impact, same_meaning};

pub(in crate::lean::ledger::storage) fn validate_change(
    db: &Connection,
    change: &LedgerChange,
    state: &std::path::Path,
    user: bool,
) -> Result<LedgerDiff> {
    let r = run(db, &change.conversation, &change.run_id)?;
    validate_change_scope(change, &r)?;
    let before = entries(db, &r.id, r.spec_revision)?;
    let mut after = existing_specs(&before);
    let changed = apply_entry_changes(db, change, state, user, &mut after)?;
    make_diff(db, change, r, before, after, changed)
}

fn make_diff(
    db: &Connection,
    change: &LedgerChange,
    run: Run,
    before: Vec<LedgerEntry>,
    after: HashMap<String, EntrySpec>,
    changed: HashSet<String>,
) -> Result<LedgerDiff> {
    let altered = altered_entries(&changed, &before, &after);
    let all: Vec<_> = after.into_values().collect();
    checked_graph(&all)?;
    let previous: Vec<_> = before.iter().map(|entry| entry.spec.clone()).collect();
    let affected_tasks = impact(db, &run.id, &altered, &previous, &all)?;
    Ok(LedgerDiff {
        run_id: run.id,
        from_revision: run.spec_revision,
        to_revision: run.spec_revision + 1,
        before: before
            .into_iter()
            .filter(|entry| changed.contains(&entry.spec.id))
            .collect(),
        after: change.entries.clone(),
        affected_tasks,
    })
}

fn altered_entries(
    changed: &HashSet<String>,
    before: &[LedgerEntry],
    after: &HashMap<String, EntrySpec>,
) -> HashSet<String> {
    changed
        .iter()
        .filter(|id| {
            before
                .iter()
                .find(|old| &old.spec.id == *id)
                .is_none_or(|old| !same_meaning(&old.spec, &after[*id]))
        })
        .cloned()
        .collect()
}

fn existing_specs(entries: &[LedgerEntry]) -> HashMap<String, EntrySpec> {
    entries
        .iter()
        .map(|entry| (entry.spec.id.clone(), entry.spec.clone()))
        .collect()
}

fn apply_entry_changes(
    db: &Connection,
    change: &LedgerChange,
    state: &std::path::Path,
    user: bool,
    after: &mut HashMap<String, EntrySpec>,
) -> Result<HashSet<String>> {
    let mut changed = HashSet::new();
    for entry in &change.entries {
        validate_entry_change(db, change, state, user, entry, after, &mut changed)?;
    }
    Ok(changed)
}

fn validate_change_scope(change: &LedgerChange, run: &Run) -> Result<()> {
    require(
        change.expected_spec_revision >= 0 && change.expected_spec_revision < i64::MAX,
        "invalid spec revision",
    )?;
    require(
        run.spec_revision == change.expected_spec_revision,
        "spec revision conflict; reload before saving; draft preserved",
    )?;
    require(
        !change.operation_id.trim().is_empty() && !change.entries.is_empty(),
        "operation identity and changes are required",
    )
}

fn validate_entry_change(
    db: &Connection,
    change: &LedgerChange,
    state: &std::path::Path,
    user: bool,
    entry: &EntrySpec,
    after: &mut HashMap<String, EntrySpec>,
    changed: &mut HashSet<String>,
) -> Result<()> {
    require(
        changed.insert(entry.id.clone()),
        "duplicate change identity",
    )?;
    validate_entry_authority(entry, after.contains_key(&entry.id), change, user)?;
    let source = source_text(db, &change.run_id, &entry.source.source_id, state)?;
    entry.source.text(&source)?;
    after.insert(entry.id.clone(), entry.clone());
    Ok(())
}

fn validate_entry_authority(
    entry: &EntrySpec,
    exists: bool,
    change: &LedgerChange,
    user: bool,
) -> Result<()> {
    if exists {
        require(
            user,
            "agents may propose semantic changes but only the user may apply them",
        )?;
        require(
            !change.reason.trim().is_empty(),
            "scope edit requires a user rationale",
        )
    } else {
        require(
            entry.disposition == Disposition::Active,
            "new obligation must be active",
        )
    }
}

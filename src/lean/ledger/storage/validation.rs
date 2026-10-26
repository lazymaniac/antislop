use super::*;

pub(super) fn checked_graph(all: &[EntrySpec]) -> Result<()> {
    let index: HashMap<_, _> = all.iter().map(|e| (e.id.as_str(), e)).collect();
    require(index.len() == all.len(), "duplicate ledger identity")?;
    let mut criterion_ids = HashSet::new();
    for e in all {
        validate_entry(e, all, &index, &mut criterion_ids)?;
    }
    Ok(())
}

fn validate_entry<'a>(
    entry: &'a EntrySpec,
    all: &[EntrySpec],
    index: &HashMap<&'a str, &'a EntrySpec>,
    criterion_ids: &mut HashSet<&'a str>,
) -> Result<()> {
    require(
        !entry.id.trim().is_empty() && !entry.interpretation.trim().is_empty(),
        "ledger identity and interpretation are required",
    )?;
    validate_criteria(entry, criterion_ids)?;
    validate_relations(entry, index)?;
    validate_composite_requirement(entry, all)?;
    validate_entry_cycle(entry, index)
}

fn validate_criteria<'a>(entry: &'a EntrySpec, criterion_ids: &mut HashSet<&'a str>) -> Result<()> {
    require(
        entry.criteria.iter().all(|criterion| {
            !criterion.id.trim().is_empty()
                && !criterion.text.trim().is_empty()
                && criterion_ids.insert(criterion.id.as_str())
        }),
        "criteria need unique identities and nonempty text",
    )
}

fn validate_relations(entry: &EntrySpec, index: &HashMap<&str, &EntrySpec>) -> Result<()> {
    if let Some(parent) = &entry.parent {
        require(
            index.contains_key(parent.as_str()),
            &format!("entry {} has unknown parent {parent}", entry.id),
        )?;
    }
    for id in &entry.depends_on {
        require(index.contains_key(id.as_str()), "unknown ledger dependency")?;
    }
    Ok(())
}

fn validate_composite_requirement(entry: &EntrySpec, all: &[EntrySpec]) -> Result<()> {
    if entry.mandatory
        && all
            .iter()
            .any(|child| child.parent.as_deref() == Some(&entry.id))
    {
        require(
            entry
                .criteria
                .iter()
                .any(|criterion| criterion.mandatory && criterion.integration),
            &format!(
                "composite requirement {} needs its own integration criterion: add a criterion to this parent's criteria with mandatory=true and integration=true describing how its children work together; child criteria alone do not satisfy this check",
                entry.id
            ),
        )?;
    }
    Ok(())
}

fn validate_entry_cycle(entry: &EntrySpec, index: &HashMap<&str, &EntrySpec>) -> Result<()> {
    let mut todo = vec![entry.id.as_str()];
    let mut seen = HashSet::new();
    while let Some(id) = todo.pop() {
        if !seen.insert(id) {
            continue;
        }
        for dependency in index[id].depends_on.iter().chain(index[id].parent.iter()) {
            require(
                dependency != &entry.id,
                "cyclic ledger dependency or hierarchy",
            )?;
            todo.push(dependency);
        }
    }
    Ok(())
}

pub(super) fn impact(
    db: &Connection,
    run: &str,
    changed: &HashSet<String>,
    all: &[EntrySpec],
) -> Result<Vec<String>> {
    let ids = impacted_entries(changed, all);
    let mut tasks = tasks_for_entries(db, run, &ids)?;
    expand_dependent_tasks(db, run, &mut tasks)?;
    let mut out: Vec<_> = tasks.into_iter().collect();
    out.sort();
    Ok(out)
}

fn impacted_entries(changed: &HashSet<String>, all: &[EntrySpec]) -> HashSet<String> {
    let mut ids = changed.clone();
    loop {
        let n = ids.len();
        for e in all {
            if e.depends_on
                .iter()
                .chain(e.parent.iter())
                .any(|id| ids.contains(id))
                || all
                    .iter()
                    .any(|child| child.parent.as_deref() == Some(&e.id) && ids.contains(&child.id))
            {
                ids.insert(e.id.clone());
            }
        }
        if n == ids.len() {
            break;
        }
    }
    ids
}

fn tasks_for_entries(db: &Connection, run: &str, ids: &HashSet<String>) -> Result<HashSet<String>> {
    let mut tasks = HashSet::new();
    let mut stmt=db.prepare("SELECT i.task_id,l.entry_id FROM task_inputs i JOIN task_ledger_inputs l ON i.task_id=l.task_id WHERE i.run_id=?1")?;
    for row in stmt.query_map([run], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })? {
        let (task, id) = row?;
        if ids.contains(&id) {
            tasks.insert(task);
        }
    }
    Ok(tasks)
}

fn expand_dependent_tasks(db: &Connection, run: &str, tasks: &mut HashSet<String>) -> Result<()> {
    let rows = plan_task_rows(db, run)?;
    loop {
        let n = tasks.len();
        for (id, plan, spec) in &rows {
            let spec: TaskContract = serde_json::from_str(spec)?;
            for dep in &spec.depends_on {
                if rows.iter().any(|(up, p, s)| {
                    p == plan
                        && tasks.contains(up)
                        && serde_json::from_str::<TaskContract>(s).is_ok_and(|s| s.id == *dep)
                }) {
                    tasks.insert(id.clone());
                }
            }
        }
        if n == tasks.len() {
            break;
        }
    }
    Ok(())
}

fn plan_task_rows(db: &Connection, run: &str) -> Result<Vec<(String, String, String)>> {
    Ok(db
        .prepare("SELECT t.id,t.plan_id,t.spec FROM tasks t JOIN plan_runs p ON p.plan_id=t.plan_id WHERE p.run_id=?1")?
        .query_map([run], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<std::result::Result<_, _>>()?)
}

pub(super) fn validate_change(
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
    let all: Vec<_> = after.into_values().collect();
    checked_graph(&all)?;
    let affected_tasks = impact(db, &r.id, &changed, &all)?;
    Ok(LedgerDiff {
        run_id: r.id,
        from_revision: r.spec_revision,
        to_revision: r.spec_revision + 1,
        before: before
            .into_iter()
            .filter(|e| changed.contains(&e.spec.id))
            .collect(),
        after: change.entries.clone(),
        affected_tasks,
    })
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

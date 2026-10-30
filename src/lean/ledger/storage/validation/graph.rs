use super::super::*;

pub(in crate::lean::ledger::storage) fn checked_graph(all: &[EntrySpec]) -> Result<()> {
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

use super::{Disposition, LedgerEntry};
use crate::{Result, require};

/// Fills `requirement_ids` and `criterion_ids` from what the planner named in
/// `covers`: a criterion brings its requirement along, a requirement stands
/// for itself. Links already present (a stored plan) are kept when `covers`
/// is empty.
pub(crate) fn resolve_covers(
    spec: &mut crate::lean::tasks::TaskContract,
    entries: &[LedgerEntry],
) -> Result<()> {
    if spec.covers.is_empty() {
        if spec.requirement_ids.is_empty() {
            return Err(crate::Error::Invalid(format!(
                "task {}: `covers` is empty; name the acceptance criteria (or requirements) it delivers",
                spec.id
            )));
        }
        return Ok(());
    }
    let (mut requirements, mut criteria) = (Vec::new(), Vec::new());
    for id in &spec.covers {
        let (requirement, criterion) = resolve_cover(id, &spec.id, entries)?;
        push_unique(&mut requirements, requirement);
        if criterion {
            push_unique(&mut criteria, id);
        }
    }
    spec.requirement_ids = requirements;
    spec.criterion_ids = criteria;
    Ok(())
}

fn resolve_cover<'a>(
    id: &'a str,
    task: &str,
    entries: &'a [LedgerEntry],
) -> Result<(&'a str, bool)> {
    let mut active = entries
        .iter()
        .filter(|entry| entry.spec.disposition == Disposition::Active);
    if let Some(entry) = active.clone().find(|entry| entry.spec.id == id) {
        return Ok((&entry.spec.id, false));
    }
    active
        .find(|entry| entry.spec.criteria.iter().any(|criterion| criterion.id == id))
        .map(|entry| (entry.spec.id.as_str(), true))
        .ok_or_else(|| {
            crate::Error::Invalid(format!(
                "task {task}: covers names {id}, which is neither an active requirement nor one of its criteria"
            ))
        })
}

fn push_unique(ids: &mut Vec<String>, id: &str) {
    if !ids.iter().any(|known| known == id) {
        ids.push(id.to_owned());
    }
}

/// Shared by initial plans and edits of the running task graph.
pub(crate) fn validate_task_links(
    spec: &crate::lean::tasks::TaskContract,
    entries: &[LedgerEntry],
) -> Result<()> {
    require(
        !spec.requirement_ids.is_empty(),
        "task needs at least one active requirement",
    )?;
    for id in &spec.requirement_ids {
        require(
            entries
                .iter()
                .any(|e| e.spec.id == *id && e.spec.disposition == Disposition::Active),
            "unknown or inactive task requirement",
        )?;
    }
    for id in &spec.criterion_ids {
        require(
            entries.iter().any(|e| {
                spec.requirement_ids.contains(&e.spec.id)
                    && e.spec.criteria.iter().any(|c| c.id == *id)
            }),
            "task criterion is not owned by a linked requirement",
        )?;
    }
    Ok(())
}

pub(crate) fn validate_criterion_coverage(
    entries: &[LedgerEntry],
    covered: &std::collections::HashSet<String>,
) -> Result<()> {
    for e in entries
        .iter()
        .filter(|e| e.spec.disposition == Disposition::Active && e.spec.mandatory)
    {
        require(
            e.spec.criteria.iter().any(|c| c.mandatory),
            &format!(
                "mandatory requirement {} needs clarification and acceptance criteria",
                e.spec.id
            ),
        )?;
        for c in e.spec.criteria.iter().filter(|c| c.mandatory) {
            require(
                covered.contains(&c.id),
                &format!("mandatory criterion {} has no task", c.id),
            )?;
        }
    }
    Ok(())
}

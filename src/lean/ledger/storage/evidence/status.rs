use super::*;

pub(super) type LedgerStatusTask = (String, String, String, u32, i64, String);

impl Store {
    pub(super) fn ledger_status_tasks(&self, run_id: &str) -> Result<Vec<LedgerStatusTask>> {
        let db = self.db();
        Ok(db.prepare("SELECT t.id,t.spec,t.status,t.round,i.generation,t.plan_id FROM tasks t JOIN task_inputs i ON i.task_id=t.id WHERE i.run_id=?1 ORDER BY t.created_ms DESC,t.rowid DESC")?
            .query_map([run_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?
            .collect::<std::result::Result<_, _>>()?)
    }
}

pub(super) fn set_ledger_status(
    entry: &mut LedgerEntry,
    tasks: &[LedgerStatusTask],
    evidence: &[Evidence],
    fingerprint: &str,
) -> Result<()> {
    if entry.spec.disposition != Disposition::Active {
        entry.status = disposition_status(entry.spec.disposition)?;
        return Ok(());
    }
    let linked = linked_tasks(entry, tasks);
    entry.status = if entry_verified(entry, evidence, tasks, fingerprint) {
        "verified"
    } else {
        linked_status(entry, &linked)
    }
    .into();
    Ok(())
}

fn disposition_status(disposition: Disposition) -> Result<String> {
    Ok(serde_json::to_value(disposition)?.as_str().unwrap().into())
}

fn linked_tasks<'a>(
    entry: &LedgerEntry,
    tasks: &'a [LedgerStatusTask],
) -> Vec<&'a LedgerStatusTask> {
    let mut linked: Vec<_> = tasks
        .iter()
        .filter(|(_, spec, _, _, _, _)| {
            serde_json::from_str::<TaskContract>(spec)
                .is_ok_and(|spec| spec.requirement_ids.contains(&entry.spec.id))
        })
        .collect();
    if let Some(latest) = linked.first().map(|task| task.5.as_str()) {
        linked.retain(|task| task.5 == latest);
    }
    linked
}

fn entry_verified(
    entry: &LedgerEntry,
    evidence: &[Evidence],
    tasks: &[LedgerStatusTask],
    fingerprint: &str,
) -> bool {
    let mandatory: Vec<_> = entry
        .spec
        .criteria
        .iter()
        .filter(|criterion| criterion.mandatory)
        .collect();
    !mandatory.is_empty()
        && mandatory.iter().all(|criterion| {
            criterion_verified(entry, criterion.id.as_str(), evidence, tasks, fingerprint)
        })
}

fn criterion_verified(
    entry: &LedgerEntry,
    criterion: &str,
    evidence: &[Evidence],
    tasks: &[LedgerStatusTask],
    fingerprint: &str,
) -> bool {
    let current: Vec<_> = evidence
        .iter()
        .filter(|item| evidence_matches(entry, criterion, item, tasks, fingerprint))
        .collect();
    !current.is_empty() && current.iter().all(|item| item.approved)
}

fn evidence_matches(
    entry: &LedgerEntry,
    criterion: &str,
    item: &Evidence,
    tasks: &[LedgerStatusTask],
    fingerprint: &str,
) -> bool {
    let proposal = &item.proposal;
    proposal.entry_id == entry.spec.id
        && proposal.entry_version == entry.version
        && proposal.criterion_id == criterion
        && proposal.integrated_fingerprint == fingerprint
        && tasks.iter().any(|(id, _, status, round, generation, _)| {
            *id == proposal.task_id
                && *generation == proposal.generation
                && *round == proposal.attempt
                && status == "approved"
        })
}

fn linked_status(entry: &LedgerEntry, linked: &[&LedgerStatusTask]) -> &'static str {
    if has_task_status(linked, |status| status == "stale") {
        "stale"
    } else if all_approved(linked) {
        "implemented"
    } else if has_task_status(linked, |status| matches!(status, "working" | "verifying")) {
        "in_progress"
    } else if has_task_status(linked, |status| {
        matches!(status, "failed" | "needs_user" | "cancelled" | "waiting")
    }) {
        "blocked"
    } else if !linked.is_empty() {
        "planned"
    } else if entry.spec.criteria.is_empty() {
        "clarification_needed"
    } else {
        "identified"
    }
}

fn has_task_status(linked: &[&LedgerStatusTask], matches: impl Fn(&str) -> bool) -> bool {
    linked.iter().any(|(_, _, status, _, _, _)| matches(status))
}

fn all_approved(linked: &[&LedgerStatusTask]) -> bool {
    !linked.is_empty()
        && linked
            .iter()
            .all(|(_, _, status, _, _, _)| status == "approved")
}

pub(super) fn enforce_dependency_statuses(entries: &mut [LedgerEntry]) {
    loop {
        let invalid: Vec<_> = entries
            .iter()
            .filter(|entry| entry.status == "verified" && has_unverified_dependency(entry, entries))
            .map(|entry| entry.spec.id.clone())
            .collect();
        if invalid.is_empty() {
            break;
        }
        for entry in entries
            .iter_mut()
            .filter(|entry| invalid.contains(&entry.spec.id))
        {
            entry.status = "implemented".into();
        }
    }
}

fn has_unverified_dependency(entry: &LedgerEntry, entries: &[LedgerEntry]) -> bool {
    entries.iter().any(|child| {
        child.spec.disposition == Disposition::Active
            && ((child.spec.mandatory && child.spec.parent.as_deref() == Some(&entry.spec.id))
                || entry.spec.depends_on.contains(&child.spec.id))
            && child.status != "verified"
    })
}

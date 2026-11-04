use super::*;
use std::collections::{HashMap, HashSet, VecDeque};

pub(super) type LedgerStatusTask = (String, String, String, u32, i64, String);

#[derive(Default)]
struct LedgerStatusIndex {
    linked: HashMap<String, LinkedTasks>,
    reviewed_criteria: HashMap<(String, i64, String), EvidenceApproval>,
}

#[derive(Default)]
struct LinkedTasks {
    plan: String,
    rows: Vec<usize>,
}

#[derive(Default)]
struct EvidenceApproval {
    count: usize,
    all_approved: bool,
}

impl Store {
    pub(super) fn ledger_status_tasks(&self, run_id: &str) -> Result<Vec<LedgerStatusTask>> {
        let db = self.db();
        Ok(db.prepare("SELECT t.id,t.spec,t.status,t.round,i.generation,t.plan_id FROM tasks t JOIN task_inputs i ON i.task_id=t.id WHERE i.run_id=?1 ORDER BY t.created_ms DESC,t.rowid DESC")?
            .query_map([run_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?
            .collect::<std::result::Result<_, _>>()?)
    }
}

impl LedgerStatusIndex {
    fn build(tasks: &[LedgerStatusTask], evidence: &[Evidence], fingerprint: &str) -> Self {
        let linked = linked_task_index(tasks);
        let current_tasks = current_approved_tasks(tasks);
        let reviewed_criteria = current_evidence_index(evidence, fingerprint, &current_tasks);
        Self {
            linked,
            reviewed_criteria,
        }
    }

    fn apply(&self, entry: &mut LedgerEntry, tasks: &[LedgerStatusTask]) -> Result<()> {
        if entry.spec.disposition != Disposition::Active {
            entry.status = disposition_status(entry.spec.disposition)?;
            return Ok(());
        }
        let linked = self.linked.get(&entry.spec.id);
        entry.status = if self.entry_reviewed(entry) {
            "reviewed"
        } else {
            linked_status(entry, tasks, linked)
        }
        .into();
        Ok(())
    }

    fn entry_reviewed(&self, entry: &LedgerEntry) -> bool {
        let mandatory = entry
            .spec
            .criteria
            .iter()
            .filter(|criterion| criterion.mandatory);
        let mut found = false;
        for criterion in mandatory {
            found = true;
            let key = (entry.spec.id.clone(), entry.version, criterion.id.clone());
            if !self
                .reviewed_criteria
                .get(&key)
                .is_some_and(|approval| approval.count > 0 && approval.all_approved)
            {
                return false;
            }
        }
        found
    }
}

pub(super) fn set_ledger_statuses(
    entries: &mut [LedgerEntry],
    tasks: &[LedgerStatusTask],
    evidence: &[Evidence],
    fingerprint: &str,
) -> Result<()> {
    let index = LedgerStatusIndex::build(tasks, evidence, fingerprint);
    for entry in entries {
        index.apply(entry, tasks)?;
    }
    Ok(())
}

fn linked_task_index(tasks: &[LedgerStatusTask]) -> HashMap<String, LinkedTasks> {
    let mut linked: HashMap<String, LinkedTasks> = HashMap::new();
    for (index, (_, encoded, _, _, _, plan)) in tasks.iter().enumerate() {
        let Ok(spec) = serde_json::from_str::<TaskContract>(encoded) else {
            continue;
        };
        for requirement in spec.requirement_ids {
            let group = linked.entry(requirement).or_default();
            if group.rows.is_empty() {
                group.plan = plan.clone();
            }
            if group.plan == *plan {
                group.rows.push(index);
            }
        }
    }
    linked
}

fn current_approved_tasks(tasks: &[LedgerStatusTask]) -> HashSet<(String, i64, u32)> {
    tasks
        .iter()
        .filter(|(_, _, status, _, _, _)| status == "approved")
        .map(|(id, _, _, round, generation, _)| (id.clone(), *generation, *round))
        .collect()
}

fn current_evidence_index(
    evidence: &[Evidence],
    fingerprint: &str,
    current_tasks: &HashSet<(String, i64, u32)>,
) -> HashMap<(String, i64, String), EvidenceApproval> {
    let mut current = HashMap::new();
    for item in evidence {
        let proposal = &item.proposal;
        if proposal.integrated_fingerprint != fingerprint
            || !current_tasks.contains(&(
                proposal.task_id.clone(),
                proposal.generation,
                proposal.attempt,
            ))
        {
            continue;
        }
        let key = evidence_key(proposal);
        let approval = current.entry(key).or_insert(EvidenceApproval {
            count: 0,
            all_approved: true,
        });
        approval.count += 1;
        approval.all_approved &= item.approved;
    }
    current
}

fn evidence_key(proposal: &EvidenceProposal) -> (String, i64, String) {
    (
        proposal.entry_id.clone(),
        proposal.entry_version,
        proposal.criterion_id.clone(),
    )
}

fn disposition_status(disposition: Disposition) -> Result<String> {
    Ok(serde_json::to_value(disposition)?.as_str().unwrap().into())
}

fn linked_status(
    entry: &LedgerEntry,
    tasks: &[LedgerStatusTask],
    linked: Option<&LinkedTasks>,
) -> &'static str {
    let Some(linked) = linked else {
        return empty_linked_status(entry);
    };
    let rows = linked.rows.iter().map(|index| &tasks[*index]);
    let mut any = false;
    let mut all_approved = true;
    let mut stale = false;
    let mut working = false;
    let mut blocked = false;
    for (_, _, status, _, _, _) in rows {
        any = true;
        all_approved &= status == "approved";
        stale |= status == "stale";
        working |= matches!(status.as_str(), "working" | "reviewing");
        blocked |= matches!(
            status.as_str(),
            "failed" | "needs_user" | "cancelled" | "waiting"
        );
    }
    choose_linked_status(entry, any, all_approved, stale, working, blocked)
}

fn choose_linked_status(
    entry: &LedgerEntry,
    any: bool,
    all_approved: bool,
    stale: bool,
    working: bool,
    blocked: bool,
) -> &'static str {
    if stale {
        "stale"
    } else if any && all_approved {
        "implemented"
    } else if working {
        "in_progress"
    } else if blocked {
        "blocked"
    } else if any {
        "planned"
    } else {
        empty_linked_status(entry)
    }
}

fn empty_linked_status(entry: &LedgerEntry) -> &'static str {
    if entry.spec.criteria.is_empty() {
        "clarification_needed"
    } else {
        "identified"
    }
}

pub(super) fn enforce_dependency_statuses(entries: &mut [LedgerEntry]) {
    let ids: HashMap<_, _> = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.spec.id.clone(), index))
        .collect();
    let dependents = dependency_dependents(entries, &ids);
    demote_unreviewed_parents(entries, &dependents);
}

fn dependency_dependents(entries: &[LedgerEntry], ids: &HashMap<String, usize>) -> Vec<Vec<usize>> {
    let mut dependents = vec![Vec::new(); entries.len()];
    for (child_index, child) in entries.iter().enumerate() {
        if child.spec.disposition == Disposition::Active
            && child.spec.mandatory
            && let Some(parent_id) = child.spec.parent.as_deref()
            && let Some(&parent_index) = ids.get(parent_id)
        {
            dependents[child_index].push(parent_index);
        }
    }
    for (parent_index, parent) in entries.iter().enumerate() {
        for child in &parent.spec.depends_on {
            if let Some(&child_index) = ids.get(child.as_str())
                && entries[child_index].spec.disposition == Disposition::Active
            {
                dependents[child_index].push(parent_index);
            }
        }
    }
    dependents
}

fn demote_unreviewed_parents(entries: &mut [LedgerEntry], dependents: &[Vec<usize>]) {
    let mut pending: VecDeque<usize> = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| (entry.status != "reviewed").then_some(index))
        .collect();
    while let Some(child) = pending.pop_front() {
        for &parent in &dependents[child] {
            if entries[parent].status == "reviewed" {
                entries[parent].status = "implemented".into();
                pending.push_back(parent);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_statuses_scale_across_entries_and_ignore_stale_fingerprints() {
        let count = 1_000;
        let mut entries = Vec::with_capacity(count);
        let mut tasks = Vec::with_capacity(count);
        let mut evidence = Vec::with_capacity(count);
        for index in 0..count {
            let id = format!("entry-{index}");
            let task_id = format!("task-{index}");
            entries.push(status_entry(&id));
            tasks.push((
                task_id.clone(),
                serde_json::json!({
                    "id": task_id,
                    "title": "Implement",
                    "goal": "Implement requirement",
                    "kind": "implementation",
                    "requirement_ids": [id],
                })
                .to_string(),
                "approved".into(),
                1,
                0,
                "plan".into(),
            ));
            evidence.push(status_evidence(
                index,
                if index % 2 == 0 { "current" } else { "stale" },
            ));
        }

        set_ledger_statuses(&mut entries, &tasks, &evidence, "current").unwrap();
        for (index, entry) in entries.iter().enumerate() {
            assert_eq!(
                entry.status,
                if index % 2 == 0 {
                    "reviewed"
                } else {
                    "implemented"
                }
            );
        }
    }

    fn status_entry(id: &str) -> LedgerEntry {
        LedgerEntry {
            run_id: "run".into(),
            version: 1,
            spec_revision: 1,
            original: id.into(),
            spec: EntrySpec {
                constraints: Vec::new(),
                id: id.into(),
                source: SourceSpan {
                    source_id: "source".into(),
                    start: 0,
                    end: 1,
                },
                interpretation: "Implement".into(),
                qualifiers: Vec::new(),
                parent: None,
                depends_on: Vec::new(),
                mandatory: true,
                criteria: vec![Criterion {
                    id: "criterion".into(),
                    text: "Works".into(),
                    mandatory: true,
                    integration: false,
                }],
                position: 0,
                disposition: Disposition::Active,
            },
            status: "identified".into(),
        }
    }

    fn status_evidence(index: usize, fingerprint: &str) -> Evidence {
        Evidence {
            id: format!("evidence-{index}"),
            run_id: "run".into(),
            proposal: EvidenceProposal {
                entry_id: format!("entry-{index}"),
                entry_version: 1,
                criterion_id: "criterion".into(),
                task_id: format!("task-{index}"),
                generation: 0,
                attempt: 1,
                producer_turn: "producer".into(),
                result: "result".into(),
                artifact: None,
                integrated_fingerprint: fingerprint.into(),
            },
            reviewer_turn: "reviewer".into(),
            assessment: "accepted".into(),
            approved: true,
        }
    }
}

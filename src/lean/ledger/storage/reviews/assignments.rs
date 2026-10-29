use super::super::*;
use crate::lean::store::Task;

fn prior_covered_criteria(store: &Store, prior: &[Task], plan: &str) -> Result<HashSet<String>> {
    let mut covered = HashSet::new();
    for task in prior
        .iter()
        .filter(|task| task.plan_id != plan && task.status == "approved")
    {
        if store.assert_task_inputs(task).is_ok() {
            covered
                .extend(serde_json::from_value::<TaskContract>(task.spec.clone())?.criterion_ids);
        }
    }
    Ok(covered)
}

fn bind_current_tasks(
    tx: &rusqlite::Transaction<'_>,
    run: &str,
    tasks: &[Task],
    all: &[LedgerEntry],
    covered: &mut HashSet<String>,
) -> Result<()> {
    for task in tasks {
        bind_task_inputs(tx, run, task, all, covered)?;
    }
    Ok(())
}

fn bind_task_inputs(
    tx: &rusqlite::Transaction<'_>,
    run: &str,
    task: &Task,
    all: &[LedgerEntry],
    covered: &mut HashSet<String>,
) -> Result<()> {
    let spec: TaskContract = serde_json::from_value(task.spec.clone())?;
    crate::lean::ledger::validate_task_links(&spec, all)?;
    covered.extend(spec.criterion_ids.iter().cloned());
    tx.execute("INSERT INTO task_inputs(task_id,run_id) VALUES(?1,?2) ON CONFLICT(task_id) DO UPDATE SET run_id=excluded.run_id", params![task.id,run])?;
    tx.execute(
        "DELETE FROM task_ledger_inputs WHERE task_id=?1",
        [&task.id],
    )?;
    bind_requirement_inputs(tx, &task.id, &spec.requirement_ids, all)
}

fn bind_requirement_inputs(
    tx: &rusqlite::Transaction<'_>,
    task: &str,
    requirements: &[String],
    all: &[LedgerEntry],
) -> Result<()> {
    for id in requirements.iter().collect::<HashSet<_>>() {
        let entry = all
            .iter()
            .find(|entry| entry.spec.id == *id)
            .ok_or_else(|| invalid("unknown ledger task input"))?;
        tx.execute(
            "INSERT INTO task_ledger_inputs VALUES(?1,?2,?3)",
            params![task, id, entry.version],
        )?;
    }
    Ok(())
}

fn validate_assignments(all: &[LedgerEntry], current: &[Task], prior: &[Task]) -> Result<()> {
    let approved: Vec<_> = prior
        .iter()
        .filter(|task| task.status == "approved")
        .collect();
    for entry in all
        .iter()
        .filter(|entry| entry.spec.disposition == Disposition::Active)
    {
        for assigned in &entry.spec.task_ids {
            require(
                assignment_is_present(assigned, &entry.spec.id, current, &approved),
                &format!(
                    "ledger assignment {} -> {assigned} is not honored by the task plan",
                    entry.spec.id
                ),
            )?;
        }
    }
    Ok(())
}

fn assignment_is_present(
    assigned: &str,
    requirement: &str,
    current: &[Task],
    prior: &[&Task],
) -> bool {
    current.iter().chain(prior.iter().copied()).any(|task| {
        (task.id == assigned || task.spec["id"].as_str() == Some(assigned))
            && serde_json::from_value::<TaskContract>(task.spec.clone())
                .is_ok_and(|spec| spec.requirement_ids.iter().any(|id| id == requirement))
    })
}

impl Store {
    pub fn bind_task_ledger(&self, conversation: &str, run_id: &str, plan: &str) -> Result<()> {
        let all = self.ledger_entries(conversation, run_id, None)?;
        let tasks = self.plan(plan)?;
        let prior = self.run_tasks(conversation, run_id)?;
        let mut covered = prior_covered_criteria(self, &prior, plan)?;
        let mut db = self.db();
        let tx = db.transaction()?;
        bind_current_tasks(&tx, run_id, &tasks, &all, &mut covered)?;
        validate_assignments(&all, &tasks, &prior)?;
        crate::lean::ledger::validate_criterion_coverage(&all, &covered)?;
        tx.commit()?;
        Ok(())
    }
}

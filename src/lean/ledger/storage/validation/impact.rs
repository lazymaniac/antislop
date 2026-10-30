use super::super::*;

pub(in crate::lean::ledger::storage) fn impact(
    db: &Connection,
    run: &str,
    changed: &HashSet<String>,
    before: &[EntrySpec],
    all: &[EntrySpec],
) -> Result<Vec<String>> {
    let reach: HashMap<String, Reach> = impacted_entries(changed, all)
        .into_iter()
        .map(|id| {
            let reach = reach_of(&id, changed, before, all);
            (id, reach)
        })
        .collect();
    let mut tasks = tasks_for_entries(db, run, &reach)?;
    expand_dependent_tasks(db, run, &mut tasks)?;
    let mut out: Vec<_> = tasks.into_iter().collect();
    out.sort();
    Ok(out)
}

/// Whether two versions of an entry ask for the same thing. The quoted span,
/// position, owners and linked tasks are bookkeeping; a resubmission that
/// only moves them changes nothing the approved work was built against.
pub(in crate::lean::ledger::storage) fn same_meaning(old: &EntrySpec, new: &EntrySpec) -> bool {
    old.interpretation == new.interpretation
        && old.qualifiers == new.qualifiers
        && old.parent == new.parent
        && old.depends_on == new.depends_on
        && old.mandatory == new.mandatory
        && old.criteria == new.criteria
        && old.disposition == new.disposition
}

/// Entries whose meaning a change reaches: the changed ones and, downward,
/// those that depend on them or sit under them. A parent is not reached by
/// a new or edited child; the child is its own entry with its own tasks.
fn impacted_entries(changed: &HashSet<String>, all: &[EntrySpec]) -> HashSet<String> {
    let mut ids = changed.clone();
    loop {
        let n = ids.len();
        for e in all {
            if e.depends_on
                .iter()
                .chain(e.parent.iter())
                .any(|id| ids.contains(id))
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

/// Tasks a change no longer reaches: already stale, or of a plan replaced,
/// sent back or cancelled. Marking them again only repeats what is known.
const RETIRED: &str = "('stale','superseded','revised','cancelled')";

/// How far a change to an entry reaches the tasks linked to it.
enum Reach {
    /// Its meaning changed, or a change above it reached it.
    Whole,
    /// Only these criteria changed, among all the entry had or has: a task
    /// covering other criteria of it was built against nothing that changed.
    Criteria {
        changed: HashSet<String>,
        all: HashSet<String>,
    },
}

fn reach_of(id: &str, changed: &HashSet<String>, before: &[EntrySpec], all: &[EntrySpec]) -> Reach {
    let old = before.iter().find(|entry| entry.id == id);
    let new = all.iter().find(|entry| entry.id == id);
    let (true, Some(old), Some(new)) = (changed.contains(id), old, new) else {
        return Reach::Whole;
    };
    let mut rest = new.clone();
    rest.criteria = old.criteria.clone();
    if !same_meaning(old, &rest) {
        return Reach::Whole;
    }
    let ids: HashSet<String> = old
        .criteria
        .iter()
        .chain(&new.criteria)
        .map(|criterion| criterion.id.clone())
        .collect();
    let find = |list: &[Criterion], id: &str| list.iter().find(|c| c.id == id).cloned();
    Reach::Criteria {
        changed: ids
            .iter()
            .filter(|id| find(&old.criteria, id) != find(&new.criteria, id))
            .cloned()
            .collect(),
        all: ids,
    }
}

fn tasks_for_entries(
    db: &Connection,
    run: &str,
    reach: &HashMap<String, Reach>,
) -> Result<HashSet<String>> {
    let mut tasks = HashSet::new();
    let mut stmt = db.prepare(&format!(
        "SELECT i.task_id,l.entry_id,t.spec FROM task_inputs i JOIN task_ledger_inputs l ON i.task_id=l.task_id JOIN tasks t ON t.id=i.task_id WHERE i.run_id=?1 AND t.status NOT IN {RETIRED}"
    ))?;
    for row in stmt.query_map([run], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })? {
        let (task, id, spec) = row?;
        if task_is_reached(reach.get(&id), &spec) {
            tasks.insert(task);
        }
    }
    Ok(tasks)
}

fn task_is_reached(reach: Option<&Reach>, spec: &str) -> bool {
    match reach {
        None => false,
        Some(Reach::Whole) => true,
        Some(Reach::Criteria { changed, all }) => {
            let covered: Vec<String> = serde_json::from_str::<TaskContract>(spec)
                .map(|spec| spec.criterion_ids)
                .unwrap_or_default()
                .into_iter()
                .filter(|criterion| all.contains(criterion))
                .collect();
            covered.is_empty() || covered.iter().any(|criterion| changed.contains(criterion))
        }
    }
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
        .prepare(&format!("SELECT t.id,t.plan_id,t.spec FROM tasks t JOIN plan_runs p ON p.plan_id=t.plan_id WHERE p.run_id=?1 AND t.status NOT IN {RETIRED}"))?
        .query_map([run], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<std::result::Result<_, _>>()?)
}

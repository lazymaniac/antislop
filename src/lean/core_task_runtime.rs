//! Durable, task-owned runtime files live outside disposable Git checkouts.
use super::*;

pub(super) fn directory(state: &Path, task: &Task) -> Result<PathBuf> {
    let id = super::core_path_helpers::task_path_component(&task.id)?;
    let root = state.join("task-state");
    let owner = root.join(id);
    let generation = owner.join(format!("g{}", task.input_generation));
    for path in [&root, &owner, &generation] {
        daemon::private_directory(path)?;
    }
    Ok(generation)
}

pub(super) fn sources(core: &Core, task: &Task) -> Result<Vec<serde_json::Value>> {
    let mut pending = vec![task.clone()];
    let contract: tasks::TaskContract = serde_json::from_value(task.spec.clone())?;
    for id in contract.depends_on {
        if let Some(dependency) = core.store.task_in_plan_by_id(&task.plan_id, &id)? {
            pending.push(dependency);
        }
    }
    let conversation = core.store.plan_conversation(&task.plan_id)?;
    let mut visited = std::collections::BTreeSet::new();
    let mut sources = Vec::new();
    while let Some(owner) = pending.pop() {
        if !visited.insert(owner.id.clone()) {
            continue;
        }
        require(
            visited.len() <= 64,
            "runtime handoff exceeds 64 owners; narrow the task dependencies",
        )?;
        require(
            core.store.plan_conversation(&owner.plan_id)? == conversation,
            "runtime handoff belongs to another conversation",
        )?;
        let id = super::core_path_helpers::task_path_component(&owner.id)?;
        let root = core.state.join("task-state").join(id);
        if owner.id != task.id {
            let available = match root.symlink_metadata() {
                Ok(_) => {
                    daemon::private_directory(&root)?;
                    true
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                Err(error) => return Err(error.into()),
            };
            sources.push(json!({"task":owner.id,"directory":root,"available":available,"access":"read-only"}));
        }
        let prior = {
            let db = core.store.db();
            let mut query = db.prepare("SELECT DISTINCT source_task FROM task_resume_results WHERE task_id=?1 AND generation<=?2 AND source_task<>?1")?;
            query
                .query_map(rusqlite::params![owner.id, owner.input_generation], |r| {
                    r.get::<_, String>(0)
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        for id in prior {
            pending.push(
                core.store
                    .task_by_id(&id)?
                    .ok_or_else(|| Error::Invalid("runtime handoff source disappeared".into()))?,
            );
        }
    }
    Ok(sources)
}

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

/// An approved task keeps only the generation that produced its result. A
/// generation directory is removed only when the approved one exists, and a
/// symlink or a name the runtime does not allocate is never touched.
pub(super) fn prune_settled_generations(state: &Path, task: &Task) -> Result<()> {
    if task.status != tasks::status::APPROVED {
        return Ok(());
    }
    let id = super::core_path_helpers::task_path_component(&task.id)?;
    let owner = state.join("task-state").join(id);
    let keep = format!("g{}", task.input_generation);
    if !owner.join(&keep).is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(&owner)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let generation = name
            .strip_prefix('g')
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
        if generation && name != keep && entry.file_type()?.is_dir() {
            std::fs::remove_dir_all(entry.path())?;
        }
    }
    Ok(())
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
            sources.push(json!({"task":owner.id,"directory":root,"available":available}));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_sources_support_more_than_sixty_four_dependencies() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::from_toml(include_str!("../../examples/antislop.toml")).unwrap();
        let core = Core::open(&dir.path().join("state"), config).unwrap();
        let conversation = core
            .store
            .create_conversation(dir.path().to_str().unwrap())
            .unwrap()
            .id;
        let dependencies: Vec<_> = (0..70).map(|index| format!("source-{index}")).collect();
        let mut specs: Vec<_> = dependencies.iter().map(|id| json!({
            "id":id,"title":"Source","goal":"Provide evidence","kind":"analysis","change_kind":"none"
        })).collect();
        specs.push(
            json!({"id":"consumer","title":"Consumer","goal":"Assess evidence", "kind":"analysis",
            "change_kind":"none","depends_on":dependencies}),
        );
        let plan = core.store.add_plan(&conversation, &specs).unwrap();
        let consumer = plan
            .iter()
            .find(|task| task.spec["id"] == "consumer")
            .unwrap();
        let result = sources(&core, consumer).unwrap();
        assert_eq!(result.len(), 70);
        let owners: std::collections::BTreeSet<_> = result
            .iter()
            .map(|source| source["task"].as_str().unwrap())
            .collect();
        assert_eq!(owners.len(), 70);
    }
}

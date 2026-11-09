//! Durable, task-owned runtime files live outside disposable Git checkouts.
use crate::lean::*;

pub(in crate::lean) fn directory(state: &Path, task: &Task) -> Result<PathBuf> {
    let id = crate::lean::core::worktree::paths::task_path_component(&task.id)?;
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
pub(in crate::lean) fn prune_settled_generations(state: &Path, task: &Task) -> Result<()> {
    if task.status != tasks::status::APPROVED {
        return Ok(());
    }
    let id = crate::lean::core::worktree::paths::task_path_component(&task.id)?;
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

/// What a round-end sweep removed from a task's runtime allocation.
#[derive(Debug, Default, PartialEq)]
pub(in crate::lean) struct Sweep {
    pub(in crate::lean) removed: Vec<PathBuf>,
    pub(in crate::lean) bytes: u64,
    pub(in crate::lean) errors: Vec<String>,
}

/// Removes runtime entries that the settled round left uncited. An entry is
/// kept when the citation text names its exact path (absolute, or relative to
/// the task or generation directory) or names it standalone; a directory that
/// only contains a cited path is descended so its uncited siblings can go. An
/// entry with anything modified at or after `cutoff` (the settled round's
/// start) is kept, so a round's own output survives until the next round
/// settles without citing it. Symlinks and names outside `g<N>` are never
/// touched. Failures are collected, not raised: a sweep never fails a round.
pub(in crate::lean) fn sweep_uncited(
    state: &Path,
    task: &str,
    citations: &str,
    cutoff: std::time::SystemTime,
) -> Result<Sweep> {
    let id = crate::lean::core::worktree::paths::task_path_component(task)?;
    let owner = state.join("task-state").join(id);
    let mut sweep = Sweep::default();
    let entries = match std::fs::read_dir(&owner) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(sweep),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let generation = name
            .strip_prefix('g')
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
        if generation && entry.file_type()?.is_dir() {
            sweep_directory(&owner, &entry.path(), citations, cutoff, &mut sweep);
        }
    }
    Ok(sweep)
}

fn sweep_directory(
    owner: &Path,
    directory: &Path,
    citations: &str,
    cutoff: std::time::SystemTime,
    sweep: &mut Sweep,
) {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            sweep
                .errors
                .push(format!("{}: {error}", directory.display()));
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        let citation = cited(owner, &path, citations);
        if citation.exact {
            continue;
        }
        if citation.ancestor && kind.is_dir() {
            sweep_directory(owner, &path, citations, cutoff, sweep);
            continue;
        }
        let Some((newest, bytes)) = newest_modification(&path) else {
            continue;
        };
        if newest >= cutoff {
            continue;
        }
        let removed = if kind.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        match removed {
            Ok(()) => {
                sweep.removed.push(path);
                sweep.bytes += bytes;
            }
            Err(error) => sweep.errors.push(format!("{}: {error}", path.display())),
        }
    }
}

#[derive(Default)]
struct Citation {
    exact: bool,
    ancestor: bool,
}

fn is_path_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '.' | '_' | '-' | '/' | '~' | '+' | '=' | '@')
}

fn cited(owner: &Path, path: &Path, text: &str) -> Citation {
    let mut citation = Citation::default();
    let mut needles = vec![path.to_string_lossy().into_owned()];
    if let Ok(relative) = path.strip_prefix(owner) {
        let mut components = relative.components();
        needles.push(relative.to_string_lossy().into_owned());
        components.next();
        let below = components.as_path().to_string_lossy().into_owned();
        if below.contains('/') {
            needles.push(below);
        }
    }
    for needle in &needles {
        scan(text, needle, &mut citation);
    }
    if let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) {
        let mut standalone = Citation::default();
        scan(text, &name, &mut standalone);
        citation.exact |= standalone.exact;
    }
    citation
}

/// Classifies every occurrence of `needle` that starts at a path boundary:
/// followed by `/` and more path text it is an ancestor of a cited path,
/// otherwise (end of text, punctuation or a trailing `/`) it is cited itself.
fn scan(text: &str, needle: &str, citation: &mut Citation) {
    if needle.is_empty() {
        return;
    }
    for (start, _) in text.match_indices(needle) {
        if text[..start].chars().next_back().is_some_and(is_path_char) {
            continue;
        }
        let mut rest = text[start + needle.len()..].chars();
        match rest.next() {
            None => citation.exact = true,
            Some('/') => match rest.next() {
                Some(c) if is_path_char(c) && c != '/' => citation.ancestor = true,
                _ => citation.exact = true,
            },
            Some('.') | Some(',') => {
                if !rest.next().is_some_and(is_path_char) {
                    citation.exact = true;
                }
            }
            Some(c) if !is_path_char(c) => citation.exact = true,
            Some(_) => {}
        }
    }
}

/// The newest modification time and total size below `path`, without
/// following symlinks. `None` when anything cannot be inspected.
fn newest_modification(path: &Path) -> Option<(std::time::SystemTime, u64)> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    let mut newest = metadata.modified().ok()?;
    let mut bytes = metadata.len();
    if metadata.is_dir() {
        for entry in std::fs::read_dir(path).ok()? {
            let (time, size) = newest_modification(&entry.ok()?.path())?;
            newest = newest.max(time);
            bytes += size;
        }
    }
    Some((newest, bytes))
}

/// Everything that can cite a task's runtime files once a round settles: the
/// round's report and verdict, the contracts, reports and verdicts of the
/// plan's tasks, and the task's evidence for its current inputs.
pub(in crate::lean) fn citations(
    core: &Core,
    task: &Task,
    report: &str,
    verdict: &str,
) -> Result<String> {
    let mut text = format!("{report}\n{verdict}\n");
    for other in core.store.plan(&task.plan_id)? {
        text.push_str(&other.spec.to_string());
        text.push('\n');
        for part in [&other.report, &other.result].into_iter().flatten() {
            text.push_str(part);
            text.push('\n');
        }
        for part in [&other.verdict, &other.feedback].into_iter().flatten() {
            text.push_str(&part.to_string());
            text.push('\n');
        }
    }
    for evidence in core
        .store
        .task_evidence_context(&task.id, task.input_generation)?
    {
        text.push_str(&core.store.evidence_text(&evidence)?);
        text.push('\n');
    }
    Ok(text)
}

pub(in crate::lean) fn sources(core: &Core, task: &Task) -> Result<Vec<serde_json::Value>> {
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
        let id = crate::lean::core::worktree::paths::task_path_component(&owner.id)?;
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

    fn write(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"data").unwrap();
    }

    #[test]
    fn sweep_keeps_cited_and_recent_entries_and_removes_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path();
        let generation = state.join("task-state/task-0001/g2");
        let scout = generation.join("scout");
        for run in ["run-1", "run-2", "run-3", "run-30", "named"] {
            write(&scout.join(run).join("journal"));
        }
        write(&scout.join("run-4/reference/journal"));
        write(&scout.join("run-4/reference/summary.txt"));
        write(&state.join("task-state/task-0001/g1/old/journal"));
        write(&state.join("task-state/task-0001/notes/keep"));
        std::os::unix::fs::symlink(scout.join("run-1"), scout.join("link")).unwrap();
        let cutoff = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        let citations = format!(
            "Kept {}/run-2.\nSee `g2/scout/run-3/` and {}/run-4/reference/summary.txt; the named run.",
            scout.display(),
            scout.display()
        );
        let sweep = sweep_uncited(state, "task-0001", &citations, cutoff).unwrap();
        assert!(sweep.errors.is_empty(), "{:?}", sweep.errors);
        assert!(!scout.join("run-1").exists());
        assert!(scout.join("run-2/journal").exists());
        assert!(scout.join("run-3/journal").exists());
        assert!(!scout.join("run-30").exists(), "run-3 must not cite run-30");
        assert!(
            scout.join("named/journal").exists(),
            "a standalone name keeps it"
        );
        assert!(scout.join("run-4/reference/summary.txt").exists());
        assert!(!scout.join("run-4/reference/journal").exists());
        assert!(!state.join("task-state/task-0001/g1/old").exists());
        assert!(state.join("task-state/task-0001/notes/keep").exists());
        assert!(std::fs::symlink_metadata(scout.join("link")).is_ok());
        assert_eq!(sweep.removed.len(), 4);
        // Four removed journals of four bytes each, plus directory metadata.
        assert!(sweep.bytes >= 16);
    }

    #[test]
    fn sweep_keeps_entries_modified_during_the_settled_round() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path();
        let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
        let run = state.join("task-state/task-0001/g0/run-new/journal");
        write(&run);
        let sweep = sweep_uncited(state, "task-0001", "", cutoff).unwrap();
        assert!(sweep.removed.is_empty());
        assert!(run.exists());
    }

    #[test]
    fn runtime_sources_support_more_than_sixty_four_dependencies() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::from_toml(include_str!("../../../../examples/antislop.toml")).unwrap();
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

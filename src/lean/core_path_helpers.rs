use super::*;

/// Validate database task IDs before using them as path or branch components.
pub(super) fn task_path_component(id: &str) -> Result<&str> {
    if id.len() < 8
        || id.starts_with("combine-")
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(Error::Invalid(format!(
            "invalid task ID {:?}: expected at least 8 ASCII letters, digits, hyphens or underscores, excluding reserved combine- prefix",
            id.chars().take(32).collect::<String>()
        )));
    }
    Ok(id)
}

/// Each input generation owns a distinct checkout and branch.
pub(super) fn worktree_path(state: &Path, task: &Task) -> Result<PathBuf> {
    let id = task_path_component(&task.id)?;
    Ok(state
        .join("worktrees")
        .join(format!("{id}-g{}", task.input_generation)))
}

pub(super) fn branch(task: &Task) -> Result<String> {
    Ok(format!(
        "antislop/{}-g{}",
        task_path_component(&task.id)?,
        task.input_generation
    ))
}

pub(super) fn worktree_branch(state: &Path, task: &Task) -> Result<String> {
    let path = worktree_path(state, task)?;
    let expected = branch(task)?;
    if path.exists() {
        let actual = git::worktree_branch(&path)?;
        require(
            actual == expected,
            &format!("task worktree is on {actual}; expected {expected}"),
        )?;
    }
    Ok(expected)
}

/// The kinds of checkout a task only reads through: a verifier's view of a
/// round's result and a non-writer's view of the plan base. Each shows a
/// commit another branch or the plan already holds.
const VIEWS: [&str; 2] = ["review", "observation"];

/// Removes `task_id`'s review and observation checkouts and their branches,
/// apart from `keep`. A session that worked in one is replaced when its next
/// turn needs another directory, so only the current one is still in use.
pub(super) fn release_views(state: &Path, task_id: &str, keep: Option<&Path>) -> Result<()> {
    let id = task_path_component(task_id)?;
    let Ok(entries) = std::fs::read_dir(state.join("worktrees")) else {
        return Ok(());
    };
    let mut first_error = None;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let view = VIEWS
            .iter()
            .any(|kind| name.starts_with(&format!("{id}-{kind}-g")));
        if !view || keep == Some(path.as_path()) {
            continue;
        }
        if let Err(error) = release_view(&path, &name) {
            first_error.get_or_insert(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

fn release_view(path: &Path, name: &str) -> Result<()> {
    let project = git::worktree_project(path)?;
    git::remove_worktree(&project, path, &format!("antislop/{name}"))
}

/// Views a restart finds for tasks that are settled or gone, their branches
/// left without a checkout, and the worktree records whose directories were
/// removed by hand.
pub(super) fn release_settled_views(store: &Store, state: &Path) -> Result<()> {
    let mut projects: std::collections::BTreeSet<PathBuf> = store
        .conversations()?
        .into_iter()
        .map(|conversation| PathBuf::from(conversation.project))
        .filter(|project| git::is_repository(project))
        .collect();
    if let Ok(entries) = std::fs::read_dir(state.join("worktrees")) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(task) = view_task(&name) else {
                continue;
            };
            if let Ok(project) = git::worktree_project(&entry.path()) {
                projects.insert(project);
            }
            if settled(store, task)? {
                let _ = release_view(&entry.path(), &name);
            }
        }
    }
    for project in projects {
        let _ = git::prune_worktrees(&project);
        for branch in git::branches(&project, "antislop/").unwrap_or_default() {
            let name = branch.trim_start_matches("antislop/");
            if let Some(task) = view_task(name)
                && settled(store, task)?
            {
                // Git refuses to delete a branch a worktree still has out.
                let _ = git::delete_branch(&project, &branch);
            }
        }
    }
    Ok(())
}

fn view_task(name: &str) -> Option<&str> {
    VIEWS
        .iter()
        .find_map(|kind| name.split_once(&format!("-{kind}-g")).map(|(id, _)| id))
}

/// A task that will not run in its current generation again; a later run
/// makes its own views.
fn settled(store: &Store, task: &str) -> Result<bool> {
    Ok(store.task_by_id(task)?.is_none_or(|task| {
        matches!(
            task.status.as_str(),
            status::APPROVED | status::CANCELLED | status::SKIPPED | status::STALE | "superseded"
        )
    }))
}

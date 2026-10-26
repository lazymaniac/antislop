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

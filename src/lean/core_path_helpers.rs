use super::*;

/// Validate database task IDs before using them as path or branch components.
/// Keep the original spelling so existing worktrees can be resumed.
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

/// New writers use the complete validated ID, so shared prefixes cannot collide.
pub(super) fn branch(task: &Task) -> Result<String> {
    Ok(format!("antislop/{}", task_path_component(&task.id)?))
}

/// Existing worktrees retain their actual branch, including legacy short names.
pub(super) fn worktree_branch(state: &Path, task: &Task) -> Result<String> {
    let id = task_path_component(&task.id)?;
    let path = state.join("worktrees").join(id);
    let expected = branch(task)?;
    if path.exists() {
        let actual = git::worktree_branch(&path)?;
        let legacy = format!("antislop/{}", &id[..8]);
        if actual != expected && actual != legacy {
            return Err(Error::Invalid(format!(
                "task {} worktree is on unrelated branch {actual}; expected {expected} or legacy {legacy}",
                task.id
            )));
        }
        Ok(actual)
    } else {
        Ok(expected)
    }
}

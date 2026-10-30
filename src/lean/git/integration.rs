mod files;
use super::*;
use files::{blobs, unchanged_since, write_files};
pub fn integrate_scoped(
    project: &Path,
    base: &str,
    change: &str,
    allowed_files: Option<&[String]>,
    commit: bool,
) -> Result<Integration> {
    let files = changed_files(project, base, change)?;
    let target_files = target_paths(&files, allowed_files);
    if target_files.is_empty() {
        return Ok(Integration::Applied);
    }
    if let Some(result) = previously_landed(project, change, &target_files, commit)? {
        return Ok(result);
    }
    if !commit && unchanged_since(project, base, &target_files)? {
        write_files(project, change, &target_files)?;
        return Ok(Integration::Applied);
    }
    integrate_current(
        project,
        base,
        change,
        &files,
        &target_files,
        allowed_files.is_none(),
        commit,
    )
}

fn target_paths(files: &[String], allowed: Option<&[String]>) -> Vec<String> {
    files
        .iter()
        .filter(|file| allowed.is_none_or(|paths| paths.contains(file)))
        .cloned()
        .collect()
}

fn previously_landed(
    project: &Path,
    change: &str,
    files: &[String],
    commit: bool,
) -> Result<Option<Integration>> {
    if !unchanged_since(project, change, files)? {
        return Ok(None);
    }
    if !commit {
        return Ok(Some(Integration::Applied));
    }
    if blobs(project, "HEAD", files)? == blobs(project, change, files)? {
        return Ok(Some(Integration::Committed(head(project)?)));
    }
    Ok(None)
}

struct MergePlan {
    dirty: Vec<String>,
    tree: String,
    committed_base: String,
}

fn integrate_current(
    project: &Path,
    base: &str,
    change: &str,
    files: &[String],
    target: &[String],
    unrestricted: bool,
    commit: bool,
) -> Result<Integration> {
    let dirty = uncommitted(project)?;
    let onto = if dirty.is_empty() {
        head(project)?
    } else {
        snapshot(project, "Working tree before integration")?
    };
    let committed_base = common_base(project, base, "HEAD")?;
    let (tree, conflict) = merge_tree(project, base, change, &onto, target, &dirty)?;
    if let Some(conflict) = conflict {
        return Ok(conflict);
    }
    let plan = MergePlan {
        dirty,
        tree,
        committed_base,
    };
    if can_cherry_pick(&plan, base, files, target, unrestricted) {
        return cherry_pick(project, change, files, commit);
    }
    write_merge_result(project, change, target, &plan.tree, commit)
}

fn merge_tree(
    project: &Path,
    base: &str,
    change: &str,
    onto: &str,
    target: &[String],
    dirty: &[String],
) -> Result<(String, Option<Integration>)> {
    let check = run_merge_tree(project, base, change, onto)?;
    if let Some(result) = conflict_result(&check, target, dirty) {
        return Ok((String::new(), Some(result)));
    }
    if !check.status.success() && check.status.code() != Some(1) {
        return Err(Error::Invalid(format!(
            "git merge-tree: {}",
            String::from_utf8_lossy(&check.stderr).trim()
        )));
    }
    let tree = merge_tree_result(&check.stdout)?;
    Ok((tree, None))
}

fn run_merge_tree(
    project: &Path,
    base: &str,
    change: &str,
    onto: &str,
) -> Result<std::process::Output> {
    let merge_base = format!("--merge-base={base}");
    git(
        project,
        &[
            "merge-tree",
            "--write-tree",
            "--name-only",
            &merge_base,
            onto,
            change,
        ],
    )
}

fn merge_tree_result(stdout: &[u8]) -> Result<String> {
    std::str::from_utf8(stdout)
        .map_err(|error| Error::Invalid(format!("git merge-tree: {error}")))?
        .lines()
        .next()
        .map(str::to_owned)
        .ok_or_else(|| Error::Invalid("git merge-tree returned no tree".into()))
}

fn conflict_result(
    check: &std::process::Output,
    target: &[String],
    dirty: &[String],
) -> Option<Integration> {
    if check.status.code() != Some(1) {
        return None;
    }
    let conflicts = String::from_utf8_lossy(&check.stdout)
        .lines()
        .skip(1)
        .take_while(|line| !line.is_empty())
        .filter(|path| target.contains(&path.to_string()))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if conflicts.is_empty() {
        return None;
    }
    Some(if conflicts.iter().any(|path| dirty.contains(path)) {
        Integration::UserEdits(conflicts)
    } else {
        Integration::Conflict(conflicts)
    })
}

fn can_cherry_pick(
    plan: &MergePlan,
    base: &str,
    files: &[String],
    target: &[String],
    unrestricted: bool,
) -> bool {
    plan.dirty.is_empty()
        && unrestricted
        && target.len() == files.len()
        && plan.committed_base == base
}

fn cherry_pick(
    project: &Path,
    change: &str,
    files: &[String],
    commit: bool,
) -> Result<Integration> {
    if commit {
        ok(
            project,
            &[
                "-c",
                "user.name=antislop",
                "-c",
                "user.email=antislop@localhost",
                "cherry-pick",
                "--allow-empty",
                change,
            ],
        )?;
        return Ok(Integration::Committed(head(project)?));
    }
    ok(project, &["cherry-pick", "--no-commit", change])?;
    unstage_paths(project, files);
    Ok(Integration::Applied)
}

fn unstage_paths(project: &Path, files: &[String]) {
    let mut args = vec!["--literal-pathspecs", "reset", "--quiet", "--"];
    args.extend(files.iter().map(String::as_str));
    let _ = git(project, &args);
}

fn write_merge_result(
    project: &Path,
    change: &str,
    target: &[String],
    tree: &str,
    commit: bool,
) -> Result<Integration> {
    write_files(project, tree, target)?;
    if !commit {
        return Ok(Integration::Applied);
    }
    let message = ok(project, &["log", "-1", "--format=%B", change])?;
    Ok(Integration::Committed(commit_paths(
        project,
        target,
        message.trim(),
    )?))
}

/// Rebuild a repaired result on the current project snapshot, carrying only
/// the writer's changed paths. `worktree` must be the allocated task worktree;
/// unrelated stale files in it must never revert concurrent project edits.
pub fn transplant_repair(worktree: &Path, base: &str, onto: &str, message: &str) -> Result<String> {
    let repaired = commit_all(worktree, message)?;
    let files = changed_files(worktree, base, &repaired)?;
    ok(worktree, &["reset", "--hard", onto])?;
    write_files(worktree, &repaired, &files)?;
    commit_all(worktree, message)
}

/// Applies the squashed commit `change` (made on `base`) to the project's
/// current branch.
pub fn integrate(project: &Path, base: &str, change: &str, commit: bool) -> Result<Integration> {
    integrate_scoped(project, base, change, None, commit)
}

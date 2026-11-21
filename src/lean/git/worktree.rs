use super::*;
pub fn is_repository(dir: &Path) -> bool {
    git(dir, &["rev-parse", "--is-inside-work-tree"]).is_ok_and(|output| output.status.success())
}

pub fn session_branch(dir: &Path) -> Result<String> {
    ok(dir, &["symbolic-ref", "--quiet", "--short", "HEAD"]).or_else(|_| head(dir))
}

/// Read only a symbolic branch from an actual worktree, never a detached SHA
/// or the branch inherited by Git discovery from a parent directory.
pub fn worktree_branch(path: &Path) -> Result<String> {
    if !path.join(".git").is_file() {
        return Err(Error::Invalid(format!(
            "not a linked worktree: {}",
            path.display()
        )));
    }
    ok(path, &["symbolic-ref", "--quiet", "--short", "HEAD"])
}

fn require_worktree_branch(path: &Path, expected: &str) -> Result<()> {
    let actual = worktree_branch(path)?;
    if actual != expected {
        return Err(Error::Invalid(format!(
            "worktree branch {actual} differs from expected branch {expected}"
        )));
    }
    Ok(())
}

pub fn head(dir: &Path) -> Result<String> {
    ok(dir, &["rev-parse", "HEAD"])
}

pub fn tree_identity(dir: &Path, commit: &str) -> Result<String> {
    ok(dir, &["rev-parse", &format!("{commit}^{{tree}}")])
}

pub fn common_base(dir: &Path, left: &str, right: &str) -> Result<String> {
    ok(dir, &["merge-base", left, right])
}

/// A worktree on a new branch `branch` at `base`; reused if it exists.
pub fn add_worktree(project: &Path, path: &Path, branch: &str, base: &str) -> Result<()> {
    if path.exists() {
        let expected_project = worktree_project(project)?.canonicalize()?;
        let actual_project = worktree_project(path)?.canonicalize()?;
        if actual_project != expected_project {
            return Err(Error::Invalid(format!(
                "worktree {} belongs to a different project",
                path.display()
            )));
        }
        require_worktree_branch(path, branch)?;
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let target = path.to_string_lossy();
    let reference = format!("refs/heads/{branch}");
    if git(project, &["show-ref", "--verify", "--quiet", &reference])?
        .status
        .success()
    {
        // Reattach a preserved branch at its current tip. `git worktree add`
        // enforces that the branch is not already checked out elsewhere.
        ok(project, &["worktree", "add", "--quiet", &target, branch])?;
    } else {
        ok(
            project,
            &["worktree", "add", "--quiet", "-b", branch, &target, base],
        )?;
    }
    Ok(())
}

pub fn remove_worktree(project: &Path, path: &Path, branch: &str) -> Result<()> {
    require_worktree_branch(path, branch)?;
    let target = path.to_string_lossy();
    ok(project, &["worktree", "remove", "--force", &target])?;
    let _ = git(project, &["branch", "-D", branch]);
    Ok(())
}

/// Keep an exact, lightweight history reference before deleting a clean view.
/// Normal removal refuses tracked/untracked edits, including changes racing the check;
/// ignored files go with the checkout, so a retired one leaves nothing on disk.
pub(crate) fn remove_clean_view(project: &Path, path: &Path, branch: &str) -> Result<()> {
    require_worktree_branch(path, branch)?;
    crate::require(
        is_clean(path),
        &format!(
            "worktree {} has uncommitted files; preserved",
            path.display()
        ),
    )?;
    let snapshot = view_snapshot_ref(path);
    let commit = head(path)?;
    ok(project, &["update-ref", &snapshot, &commit])?;
    ok(project, &["worktree", "remove", &path.to_string_lossy()])?;
    delete_branch(project, branch)
}

pub(crate) fn restore_clean_view(project: &Path, path: &Path) -> Result<()> {
    let snapshot = view_snapshot_ref(path);
    let commit = ok(project, &["rev-parse", "--verify", &snapshot])?;
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| Error::Invalid("invalid view path".into()))?;
    add_worktree(project, path, &format!("antislop/{name}"), &commit)
}

fn view_snapshot_ref(path: &Path) -> String {
    format!(
        "refs/antislop/view-history/{}",
        crate::hash(path.as_os_str().as_encoded_bytes())
    )
}

/// Remove only the checkout of retired writer work; its branch retains all commits.
/// Declared outputs must be committed; ignored files go with the checkout.
pub(crate) fn remove_clean_writer(
    project: &Path,
    path: &Path,
    branch: &str,
    outputs: &[String],
) -> Result<()> {
    require_worktree_branch(path, branch)?;
    for output in outputs {
        crate::require(
            !output.is_empty()
                && Path::new(output)
                    .components()
                    .all(|part| matches!(part, std::path::Component::Normal(_))),
            "invalid artifact path; writer worktree preserved",
        )?;
        match path.join(output).symlink_metadata() {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        }
        if ok(path, &["--literal-pathspecs", "ls-files", "--", output])?.is_empty() {
            return Err(Error::Invalid(format!(
                "uncommitted artifact {output} (possibly ignored); writer worktree preserved"
            )));
        }
    }
    ok(project, &["worktree", "remove", &path.to_string_lossy()]).map(|_| ())
}

/// The repository a linked worktree belongs to, read from the worktree itself.
pub fn worktree_project(path: &Path) -> Result<PathBuf> {
    let common = PathBuf::from(ok(
        path,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?);
    match (common.file_name(), common.parent()) {
        (Some(name), Some(project)) if name == ".git" => Ok(project.to_path_buf()),
        _ => Err(Error::Invalid(format!(
            "worktree {} belongs to no project checkout",
            path.display()
        ))),
    }
}

/// Forgets worktrees whose directories are gone.
pub fn prune_worktrees(project: &Path) -> Result<()> {
    ok(project, &["worktree", "prune"]).map(|_| ())
}

/// Whether a checkout has no uncommitted change, ignored files aside.
pub fn is_clean(path: &Path) -> bool {
    ok(path, &["status", "--porcelain"]).is_ok_and(|status| status.is_empty())
}

/// Local branches whose names start with `prefix`.
pub fn branches(project: &Path, prefix: &str) -> Result<Vec<String>> {
    let pattern = format!("refs/heads/{prefix}");
    Ok(ok(
        project,
        &["for-each-ref", "--format=%(refname:short)", &pattern],
    )?
    .lines()
    .map(str::to_owned)
    .collect())
}

/// Deletes a branch no worktree has checked out.
pub fn delete_branch(project: &Path, branch: &str) -> Result<()> {
    ok(project, &["branch", "-D", branch]).map(|_| ())
}

/// Renames a branch, so the work on it is kept when its name is needed again.
pub fn rename_branch(project: &Path, from: &str, to: &str) -> Result<()> {
    ok(project, &["branch", "-m", from, to]).map(|_| ())
}

pub fn remove_worktree_keep_branch(project: &Path, path: &Path) -> Result<()> {
    let target = path.to_string_lossy();
    let _ = git(project, &["worktree", "remove", "--force", &target]);
    Ok(())
}

/// Commits everything the worker changed. Returns the new HEAD (unchanged if
/// the worker changed nothing).
pub fn commit_all(worktree: &Path, purpose: CommitPurpose) -> Result<String> {
    super::snapshots::stage_source(worktree, None)?;
    commit_staged(worktree, commit_subject(purpose))
}

pub(crate) fn commit_all_with_message(worktree: &Path, message: &str) -> Result<String> {
    super::snapshots::stage_source(worktree, None)?;
    commit_staged(worktree, message)
}

pub(super) fn commit_staged(worktree: &Path, message: &str) -> Result<String> {
    if !git(worktree, &["diff", "--cached", "--quiet"])?
        .status
        .success()
    {
        ok(
            worktree,
            &["commit", "--quiet", "--no-verify", "-m", message],
        )?;
    }
    head(worktree)
}

/// A commit with the changes `(base, result)` of several writers laid on
/// `onto`, made in a scratch worktree. A change already contained in `onto`
/// (it was integrated and committed) is skipped; one that does not apply is
/// an error saying which.
pub enum CombineResult {
    Applied(String),
    Conflicted(String),
}

pub fn combine(
    project: &Path,
    scratch: &Path,
    onto: &str,
    changes: &[(String, String)],
) -> Result<String> {
    match combine_or_conflict(project, scratch, onto, changes)? {
        CombineResult::Applied(commit) => Ok(commit),
        CombineResult::Conflicted(detail) => Err(Error::Invalid(detail)),
    }
}

/// A content conflict is recoverable input; other Git failures remain errors.
pub fn combine_or_conflict(
    project: &Path,
    scratch: &Path,
    onto: &str,
    changes: &[(String, String)],
) -> Result<CombineResult> {
    combine_or_conflict_with_outputs(project, scratch, onto, changes, &[])
}

pub(crate) fn combine_checkpoint_or_conflict(
    project: &Path,
    scratch: &Path,
    onto: &str,
    base: &str,
    result: &str,
    outputs: &[String],
) -> Result<CombineResult> {
    combine_or_conflict_with_outputs(
        project,
        scratch,
        onto,
        &[(base.to_owned(), result.to_owned())],
        outputs,
    )
}

fn combine_or_conflict_with_outputs(
    project: &Path,
    scratch: &Path,
    onto: &str,
    changes: &[(String, String)],
    outputs: &[String],
) -> Result<CombineResult> {
    let branch = format!(
        "antislop/combine-{}",
        scratch
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
    );
    if scratch.exists() {
        let _ = remove_worktree(project, scratch, &branch);
    }
    add_worktree(project, scratch, &branch, onto)?;
    let file = scratch.with_extension("patch");
    let result = apply_changes(project, scratch, onto, changes, outputs, &file);
    let conflicted = result.is_err()
        && ok(scratch, &["diff", "--name-only", "--diff-filter=U"])
            .is_ok_and(|files| !files.trim().is_empty());
    let result = match result {
        Ok(commit) => Ok(CombineResult::Applied(commit)),
        Err(error) if conflicted => Ok(CombineResult::Conflicted(error.to_string())),
        Err(error) => Err(error),
    };
    let _ = std::fs::remove_file(&file);
    let cleanup = remove_worktree(project, scratch, &branch);
    if result.is_ok() {
        cleanup?;
    }

    result
}

fn apply_changes(
    project: &Path,
    scratch: &Path,
    onto: &str,
    changes: &[(String, String)],
    outputs: &[String],
    file: &Path,
) -> Result<String> {
    for (base, change) in changes {
        apply_change(project, scratch, onto, base, change, outputs, file)?;
    }
    super::snapshots::stage_source(scratch, None)?;
    let mut stage_outputs = vec!["--literal-pathspecs", "add", "--all", "--force", "--"];
    stage_outputs.extend(outputs.iter().map(String::as_str));
    if !outputs.is_empty() {
        ok(scratch, &stage_outputs)?;
    }
    commit_staged(scratch, commit_subject(CommitPurpose::DependencySnapshot))
}

fn apply_change(
    project: &Path,
    scratch: &Path,
    onto: &str,
    base: &str,
    change: &str,
    outputs: &[String],
    file: &Path,
) -> Result<()> {
    if git(scratch, &["merge-base", "--is-ancestor", change, "HEAD"])?
        .status
        .success()
    {
        return Ok(());
    }
    let patch = source_and_artifact_patch(project, base, change, outputs)?;
    if patch.trim().is_empty() {
        return Ok(());
    }
    std::fs::write(file, format!("{patch}\n"))?;
    let path = file.to_string_lossy();
    apply_patch(scratch, onto, base, change, &path)
}

// Explicit artifact inputs are overlaid separately; composing source views
// must not replay runtime files accidentally captured by older workers.
fn source_and_artifact_patch(
    dir: &Path,
    base: &str,
    rev: &str,
    outputs: &[String],
) -> Result<String> {
    let mut patch = super::changes::source_patch(dir, base, rev)?;
    let ignored_outputs = outputs
        .iter()
        .filter(|path| path.starts_with(".antislop/"))
        .collect::<Vec<_>>();
    if ignored_outputs.is_empty() {
        return Ok(patch);
    }
    let mut args = vec![
        "--literal-pathspecs",
        "-c",
        "core.quotePath=false",
        "diff",
        "--no-color",
        "--no-ext-diff",
        "--binary",
        "--full-index",
        "-M",
        base,
        rev,
        "--",
    ];
    args.extend(ignored_outputs.into_iter().map(String::as_str));
    let output = git(dir, &args)?;
    crate::require(
        output.status.success(),
        &format!(
            "git diff: {}",
            crate::lean::agent::clip_tail(&String::from_utf8_lossy(&output.stderr), 8000)
        ),
    )?;
    let artifact_patch = String::from_utf8_lossy(&output.stdout);
    if !patch.is_empty() && !artifact_patch.is_empty() && !patch.ends_with('\n') {
        patch.push('\n');
    }
    patch.push_str(&artifact_patch);
    Ok(patch)
}

fn apply_patch(scratch: &Path, onto: &str, base: &str, change: &str, path: &str) -> Result<()> {
    if git(scratch, &["apply", "--check", "-R", path])?
        .status
        .success()
    {
        return Ok(());
    }
    ok(scratch, &["apply", "--3way", "--index", path]).map_err(|error| {
        Error::Invalid(format!(
            "the change {base}..{change} does not apply to {onto}: {error}"
        ))
    })?;
    Ok(())
}

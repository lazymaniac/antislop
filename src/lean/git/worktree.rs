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
        // A missing worktree can be reattached only at the requested result.
        // Never reset a preserved branch, even when its name matches.
        if ok(project, &["rev-parse", &reference])? != ok(project, &["rev-parse", base])? {
            return Err(Error::Invalid(format!(
                "existing branch {branch} differs from requested worktree base {base}"
            )));
        }
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
/// Normal removal refuses tracked/untracked edits, including changes racing the check.
pub(crate) fn remove_clean_view(project: &Path, path: &Path, branch: &str) -> Result<()> {
    require_worktree_branch(path, branch)?;
    crate::require(
        is_clean(path),
        &format!(
            "worktree {} has uncommitted files; preserved",
            path.display()
        ),
    )?;
    require_no_ignored_files(path)?;
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
pub(crate) fn remove_clean_writer(
    project: &Path,
    path: &Path,
    branch: &str,
    outputs: &[String],
) -> Result<()> {
    require_worktree_branch(path, branch)?;
    require_no_ignored_files(path)?;
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

/// Git cleanliness excludes ignored files, including irreplaceable runtime state.
/// Never infer that an undeclared ignored file is a disposable build cache.
fn require_no_ignored_files(path: &Path) -> Result<()> {
    let ignored = ok(
        path,
        &[
            "ls-files",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--directory",
            "--no-empty-directory",
            "-z",
        ],
    )?;
    let durable = ignored
        .split('\0')
        .find(|entry| !entry.is_empty() && !is_cache_directory(&path.join(entry)));
    crate::require(
        durable.is_none(),
        &format!(
            "ignored files are not preserved by the Git checkpoint; worktree {} preserved ({}). Declare durable outputs before handoff; remove only verified disposable files explicitly",
            path.display(),
            crate::lean::agent::clip(durable.unwrap_or_default(), 300)
        ),
    )
}

/// A directory that marks itself as a regenerable cache with the standard
/// `CACHEDIR.TAG` signature (written by cargo for `target/`, among others).
fn is_cache_directory(directory: &Path) -> bool {
    use std::io::Read;
    const SIGNATURE: &[u8] = b"Signature: 8a477f597d28d172789f06886806bc55";
    let Ok(tag) = std::fs::File::open(directory.join("CACHEDIR.TAG")) else {
        return false;
    };
    let mut head = Vec::with_capacity(SIGNATURE.len());
    tag.take(SIGNATURE.len() as u64)
        .read_to_end(&mut head)
        .is_ok()
        && head == SIGNATURE
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
pub fn commit_all(worktree: &Path, message: &str) -> Result<String> {
    super::snapshots::stage_source(worktree, None)?;
    commit_staged(worktree, message)
}

pub(super) fn commit_staged(worktree: &Path, message: &str) -> Result<String> {
    let subject = commit_subject(message);
    let message = subject.as_str();
    if !git(worktree, &["diff", "--cached", "--quiet"])?
        .status
        .success()
    {
        ok(
            worktree,
            &[
                "-c",
                "user.name=antislop",
                "-c",
                "user.email=antislop@localhost",
                "commit",
                "--quiet",
                "--no-verify",
                "-m",
                message,
            ],
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
    let result = apply_changes(project, scratch, onto, changes, &file);
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
    file: &Path,
) -> Result<String> {
    for (base, change) in changes {
        apply_change(project, scratch, onto, base, change, file)?;
    }
    commit_all(scratch, "Results of the tasks this one depends on")
}

fn apply_change(
    project: &Path,
    scratch: &Path,
    onto: &str,
    base: &str,
    change: &str,
    file: &Path,
) -> Result<()> {
    if git(scratch, &["merge-base", "--is-ancestor", change, "HEAD"])?
        .status
        .success()
    {
        return Ok(());
    }
    let patch = super::changes::source_patch(project, base, change)?;
    if patch.trim().is_empty() {
        return Ok(());
    }
    std::fs::write(file, format!("{patch}\n"))?;
    let path = file.to_string_lossy();
    apply_patch(scratch, onto, base, change, &path)
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

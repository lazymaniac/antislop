//! Worktrees from the project's own repository and integration of a writer's
//! result. Integration never overwrites the user's uncommitted edits and never
//! leaves conflict markers in the project: it is checked in memory first.

use crate::{Error, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> Result<std::process::Output> {
    Ok(Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()?)
}

fn ok(dir: &Path, args: &[&str]) -> Result<String> {
    let output = git(dir, args)?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        Err(Error::Invalid(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

/// The working tree as it is now (tracked and untracked files, ignored ones
/// left out) recorded as a commit on top of HEAD, without touching the user's
/// index, files or branch. A copy of the index keeps git from re-reading
/// files that did not change, so the cost follows the changes, not the repo.
pub fn snapshot(dir: &Path, message: &str) -> Result<String> {
    let index = PathBuf::from(ok(dir, &["rev-parse", "--git-path", "index"])?);
    let index = if index.is_absolute() {
        index
    } else {
        dir.join(index)
    };
    let scratch = index.with_file_name(format!(
        "antislop-snapshot-{}-{}.index",
        std::process::id(),
        crate::id()
    ));
    let result = (|| {
        if index.exists() {
            std::fs::copy(&index, &scratch)?;
        }
        let with_index = |args: &[&str]| -> Result<String> {
            let output = Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .env("GIT_INDEX_FILE", &scratch)
                .env("GIT_TERMINAL_PROMPT", "0")
                .output()?;
            if output.status.success() {
                Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
            } else {
                Err(Error::Invalid(format!(
                    "git {}: {}",
                    args.join(" "),
                    String::from_utf8_lossy(&output.stderr).trim()
                )))
            }
        };
        with_index(&["add", "--all", "--", "."])?;
        let tree = with_index(&["write-tree"])?;
        ok(
            dir,
            &[
                "-c",
                "user.name=antislop",
                "-c",
                "user.email=antislop@localhost",
                "commit-tree",
                &tree,
                "-p",
                "HEAD",
                "-m",
                message,
            ],
        )
    })();
    let _ = std::fs::remove_file(&scratch);
    result
}

/// Commits exactly `files` as they are in the working tree, whatever else
/// the user has staged or changed.
pub fn commit_paths(dir: &Path, files: &[String], message: &str) -> Result<String> {
    let mut add = vec!["--literal-pathspecs", "add", "--all", "--"];
    add.extend(files.iter().map(String::as_str));
    ok(dir, &add)?;
    // Committed already, by a run cut off before it recorded that.
    let mut staged = vec![
        "--literal-pathspecs",
        "diff",
        "--cached",
        "--quiet",
        "HEAD",
        "--",
    ];
    staged.extend(files.iter().map(String::as_str));
    if git(dir, &staged)?.status.success() {
        return head(dir);
    }
    let mut commit = vec![
        "--literal-pathspecs",
        "-c",
        "user.name=antislop",
        "-c",
        "user.email=antislop@localhost",
        "commit",
        "--quiet",
        "--only",
        "-m",
        message,
        "--",
    ];
    commit.extend(files.iter().map(String::as_str));
    ok(dir, &commit)?;
    head(dir)
}

/// Paths with uncommitted changes, untracked ones included.
pub fn uncommitted(dir: &Path) -> Result<Vec<String>> {
    // Not `ok()`: trimming would eat the leading space of " M path".
    let status = git(dir, &["status", "--porcelain=v1", "-z", "--no-renames"])?;
    Ok(String::from_utf8_lossy(&status.stdout)
        .split('\0')
        .filter(|entry| entry.len() > 3)
        .map(|entry| entry[3..].to_owned())
        .collect())
}

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

pub fn head(dir: &Path) -> Result<String> {
    ok(dir, &["rev-parse", "HEAD"])
}

pub fn common_base(dir: &Path, left: &str, right: &str) -> Result<String> {
    ok(dir, &["merge-base", left, right])
}

/// A worktree on a new branch `branch` at `base`; reused if it exists.
pub fn add_worktree(project: &Path, path: &Path, branch: &str, base: &str) -> Result<()> {
    if path.exists() {
        worktree_branch(path)?;
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
    let target = path.to_string_lossy();
    ok(project, &["worktree", "remove", "--force", &target])?;
    let _ = git(project, &["branch", "-D", branch]);
    Ok(())
}

pub fn remove_worktree_keep_branch(project: &Path, path: &Path) -> Result<()> {
    let target = path.to_string_lossy();
    let _ = git(project, &["worktree", "remove", "--force", &target]);
    Ok(())
}

/// Fingerprint of working tree files in `scope` for freshness validation.
pub fn working_tree_fingerprint(project: &Path, scope: &[String]) -> String {
    let mut sorted_scope = scope.to_vec();
    sorted_scope.sort();
    let mut fp = String::new();
    for rel_path in &sorted_scope {
        let full = project.join(rel_path);
        if full.is_file() {
            if let Ok(bytes) = std::fs::read(&full) {
                fp.push_str(rel_path);
                fp.push(':');
                fp.push_str(&crate::hash(&bytes));
                fp.push('\n');
            }
        } else if full.is_dir() {
            if let Ok(status) = Command::new("git")
                .arg("-C")
                .arg(project)
                .args(["status", "--porcelain=v1", "-z", "--", rel_path])
                .output()
            {
                fp.push_str(rel_path);
                fp.push(':');
                fp.push_str(&crate::hash(&status.stdout));
                fp.push('\n');
            }
        } else {
            fp.push_str(rel_path);
            fp.push_str(":missing\n");
        }
    }
    crate::hash(fp.as_bytes())
}

/// Commits everything the worker changed. Returns the new HEAD (unchanged if
/// the worker changed nothing).
pub fn commit_all(worktree: &Path, message: &str) -> Result<String> {
    ok(worktree, &["add", "--all"])?;
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
pub fn combine(
    project: &Path,
    scratch: &Path,
    onto: &str,
    changes: &[(String, String)],
) -> Result<String> {
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
    let result = (|| {
        for (base, change) in changes {
            if git(scratch, &["merge-base", "--is-ancestor", change, "HEAD"])?
                .status
                .success()
            {
                continue;
            }
            let patch = patch(project, base, Some(change))?;
            if patch.trim().is_empty() {
                continue;
            }
            std::fs::write(&file, format!("{patch}\n"))?;
            let path = file.to_string_lossy();
            // Check first: a failed three-way apply can leave a conflicted index.
            if git(scratch, &["apply", "--check", "-R", &path])?
                .status
                .success()
            {
                continue;
            }
            ok(scratch, &["apply", "--3way", "--index", &path]).map_err(|error| {
                Error::Invalid(format!(
                    "the change {base}..{change} does not apply to {onto}: {error}"
                ))
            })?;
        }
        commit_all(scratch, "Results of the tasks this one depends on")
    })();
    let _ = std::fs::remove_file(&file);
    let cleanup = remove_worktree(project, scratch, &branch);
    if result.is_ok() {
        cleanup?;
    }

    result
}

pub fn diff_stat(dir: &Path, base: &str, result: &str) -> Result<String> {
    ok(dir, &["diff", "--stat", base, result])
}

/// The change from `base` to `rev`, or to the working tree when `rev` is
/// `None`, as a unified patch with unquoted UTF-8 paths and renames.
pub fn patch(dir: &Path, base: &str, rev: Option<&str>) -> Result<String> {
    let mut args = vec![
        "-c",
        "core.quotePath=false",
        "diff",
        "--no-color",
        "--no-ext-diff",
        "-M",
        base,
    ];
    args.extend(rev);
    ok(dir, &args)
}

pub fn changed_files(dir: &Path, base: &str, result: &str) -> Result<Vec<String>> {
    changed_paths(dir, base, result, None)
}

/// Paths with identical content, type and mode in two snapshots. Compare
/// snapshots rather than the live tree, so a task's starting protection is
/// stable across its turns and resumes. Literal pathspecs keep filenames
/// containing glob characters from widening the comparison.
pub fn matching_paths(
    dir: &Path,
    left: &str,
    right: &str,
    paths: &[String],
) -> Result<Vec<String>> {
    if paths.is_empty() {
        return Ok(vec![]);
    }
    let changed: std::collections::HashSet<String> = changed_paths(dir, left, right, Some(paths))?
        .into_iter()
        .collect();
    Ok(paths
        .iter()
        .filter(|path| !changed.contains(*path))
        .cloned()
        .collect())
}

fn changed_paths(
    dir: &Path,
    left: &str,
    right: &str,
    paths: Option<&[String]>,
) -> Result<Vec<String>> {
    let mut args = vec![
        "--literal-pathspecs",
        "diff",
        "--no-renames",
        "--name-only",
        "-z",
        left,
        right,
        "--",
    ];
    if let Some(paths) = paths {
        args.extend(paths.iter().map(String::as_str));
    }
    let output = git(dir, &args)?;
    crate::require(
        output.status.success(),
        &format!(
            "git diff: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    )?;
    Ok(std::str::from_utf8(&output.stdout)
        .map_err(|error| Error::Invalid(format!("git paths: {error}")))?
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
        .collect())
}

/// One commit holding everything from `base` to `result`, titled `message`.
pub fn squash(worktree: &Path, base: &str, message: &str) -> Result<String> {
    let tree = ok(worktree, &["rev-parse", "HEAD^{tree}"])?;
    ok(
        worktree,
        &[
            "-c",
            "user.name=antislop",
            "-c",
            "user.email=antislop@localhost",
            "commit-tree",
            &tree,
            "-p",
            base,
            "-m",
            message,
        ],
    )
}

#[derive(Debug, PartialEq)]
pub enum Integration {
    Committed(String),
    /// Applied to the working tree, left for the user to commit.
    Applied,
    Conflict(Vec<String>),
    /// The result conflicts with uncommitted edits.
    UserEdits(Vec<String>),
}

/// Applies the squashed commit `change` (made on `base`) to the project's
/// current branch, optionally restricted to `allowed_files`.
pub fn integrate_scoped(
    project: &Path,
    base: &str,
    change: &str,
    allowed_files: Option<&[String]>,
    commit: bool,
) -> Result<Integration> {
    let files = changed_files(project, base, change)?;
    let target_files: Vec<String> = if let Some(allowed) = allowed_files {
        files
            .iter()
            .filter(|f| allowed.contains(f))
            .cloned()
            .collect()
    } else {
        files.clone()
    };
    if target_files.is_empty() {
        return Ok(Integration::Applied);
    }

    // Landed already, by a run cut off before it recorded that: never twice.
    if unchanged_since(project, change, &target_files)? {
        if !commit {
            return Ok(Integration::Applied);
        }
        if blobs(project, "HEAD", &target_files)? == blobs(project, change, &target_files)? {
            return Ok(Integration::Committed(head(project)?));
        }
    }

    // Uncommitted files that are exactly as the worker started from (for
    // example an earlier task's applied result) are not the user's edits.
    if !commit && unchanged_since(project, base, &target_files)? {
        write_files(project, change, &target_files)?;
        return Ok(Integration::Applied);
    }

    // Merge the worker's delta from its actual starting snapshot, including
    // local edits it inherited, into the current working tree. Dirty paths
    // alone are not conflicts; disjoint edits in the same file can merge.
    let dirty = uncommitted(project)?;
    let onto = if dirty.is_empty() {
        head(project)?
    } else {
        snapshot(project, "Working tree before integration")?
    };
    let committed_base = common_base(project, base, "HEAD")?;
    let merge_base = format!("--merge-base={base}");
    let check = git(
        project,
        &[
            "merge-tree",
            "--write-tree",
            "--name-only",
            &merge_base,
            &onto,
            change,
        ],
    )?;
    if check.status.code() == Some(1) {
        let conflicts: Vec<String> = String::from_utf8_lossy(&check.stdout)
            .lines()
            .skip(1)
            .take_while(|line| !line.is_empty())
            .filter(|path| target_files.contains(&path.to_string()))
            .map(str::to_owned)
            .collect();
        if !conflicts.is_empty() {
            if conflicts.iter().any(|path| dirty.contains(path)) {
                return Ok(Integration::UserEdits(conflicts));
            }
            return Ok(Integration::Conflict(conflicts));
        }
    }
    if !check.status.success() && check.status.code() != Some(1) {
        return Err(Error::Invalid(format!(
            "git merge-tree: {}",
            String::from_utf8_lossy(&check.stderr).trim()
        )));
    }

    if dirty.is_empty()
        && allowed_files.is_none()
        && target_files.len() == files.len()
        && committed_base == base
    {
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
            Ok(Integration::Committed(head(project)?))
        } else {
            ok(project, &["cherry-pick", "--no-commit", change])?;
            // Leave the result as ordinary working-tree changes; unstage only
            // this change's paths, never what the user staged.
            let mut args = vec!["--literal-pathspecs", "reset", "--quiet", "--"];
            args.extend(files.iter().map(String::as_str));
            let _ = git(project, &args);
            Ok(Integration::Applied)
        }
    } else {
        let merged_tree = std::str::from_utf8(&check.stdout)
            .map_err(|error| Error::Invalid(format!("git merge-tree: {error}")))?
            .lines()
            .next()
            .ok_or_else(|| Error::Invalid("git merge-tree returned no tree".into()))?;
        write_files(project, merged_tree, &target_files)?;
        if commit {
            let msg = ok(project, &["log", "-1", "--format=%B", change])?;
            let committed = commit_paths(project, &target_files, msg.trim())?;
            Ok(Integration::Committed(committed))
        } else {
            Ok(Integration::Applied)
        }
    }
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

/// Blob ids of `files` at `rev`; files absent there are absent here.
fn blobs(dir: &Path, rev: &str, files: &[String]) -> Result<HashMap<String, String>> {
    let mut args = vec!["--literal-pathspecs", "ls-tree", "-r", "-z", rev, "--"];
    args.extend(files.iter().map(String::as_str));
    Ok(ok(dir, &args)?
        .split('\0')
        .filter_map(|entry| {
            let (meta, path) = entry.split_once('\t')?;
            Some((path.to_owned(), meta.split(' ').nth(2)?.to_owned()))
        })
        .collect())
}

/// Whether every one of `files` in the working tree is exactly as at `rev`.
fn unchanged_since(project: &Path, rev: &str, files: &[String]) -> Result<bool> {
    let expected = blobs(project, rev, files)?;
    let present: Vec<&str> = files
        .iter()
        .filter(|file| project.join(file).symlink_metadata().is_ok())
        .map(String::as_str)
        .collect();
    if present.len() != expected.len() || present.iter().any(|file| !expected.contains_key(*file)) {
        return Ok(false);
    }
    if present.is_empty() {
        return Ok(true);
    }
    let mut args = vec!["hash-object", "--"];
    args.extend(&present);
    let hashes = ok(project, &args)?;
    Ok(present
        .iter()
        .zip(hashes.lines())
        .all(|(file, hash)| expected[*file] == hash))
}

/// Writes `files` as they are at `rev` into the working tree only.
fn write_files(project: &Path, rev: &str, files: &[String]) -> Result<()> {
    let present = blobs(project, rev, files)?;
    for file in files.iter().filter(|file| !present.contains_key(*file)) {
        match std::fs::remove_file(project.join(file)) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
            _ => {}
        }
    }
    if !present.is_empty() {
        let source = format!("--source={rev}");
        let mut args = vec![
            "--literal-pathspecs",
            "restore",
            &source,
            "--worktree",
            "--",
        ];
        args.extend(present.keys().map(String::as_str));
        ok(project, &args)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        ok(path, &["init", "--quiet", "-b", "main"]).unwrap();
        std::fs::write(path.join("a.txt"), "one\n").unwrap();
        std::fs::write(path.join("b.txt"), "two\n").unwrap();
        commit_all(path, "base").unwrap();
        dir
    }

    fn worker_change(
        project: &Path,
        file: &str,
        text: &str,
    ) -> (tempfile::TempDir, String, String) {
        worker_change_as(project, "t1", file, text)
    }

    fn worker_change_as(
        project: &Path,
        task: &str,
        file: &str,
        text: &str,
    ) -> (tempfile::TempDir, String, String) {
        let trees = tempfile::tempdir().unwrap();
        let worktree = trees.path().join(task);
        let base = head(project).unwrap();
        add_worktree(project, &worktree, &format!("antislop/{task}"), &base).unwrap();
        std::fs::write(worktree.join(file), text).unwrap();
        commit_all(&worktree, "work").unwrap();
        let change = squash(&worktree, &base, "Task t1").unwrap();
        (trees, base, change)
    }

    #[test]
    fn results_of_several_writers_are_laid_on_one_base() {
        let project = repo();
        let (_t1, base1, change1) = worker_change(project.path(), "a.txt", "from first\n");
        let (_t2, base2, change2) =
            worker_change_as(project.path(), "t2", "b.txt", "from second\n");
        let scratch = tempfile::tempdir().unwrap();
        let onto = head(project.path()).unwrap();
        let combined = combine(
            project.path(),
            &scratch.path().join("base-t3"),
            &onto,
            &[(base1.clone(), change1.clone()), (base2, change2)],
        )
        .unwrap();
        let show =
            |file: &str| ok(project.path(), &["show", &format!("{combined}:{file}")]).unwrap();
        assert_eq!(show("a.txt"), "from first");
        assert_eq!(show("b.txt"), "from second");
        // The user's working tree and branch are untouched.
        assert_eq!(head(project.path()).unwrap(), onto);
        assert_eq!(
            std::fs::read_to_string(project.path().join("a.txt")).unwrap(),
            "one\n"
        );
        // A change that is already part of the base is skipped, not an error.
        let again = combine(
            project.path(),
            &scratch.path().join("base-t4"),
            &combined,
            &[(base1, change1)],
        )
        .unwrap();
        assert_eq!(
            ok(project.path(), &["show", &format!("{again}:a.txt")]).unwrap(),
            "from first"
        );
    }

    #[test]
    fn a_change_that_does_not_apply_is_named_in_the_error() {
        let project = repo();
        let (_t1, base1, change1) = worker_change(project.path(), "a.txt", "from first\n");
        // The user changed the same line differently and committed.
        std::fs::write(project.path().join("a.txt"), "user\n").unwrap();
        commit_all(project.path(), "user edit").unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let onto = head(project.path()).unwrap();
        let error = combine(
            project.path(),
            &scratch.path().join("base-t3"),
            &onto,
            &[(base1, change1)],
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("does not apply"), "{error}");
        assert!(!scratch.path().join("base-t3").exists(), "scratch removed");
    }

    #[test]
    fn a_clean_result_becomes_one_commit_and_keeps_other_user_edits() {
        let project = repo();
        let (_trees, base, change) = worker_change(project.path(), "a.txt", "changed\n");
        std::fs::write(project.path().join("b.txt"), "user edit\n").unwrap();
        let result = integrate(project.path(), &base, &change, true).unwrap();
        assert!(matches!(result, Integration::Committed(_)));
        assert_eq!(
            std::fs::read_to_string(project.path().join("a.txt")).unwrap(),
            "changed\n"
        );
        assert_eq!(
            std::fs::read_to_string(project.path().join("b.txt")).unwrap(),
            "user edit\n"
        );
        assert_eq!(
            ok(project.path(), &["log", "-1", "--format=%s"]).unwrap(),
            "Task t1"
        );
    }

    #[test]
    fn a_change_that_already_landed_is_not_landed_again() {
        let project = repo();
        let (_trees, base, change) = worker_change(project.path(), "a.txt", "changed\n");
        let Integration::Committed(first) =
            integrate(project.path(), &base, &change, true).unwrap()
        else {
            panic!("not committed");
        };
        assert_eq!(
            integrate(project.path(), &base, &change, true).unwrap(),
            Integration::Committed(first.clone())
        );
        assert_eq!(head(project.path()).unwrap(), first, "no second commit");

        let project = repo();
        let (_trees, base, change) = worker_change(project.path(), "a.txt", "changed\n");
        for _ in 0..2 {
            assert_eq!(
                integrate(project.path(), &base, &change, false).unwrap(),
                Integration::Applied
            );
        }
        assert_eq!(head(project.path()).unwrap(), base);
        assert_eq!(
            std::fs::read_to_string(project.path().join("a.txt")).unwrap(),
            "changed\n"
        );
    }

    #[test]
    fn inherited_local_edits_are_a_merge_base_in_both_commit_modes() {
        for commit in [false, true] {
            let project = repo();
            std::fs::write(project.path().join("a.txt"), "user draft\n").unwrap();
            std::fs::write(project.path().join("b.txt"), "unrelated draft\n").unwrap();
            ok(project.path(), &["add", "b.txt"]).unwrap();
            let index_before = ok(project.path(), &["ls-files", "--stage", "b.txt"]).unwrap();
            let base = snapshot(project.path(), "starting edits").unwrap();
            let trees = tempfile::tempdir().unwrap();
            let worktree = trees.path().join("worker");
            add_worktree(project.path(), &worktree, "antislop/local", &base).unwrap();
            std::fs::write(worktree.join("a.txt"), "user draft\nworker addition\n").unwrap();
            commit_all(&worktree, "work").unwrap();
            let change = squash(&worktree, &base, "Task local").unwrap();
            let outcome = integrate(project.path(), &base, &change, commit).unwrap();
            assert!(matches!(
                outcome,
                Integration::Applied | Integration::Committed(_)
            ));
            assert_eq!(
                std::fs::read_to_string(project.path().join("a.txt")).unwrap(),
                "user draft\nworker addition\n"
            );
            assert_eq!(
                std::fs::read_to_string(project.path().join("b.txt")).unwrap(),
                "unrelated draft\n"
            );
            assert_eq!(
                ok(project.path(), &["ls-files", "--stage", "b.txt"]).unwrap(),
                index_before
            );
        }
    }

    #[test]
    fn concurrent_disjoint_edits_in_one_dirty_file_merge() {
        let project = repo();
        std::fs::write(project.path().join("a.txt"), "first\n2\n3\n4\nlast\n").unwrap();
        commit_all(project.path(), "multiline").unwrap();
        let (_trees, base, change) =
            worker_change(project.path(), "a.txt", "worker\n2\n3\n4\nlast\n");
        std::fs::write(project.path().join("a.txt"), "first\n2\n3\n4\nuser\n").unwrap();
        assert_eq!(
            integrate(project.path(), &base, &change, false).unwrap(),
            Integration::Applied
        );
        assert_eq!(
            std::fs::read_to_string(project.path().join("a.txt")).unwrap(),
            "worker\n2\n3\n4\nuser\n"
        );
    }

    #[test]
    fn user_edits_and_conflicts_leave_the_project_untouched() {
        let project = repo();
        let (_trees, base, change) = worker_change(project.path(), "a.txt", "worker\n");
        std::fs::write(project.path().join("a.txt"), "user\n").unwrap();
        assert_eq!(
            integrate(project.path(), &base, &change, true).unwrap(),
            Integration::UserEdits(vec!["a.txt".into()])
        );
        commit_all(project.path(), "user commit").unwrap();
        let before = head(project.path()).unwrap();
        assert_eq!(
            integrate(project.path(), &base, &change, true).unwrap(),
            Integration::Conflict(vec!["a.txt".into()])
        );
        assert_eq!(head(project.path()).unwrap(), before);
        assert_eq!(
            std::fs::read_to_string(project.path().join("a.txt")).unwrap(),
            "user\n"
        );
    }

    #[test]
    fn without_commit_permission_the_change_is_left_unstaged() {
        let project = repo();
        let (_trees, base, change) = worker_change(project.path(), "a.txt", "changed\n");
        assert_eq!(
            integrate(project.path(), &base, &change, false).unwrap(),
            Integration::Applied
        );
        assert_eq!(head(project.path()).unwrap(), base);
        assert_eq!(
            ok(project.path(), &["status", "--porcelain"]).unwrap(),
            "M a.txt"
        );
    }

    #[test]
    fn a_dependent_result_applies_over_an_earlier_uncommitted_one() {
        let project = repo();
        let (trees, base, first) = worker_change(project.path(), "a.txt", "first\n");
        assert_eq!(
            integrate(project.path(), &base, &first, false).unwrap(),
            Integration::Applied
        );
        let worktree = trees.path().join("t2");
        add_worktree(project.path(), &worktree, "antislop/t2", &first).unwrap();
        std::fs::write(worktree.join("a.txt"), "second\n").unwrap();
        std::fs::write(worktree.join("c.txt"), "new\n").unwrap();
        std::fs::remove_file(worktree.join("b.txt")).unwrap();
        commit_all(&worktree, "work").unwrap();
        let second = squash(&worktree, &first, "Task t2").unwrap();
        assert_eq!(
            integrate(project.path(), &first, &second, false).unwrap(),
            Integration::Applied
        );
        let read = |file: &str| std::fs::read_to_string(project.path().join(file)).unwrap();
        assert_eq!(
            (read("a.txt"), read("c.txt")),
            ("second\n".into(), "new\n".into())
        );
        assert!(!project.path().join("b.txt").exists());
        assert_eq!(head(project.path()).unwrap(), base);
    }
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        ok(path, &["init", "--quiet", "-b", "main"]).unwrap();
        std::fs::write(path.join("a.txt"), "a\n").unwrap();
        std::fs::write(path.join("b.txt"), "b\n").unwrap();
        std::fs::write(path.join(".gitignore"), "target/\n").unwrap();
        ok(path, &["add", "."]).unwrap();
        ok(
            path,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-qm",
                "base",
            ],
        )
        .unwrap();
        dir
    }

    #[test]
    fn snapshot_path_comparisons_preserve_modes_deletions_and_literal_names() {
        use std::os::unix::fs::PermissionsExt;
        let dir = repo();
        let path = dir.path();
        ok(path, &["config", "core.filemode", "true"]).unwrap();
        std::fs::write(path.join(" file[1].txt"), "literal\n").unwrap();
        std::fs::write(path.join(" file1.txt"), "glob neighbour\n").unwrap();
        let before = snapshot(path, "before").unwrap();
        std::fs::rename(path.join("a.txt"), path.join("renamed.txt")).unwrap();
        std::fs::set_permissions(path.join("b.txt"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        std::fs::write(path.join(" file1.txt"), "changed neighbour\n").unwrap();
        let after = snapshot(path, "after").unwrap();
        let paths = [
            " file[1].txt",
            " file1.txt",
            "a.txt",
            "b.txt",
            "renamed.txt",
            "absent.txt",
        ]
        .map(str::to_owned);
        assert_eq!(
            matching_paths(path, &before, &after, &paths).unwrap(),
            [" file[1].txt", "absent.txt"]
        );
        let mut changed = changed_files(path, &before, &after).unwrap();
        changed.sort();
        assert_eq!(changed, [" file1.txt", "a.txt", "b.txt", "renamed.txt"]);
    }

    #[test]
    fn a_snapshot_records_the_working_tree_and_leaves_it_alone() {
        let dir = repo();
        let path = dir.path();
        std::fs::write(path.join("a.txt"), "edited\n").unwrap();
        std::fs::write(path.join("new.txt"), "new\n").unwrap();
        std::fs::remove_file(path.join("b.txt")).unwrap();
        std::fs::create_dir(path.join("target")).unwrap();
        std::fs::write(path.join("target/out"), "ignored\n").unwrap();
        ok(path, &["add", "a.txt"]).unwrap();
        let status = |path: &Path| {
            String::from_utf8(git(path, &["status", "--porcelain"]).unwrap().stdout).unwrap()
        };
        let before = status(path);

        let snapshot = snapshot(path, "s").unwrap();
        assert_eq!(status(path), before, "index and files untouched");
        assert_eq!(
            ok(path, &["rev-parse", &format!("{snapshot}^")]).unwrap(),
            head(path).unwrap()
        );
        let mut files = changed_files(path, "HEAD", &snapshot).unwrap();
        files.sort();
        assert_eq!(files, ["a.txt", "b.txt", "new.txt"]);
        assert_eq!(
            ok(path, &["show", &format!("{snapshot}:a.txt")]).unwrap(),
            "edited"
        );
        assert!(
            !path.join(".git").read_dir().unwrap().any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("antislop")),
            "no scratch index left behind"
        );
    }

    #[test]
    fn committing_paths_leaves_the_users_other_changes_alone() {
        let dir = repo();
        let path = dir.path();
        std::fs::write(path.join("a.txt"), "task\n").unwrap();
        std::fs::write(path.join("c.txt"), "task new\n").unwrap();
        std::fs::remove_file(path.join(".gitignore")).unwrap();
        std::fs::write(path.join("b.txt"), "user\n").unwrap();
        ok(path, &["add", "b.txt"]).unwrap();

        let commit = commit_paths(
            path,
            &["a.txt".into(), "c.txt".into(), ".gitignore".into()],
            "task",
        )
        .unwrap();
        let mut files = changed_files(path, &format!("{commit}^"), &commit).unwrap();
        files.sort();
        assert_eq!(files, [".gitignore", "a.txt", "c.txt"]);
        let status =
            String::from_utf8(git(path, &["status", "--porcelain"]).unwrap().stdout).unwrap();
        assert_eq!(status, "M  b.txt\n", "the user's staged edit stays staged");
        let again = commit_paths(path, &["a.txt".into(), "c.txt".into()], "task").unwrap();
        assert_eq!(again, commit, "nothing committed twice");
    }
}

/// Stable identity of the integrated files, including uncommitted content.
/// Git's copied index reuses unchanged objects; no repository text enters SQLite.
pub fn integrated_fingerprint(project: &Path) -> Result<String> {
    let snapshot = snapshot(project, "requirements integrated snapshot")?;
    ok(project, &["rev-parse", &format!("{snapshot}^{{tree}}")])
}

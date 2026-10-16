//! Worktrees from the project's own repository and integration of a writer's
//! result. Integration never overwrites the user's uncommitted edits and never
//! leaves conflict markers in the project: it is checked in memory first.

use crate::{Error, Result};
use std::collections::HashMap;
use std::path::Path;
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

pub fn is_repository(dir: &Path) -> bool {
    git(dir, &["rev-parse", "--is-inside-work-tree"]).is_ok_and(|output| output.status.success())
}

pub fn head(dir: &Path) -> Result<String> {
    ok(dir, &["rev-parse", "HEAD"])
}

/// A worktree on a new branch `branch` at `base`; reused if it exists.
pub fn add_worktree(project: &Path, path: &Path, branch: &str, base: &str) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let target = path.to_string_lossy();
    ok(
        project,
        &["worktree", "add", "--quiet", "-B", branch, &target, base],
    )?;
    Ok(())
}

pub fn remove_worktree(project: &Path, path: &Path, branch: &str) -> Result<()> {
    let target = path.to_string_lossy();
    ok(project, &["worktree", "remove", "--force", &target])?;
    let _ = git(project, &["branch", "-D", branch]);
    Ok(())
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

pub fn diff_stat(dir: &Path, base: &str, result: &str) -> Result<String> {
    ok(dir, &["diff", "--stat", base, result])
}

pub fn changed_files(dir: &Path, base: &str, result: &str) -> Result<Vec<String>> {
    Ok(ok(dir, &["diff", "--name-only", "-z", base, result])?
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
    /// The user has uncommitted edits in files the result changes.
    UserEdits(Vec<String>),
}

/// Applies the squashed commit `change` (made on `base`) to the project's
/// current branch.
pub fn integrate(project: &Path, base: &str, change: &str, commit: bool) -> Result<Integration> {
    let files = changed_files(project, base, change)?;
    // Uncommitted files that are exactly as the worker started from (for
    // example an earlier task's applied result) are not the user's edits.
    if !commit && unchanged_since(project, base, &files)? {
        write_files(project, change, &files)?;
        return Ok(Integration::Applied);
    }
    // Not `ok()`: trimming would eat the leading space of " M path".
    let status = git(project, &["status", "--porcelain=v1", "-z"])?;
    let dirty: Vec<String> = String::from_utf8_lossy(&status.stdout)
        .split('\0')
        .filter(|entry| entry.len() > 3)
        .map(|entry| entry[3..].to_owned())
        .filter(|path| files.contains(path))
        .collect();
    if !dirty.is_empty() {
        return Ok(Integration::UserEdits(dirty));
    }
    let merge_base = format!("--merge-base={base}");
    let check = git(
        project,
        &[
            "merge-tree",
            "--write-tree",
            "--name-only",
            &merge_base,
            "HEAD",
            change,
        ],
    )?;
    if check.status.code() == Some(1) {
        let conflicts = String::from_utf8_lossy(&check.stdout)
            .lines()
            .skip(1)
            .take_while(|line| !line.is_empty())
            .map(str::to_owned)
            .collect();
        return Ok(Integration::Conflict(conflicts));
    }
    if !check.status.success() {
        return Err(Error::Invalid(format!(
            "git merge-tree: {}",
            String::from_utf8_lossy(&check.stderr).trim()
        )));
    }
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
        let trees = tempfile::tempdir().unwrap();
        let worktree = trees.path().join("t1");
        let base = head(project).unwrap();
        add_worktree(project, &worktree, "antislop/t1", &base).unwrap();
        std::fs::write(worktree.join(file), text).unwrap();
        commit_all(&worktree, "work").unwrap();
        let change = squash(&worktree, &base, "Task t1").unwrap();
        (trees, base, change)
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

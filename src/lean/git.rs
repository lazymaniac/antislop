//! Worktrees from the project's own repository and integration of a writer's
//! result. Integration never overwrites the user's uncommitted edits and never
//! leaves conflict markers in the project: it is checked in memory first.

#[path = "git/recovery.rs"]
mod recovery;
pub use recovery::recovery_fingerprint;
mod artifact_update;
mod artifacts;
pub(crate) use artifact_update::{deliver_owned_artifacts, require_owned_outputs};
mod changes;
mod integration;
mod snapshots;
mod untracked;
mod worktree;
pub(crate) use artifacts::{commit_artifacts, commit_artifacts_with_message};
pub use changes::*;
pub use integration::*;
pub use snapshots::*;
pub use untracked::deliver_untracked;
pub(crate) use untracked::require_added_files;
pub use worktree::*;

use crate::{Error, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

pub(super) fn git(dir: &Path, args: &[&str]) -> Result<std::process::Output> {
    Ok(Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()?)
}

fn git_with_input(dir: &Path, args: &[&str], input: &[u8]) -> Result<std::process::Output> {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .expect("piped git stdin")
        .write_all(input)?;
    Ok(child.wait_with_output()?)
}

pub(super) fn ok(dir: &Path, args: &[&str]) -> Result<String> {
    let output = git(dir, args)?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        Err(Error::Invalid(format!(
            "git {}: {}",
            args.join(" "),
            crate::lean::agent::clip_tail(String::from_utf8_lossy(&output.stderr).trim(), 8000)
        )))
    }
}

pub(super) fn ok_with_input(dir: &Path, args: &[&str], input: &[u8]) -> Result<String> {
    let output = git_with_input(dir, args, input)?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        Err(Error::Invalid(format!(
            "git {}: {}",
            args.join(" "),
            crate::lean::agent::clip_tail(String::from_utf8_lossy(&output.stderr).trim(), 8000)
        )))
    }
}

/// Application-owned commit purposes with stable runtime subjects. Task result
/// commits use their separately validated worker-generated subject.
#[derive(Clone, Copy, Debug)]
pub enum CommitPurpose {
    TaskResult,
    Checkpoint,
    ArtifactCheckpoint,
    Integration,
    Repair,
    ManualRecovery,
    WorkspaceSnapshot,
    ConflictSnapshot,
    DependencySnapshot,
    ArtifactInputSnapshot,
    SourceInputSnapshot,
    RequirementsSnapshot,
    #[cfg(test)]
    TestFixture,
}

impl CommitPurpose {
    pub(super) fn subject(self) -> &'static str {
        match self {
            Self::TaskResult => "chore: record task result",
            Self::Checkpoint => "chore: save task checkpoint",
            Self::ArtifactCheckpoint => "chore: save artifact checkpoint",
            Self::Integration => "chore: integrate task changes",
            Self::Repair => "chore: repair integration changes",
            Self::ManualRecovery => "chore: apply reviewed recovery fixes",
            Self::WorkspaceSnapshot => "chore: snapshot workspace changes",
            Self::ConflictSnapshot => "chore: snapshot integration conflict state",
            Self::DependencySnapshot => "chore: snapshot dependency changes",
            Self::ArtifactInputSnapshot => "chore: snapshot artifact inputs",
            Self::SourceInputSnapshot => "chore: snapshot source inputs",
            Self::RequirementsSnapshot => "chore: snapshot requirements changes",
            #[cfg(test)]
            Self::TestFixture => "chore: update test fixture",
        }
    }
}

/// Return the fixed English subject for an application-owned commit purpose.
pub(super) fn commit_subject(purpose: CommitPurpose) -> &'static str {
    purpose.subject()
}

/// The working tree as it is now (tracked and untracked files, ignored ones
/// left out) recorded as a commit on top of HEAD, without touching the user's
/// index, files or branch. A copy of the index keeps git from re-reading
/// files that did not change, so the cost follows the changes, not the repo.
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
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_commit_subjects_are_fixed_english() {
        let repo = repo();
        let path = repo.path();
        let base = head(path).unwrap();
        std::fs::write(path.join("a.txt"), "changed").unwrap();
        let commit = commit_paths(path, &["a.txt".into()], CommitPurpose::TaskResult).unwrap();
        assert_eq!(
            ok(path, &["show", "-s", "--format=%B", &commit]).unwrap(),
            "chore: record task result"
        );
        assert_eq!(
            commit_subject(CommitPurpose::TaskResult),
            "chore: record task result"
        );
        let snapshot = snapshot(path, CommitPurpose::TestFixture).unwrap();
        assert_eq!(
            ok(path, &["show", "-s", "--format=%B", &snapshot]).unwrap(),
            "chore: update test fixture"
        );
        let squashed = squash(path, &base, CommitPurpose::TestFixture).unwrap();
        assert_eq!(
            ok(path, &["show", "-s", "--format=%B", &squashed]).unwrap(),
            "chore: update test fixture"
        );
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        ok(path, &["init", "--quiet", "-b", "main"]).unwrap();
        std::fs::write(path.join("a.txt"), "one\n").unwrap();
        std::fs::write(path.join("b.txt"), "two\n").unwrap();
        commit_all(path, CommitPurpose::TestFixture).unwrap();
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
        commit_all(&worktree, CommitPurpose::TestFixture).unwrap();
        let change = squash(&worktree, &base, CommitPurpose::TestFixture).unwrap();
        (trees, base, change)
    }

    #[test]
    fn a_cut_up_writer_keeps_its_work_on_a_renamed_branch_and_starts_again() {
        let project = repo();
        let trees = tempfile::tempdir().unwrap();
        let worktree = trees.path().join("t1");
        let base = head(project.path()).unwrap();
        add_worktree(project.path(), &worktree, "antislop/t1", &base).unwrap();
        std::fs::write(worktree.join("partial.txt"), "half done\n").unwrap();
        let partial = commit_all(&worktree, CommitPurpose::TestFixture).unwrap();

        remove_worktree_keep_branch(project.path(), &worktree).unwrap();
        rename_branch(project.path(), "antislop/t1", "antislop/t1-before-split").unwrap();

        assert!(!worktree.exists());
        let kept = ok(project.path(), &["rev-parse", "antislop/t1-before-split"]).unwrap();
        assert_eq!(kept, partial, "the partial work is still reachable");
        // The name is free for a fresh start from another base.
        std::fs::write(project.path().join("parts.txt"), "from the parts\n").unwrap();
        ok(project.path(), &["add", "-A"]).unwrap();
        ok(project.path(), &["commit", "-q", "-m", "parts"]).unwrap();
        let again = head(project.path()).unwrap();
        add_worktree(project.path(), &worktree, "antislop/t1", &again).unwrap();
        assert!(worktree.join("parts.txt").exists());
        assert!(!worktree.join("partial.txt").exists());
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
        commit_all(project.path(), CommitPurpose::TestFixture).unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let onto = head(project.path()).unwrap();
        let error = combine(
            project.path(),
            &scratch.path().join("base-t3"),
            &onto,
            &[(base1.clone(), change1.clone())],
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("does not apply"), "{error}");
        assert!(!scratch.path().join("base-t3").exists(), "scratch removed");
        let conflict = combine_or_conflict(
            project.path(),
            &scratch.path().join("base-t3"),
            &onto,
            &[(base1, change1)],
        )
        .unwrap();
        assert!(
            matches!(conflict, CombineResult::Conflicted(detail) if detail.contains("does not apply"))
        );
        assert!(
            !scratch.path().join("base-t3").exists(),
            "conflicted scratch removed"
        );
        assert!(
            combine_or_conflict(
                project.path(),
                &scratch.path().join("base-t3"),
                &onto,
                &[(onto.clone(), "missing-commit".into())]
            )
            .is_err()
        );
    }

    #[test]
    fn a_clean_result_becomes_one_commit_and_keeps_other_user_edits() {
        let project = repo();
        let trees = tempfile::tempdir().unwrap();
        let worktree = trees.path().join("worker");
        let base = head(project.path()).unwrap();
        add_worktree(project.path(), &worktree, "antislop/polish-task", &base).unwrap();
        std::fs::write(worktree.join("a.txt"), "changed\n").unwrap();
        ok(&worktree, &["add", "a.txt"]).unwrap();
        ok(
            &worktree,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.test",
                "commit",
                "-m",
                "Popraw błędną obsługę plików",
            ],
        )
        .unwrap();
        let change = head(&worktree).unwrap();
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
            "chore: integrate task changes"
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
            let base = snapshot(project.path(), CommitPurpose::TestFixture).unwrap();
            let trees = tempfile::tempdir().unwrap();
            let worktree = trees.path().join("worker");
            add_worktree(project.path(), &worktree, "antislop/local", &base).unwrap();
            std::fs::write(worktree.join("a.txt"), "user draft\nworker addition\n").unwrap();
            commit_all(&worktree, CommitPurpose::TestFixture).unwrap();
            let change = squash(&worktree, &base, CommitPurpose::TestFixture).unwrap();
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
        commit_all(project.path(), CommitPurpose::TestFixture).unwrap();
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
        commit_all(project.path(), CommitPurpose::TestFixture).unwrap();
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
        commit_all(&worktree, CommitPurpose::TestFixture).unwrap();
        let second = squash(&worktree, &first, CommitPurpose::TestFixture).unwrap();
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
        let before = snapshot(path, CommitPurpose::TestFixture).unwrap();
        std::fs::rename(path.join("a.txt"), path.join("renamed.txt")).unwrap();
        std::fs::set_permissions(path.join("b.txt"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        std::fs::write(path.join(" file1.txt"), "changed neighbour\n").unwrap();
        let after = snapshot(path, CommitPurpose::TestFixture).unwrap();
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
        std::fs::create_dir(path.join(".antislop")).unwrap();
        std::fs::write(path.join(".antislop/build.bin"), b"runtime\0bytes").unwrap();
        ok(path, &["add", "a.txt"]).unwrap();
        ok(path, &["add", "--force", ".antislop/build.bin"]).unwrap();
        let index = std::fs::read(path.join(".git/index")).unwrap();
        let status = |path: &Path| {
            String::from_utf8(git(path, &["status", "--porcelain"]).unwrap().stdout).unwrap()
        };
        let before = status(path);

        let snapshot = snapshot(path, CommitPurpose::TestFixture).unwrap();
        assert_eq!(status(path), before, "index and files untouched");
        assert_eq!(std::fs::read(path.join(".git/index")).unwrap(), index);
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
    fn source_continuation_replays_binary_code_but_not_legacy_runtime_files() {
        let dir = repo();
        let project = dir.path();
        ok(project, &["config", "core.abbrev", "4"]).unwrap();
        let base = head(project).unwrap();
        let views = tempfile::tempdir().unwrap();
        let writer = views.path().join("writer");
        add_worktree(project, &writer, "worker", &base).unwrap();
        std::fs::write(writer.join("image.bin"), b"source\0binary").unwrap();
        std::fs::create_dir(writer.join(".antislop")).unwrap();
        std::fs::write(writer.join(".antislop/build.bin"), b"runtime\0binary").unwrap();
        // Reproduce a checkpoint from a version that captured its own runtime.
        ok(&writer, &["add", "--all"]).unwrap();
        ok(
            &writer,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-qm",
                "legacy",
            ],
        )
        .unwrap();
        let legacy = head(&writer).unwrap();
        let source = combine(
            project,
            &views.path().join("view"),
            &base,
            &[(base.clone(), legacy)],
        )
        .unwrap();
        assert_eq!(
            changed_files(project, &base, &source).unwrap(),
            ["image.bin"]
        );
        assert_eq!(
            git(project, &["show", &format!("{source}:image.bin")])
                .unwrap()
                .stdout,
            b"source\0binary"
        );
        // New worker checkpoints exclude runtime even without a .gitignore.
        let clean = views.path().join("clean");
        add_worktree(project, &clean, "clean", &base).unwrap();
        std::fs::create_dir(clean.join(".antislop")).unwrap();
        std::fs::write(clean.join(".antislop/runtime"), "leave on disk").unwrap();
        std::fs::write(clean.join("new.txt"), "source").unwrap();
        ok(&clean, &["add", "--force", ".antislop/runtime"]).unwrap();
        let saved = commit_all(&clean, CommitPurpose::TestFixture).unwrap();
        assert_eq!(changed_files(project, &base, &saved).unwrap(), ["new.txt"]);
        assert!(clean.join(".antislop/runtime").exists());
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
            CommitPurpose::TestFixture,
        )
        .unwrap();
        let mut files = changed_files(path, &format!("{commit}^"), &commit).unwrap();
        files.sort();
        assert_eq!(files, [".gitignore", "a.txt", "c.txt"]);
        let status =
            String::from_utf8(git(path, &["status", "--porcelain"]).unwrap().stdout).unwrap();
        assert_eq!(status, "M  b.txt\n", "the user's staged edit stays staged");
        let again = commit_paths(
            path,
            &["a.txt".into(), "c.txt".into()],
            CommitPurpose::TestFixture,
        )
        .unwrap();
        assert_eq!(again, commit, "nothing committed twice");
    }
}

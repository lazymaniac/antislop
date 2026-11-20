use super::*;

#[derive(Debug, PartialEq, Eq)]
pub enum SessionMergeOutcome {
    Succeeded { commit: String },
    Failed { details: String },
    MissingInputs { inputs: Vec<String> },
}

/// Merge a preserved session branch into the local `main` ref. A checked-out
/// main is merged with Git's normal worktree safeguards; an unchecked main is
/// updated by compare-and-swap after constructing the merge commit separately.
pub fn merge_session_into_main(
    project: &Path,
    session_branch: &str,
) -> Result<SessionMergeOutcome> {
    if !project.is_dir() {
        return Ok(SessionMergeOutcome::MissingInputs {
            inputs: vec![format!("project directory {}", project.display())],
        });
    }
    if session_branch.trim().is_empty() {
        return Ok(SessionMergeOutcome::MissingInputs {
            inputs: vec!["preserved session branch".into()],
        });
    }
    let branch_check = git(project, &["check-ref-format", "--branch", session_branch])?;
    if !branch_check.status.success() {
        return Ok(SessionMergeOutcome::Failed {
            details: format!(
                "git check-ref-format --branch {session_branch}: {}",
                stderr(&branch_check)
            ),
        });
    }

    let main_ref = resolve_ref(project, "refs/heads/main")?;
    let Some(main) = main_ref else {
        return Ok(SessionMergeOutcome::MissingInputs {
            inputs: vec!["local main branch (refs/heads/main)".into()],
        });
    };
    let source_ref = format!("refs/heads/{session_branch}");
    let source = resolve_ref(project, &source_ref)?;
    let Some(source) = source else {
        return Ok(SessionMergeOutcome::MissingInputs {
            inputs: vec![format!("preserved session branch {session_branch}")],
        });
    };
    if is_ancestor(project, &source, &main)? {
        return Ok(SessionMergeOutcome::Succeeded { commit: main });
    }

    if let Some(main_worktree) = checked_out_worktree(project, "refs/heads/main")? {
        let status = git(
            &main_worktree,
            &["status", "--porcelain", "--untracked-files=all"],
        )?;
        if !status.status.success() {
            return Ok(SessionMergeOutcome::Failed {
                details: format!("git status: {}", stderr(&status)),
            });
        }
        if !status.stdout.is_empty() {
            return Ok(SessionMergeOutcome::Failed {
                details: format!(
                    "Cannot merge session branch into main because its checked-out worktree has user changes; preserve or commit them first. `git status --porcelain`:\n{}",
                    String::from_utf8_lossy(&status.stdout).trim_end()
                ),
            });
        }
        let head = git(&main_worktree, &["rev-parse", "--verify", "HEAD^{commit}"])?;
        let checked_out_head = String::from_utf8_lossy(&head.stdout).trim().to_owned();
        if !head.status.success() || checked_out_head != main {
            return Ok(SessionMergeOutcome::Failed {
                details: format!(
                    "main moved while preparing the merge (refs/heads/main={main}, checked-out HEAD={}); retry the action",
                    if head.status.success() {
                        checked_out_head
                    } else {
                        stderr(&head)
                    }
                ),
            });
        }
        return merge_checked_out_main(&main_worktree, session_branch);
    }

    merge_unchecked_main(project, &main, &source, session_branch)
}

fn resolve_ref(project: &Path, reference: &str) -> Result<Option<String>> {
    let output = git(
        project,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{reference}^{{commit}}"),
        ],
    )?;
    if output.status.success() {
        return Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        ));
    }
    if output.status.code() == Some(1) {
        return Ok(None);
    }
    Err(Error::Invalid(format!(
        "git rev-parse --verify {reference}: {}",
        stderr(&output)
    )))
}

fn is_ancestor(project: &Path, ancestor: &str, descendant: &str) -> Result<bool> {
    let output = git(
        project,
        &["merge-base", "--is-ancestor", ancestor, descendant],
    )?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(Error::Invalid(format!(
            "git merge-base --is-ancestor {ancestor} {descendant}: {}",
            stderr(&output)
        ))),
    }
}

fn checked_out_worktree(project: &Path, branch_ref: &str) -> Result<Option<PathBuf>> {
    let output = git(project, &["worktree", "list", "--porcelain"])?;
    if !output.status.success() {
        return Err(Error::Invalid(format!(
            "git worktree list --porcelain: {}",
            stderr(&output)
        )));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    for entry in text.split("\n\n") {
        let path = entry
            .lines()
            .find_map(|line| line.strip_prefix("worktree "));
        let branch = entry.lines().find_map(|line| line.strip_prefix("branch "));
        if branch == Some(branch_ref) {
            return Ok(path.map(PathBuf::from));
        }
    }
    Ok(None)
}

fn merge_checked_out_main(worktree: &Path, session_branch: &str) -> Result<SessionMergeOutcome> {
    let merge = git(worktree, &["merge", "--no-ff", "--no-edit", session_branch])?;
    if !merge.status.success() {
        let original = git_output(&merge);
        let abort = git(worktree, &["merge", "--abort"])?;
        let details = if abort.status.success() {
            original
        } else {
            format!("{original}\ngit merge --abort: {}", git_output(&abort))
        };
        return Ok(SessionMergeOutcome::Failed { details });
    }
    let head = git(worktree, &["rev-parse", "--verify", "HEAD^{commit}"])?;
    if !head.status.success() {
        return Ok(SessionMergeOutcome::Failed {
            details: format!("git rev-parse --verify HEAD: {}", stderr(&head)),
        });
    }
    Ok(SessionMergeOutcome::Succeeded {
        commit: String::from_utf8_lossy(&head.stdout).trim().to_owned(),
    })
}

fn merge_unchecked_main(
    project: &Path,
    main: &str,
    source: &str,
    session_branch: &str,
) -> Result<SessionMergeOutcome> {
    let tree = git(project, &["merge-tree", "--write-tree", main, source])?;
    if tree.status.code() == Some(1) {
        return Ok(SessionMergeOutcome::Failed {
            details: format!(
                "git merge-tree --write-tree main {session_branch}:\n{}",
                git_output(&tree)
            ),
        });
    }
    if !tree.status.success() {
        return Ok(SessionMergeOutcome::Failed {
            details: format!(
                "git merge-tree --write-tree main {session_branch}: {}",
                git_output(&tree)
            ),
        });
    }
    let tree_id = String::from_utf8_lossy(&tree.stdout)
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_owned();
    if tree_id.is_empty() {
        return Ok(SessionMergeOutcome::Failed {
            details: "git merge-tree --write-tree returned no tree object".into(),
        });
    }
    let commit = git(
        project,
        &[
            "commit-tree",
            &tree_id,
            "-p",
            main,
            "-p",
            source,
            "-m",
            &format!("Merge {session_branch} into main"),
        ],
    )?;
    if !commit.status.success() {
        return Ok(SessionMergeOutcome::Failed {
            details: format!("git commit-tree: {}", stderr(&commit)),
        });
    }
    let commit_id = String::from_utf8_lossy(&commit.stdout).trim().to_owned();
    let update = git(
        project,
        &["update-ref", "refs/heads/main", &commit_id, main],
    )?;
    if !update.status.success() {
        return Ok(SessionMergeOutcome::Failed {
            details: format!(
                "git update-ref refs/heads/main {commit_id} {main}: {}",
                stderr(&update)
            ),
        });
    }
    Ok(SessionMergeOutcome::Succeeded { commit: commit_id })
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).trim().to_owned()
}

fn git_output(output: &std::process::Output) -> String {
    let stdout_text = String::from_utf8_lossy(&output.stdout);
    let stdout = stdout_text.trim();
    let stderr = stderr(output);
    match (stdout.is_empty(), stderr.is_empty()) {
        (false, false) => format!("{stdout}\n{stderr}"),
        (false, true) => stdout.to_owned(),
        (true, false) => stderr,
        (true, true) => format!("git exited with {}", output.status),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path();
        run(path, &["init", "--initial-branch=main"]);
        run(path, &["config", "user.name", "Session Merge Test"]);
        run(
            path,
            &["config", "user.email", "session-merge@example.test"],
        );
        std::fs::write(path.join("base.txt"), "base\n").unwrap();
        run(path, &["add", "base.txt"]);
        run(path, &["commit", "-m", "base"]);
        directory
    }

    fn run(path: &Path, args: &[&str]) -> String {
        ok(path, args).unwrap()
    }

    #[test]
    fn merge_updates_main_ref_when_main_is_not_checked_out() {
        let directory = repo();
        let path = directory.path();
        let base = run(path, &["rev-parse", "HEAD"]);
        run(path, &["branch", "antislop/session/test", &base]);
        run(path, &["checkout", "antislop/session/test"]);
        std::fs::write(path.join("session.txt"), "session result\n").unwrap();
        run(path, &["add", "session.txt"]);
        run(path, &["commit", "-m", "session result"]);
        run(path, &["checkout", "-b", "user-work"]);

        let outcome = merge_session_into_main(path, "antislop/session/test").unwrap();
        let SessionMergeOutcome::Succeeded { commit } = outcome else {
            panic!("expected merge success, got {outcome:?}");
        };
        assert_eq!(run(path, &["rev-parse", "refs/heads/main"]), commit);
        assert_eq!(
            run(path, &["rev-parse", "HEAD"]),
            run(path, &["rev-parse", "refs/heads/user-work"])
        );
        assert_eq!(
            run(path, &["show", "refs/heads/main:session.txt"]),
            "session result"
        );
        assert_eq!(run(path, &["rev-parse", &format!("{commit}^1")]), base);
    }

    #[test]
    fn checked_out_main_merges_session_and_preserves_conflict_state() {
        let directory = repo();
        let path = directory.path();
        let base = run(path, &["rev-parse", "HEAD"]);
        run(path, &["branch", "antislop/session/test", &base]);
        run(path, &["checkout", "antislop/session/test"]);
        std::fs::write(path.join("session.txt"), "session result\n").unwrap();
        run(path, &["add", "session.txt"]);
        run(path, &["commit", "-m", "session result"]);
        run(path, &["checkout", "main"]);
        let outcome = merge_session_into_main(path, "antislop/session/test").unwrap();
        let SessionMergeOutcome::Succeeded { commit } = outcome else {
            panic!("expected merge success, got {outcome:?}");
        };
        assert_eq!(run(path, &["rev-parse", "HEAD"]), commit);
        assert_eq!(run(path, &["show", "HEAD:session.txt"]), "session result");

        let conflict_repo = repo();
        let conflict = conflict_repo.path();
        let base = run(conflict, &["rev-parse", "HEAD"]);
        run(conflict, &["branch", "antislop/session/conflict", &base]);
        run(conflict, &["checkout", "antislop/session/conflict"]);
        std::fs::write(conflict.join("base.txt"), "session edit\n").unwrap();
        run(conflict, &["add", "base.txt"]);
        run(conflict, &["commit", "-m", "session edit"]);
        run(conflict, &["checkout", "main"]);
        std::fs::write(conflict.join("base.txt"), "main edit\n").unwrap();
        run(conflict, &["add", "base.txt"]);
        run(conflict, &["commit", "-m", "main edit"]);
        let main_before = run(conflict, &["rev-parse", "HEAD"]);

        let outcome = merge_session_into_main(conflict, "antislop/session/conflict").unwrap();
        let SessionMergeOutcome::Failed { details } = outcome else {
            panic!("expected conflict failure, got {outcome:?}");
        };
        assert!(
            details.contains("CONFLICT") || details.contains("conflict"),
            "{details}"
        );
        assert_eq!(run(conflict, &["rev-parse", "HEAD"]), main_before);
        assert_eq!(
            std::fs::read_to_string(conflict.join("base.txt")).unwrap(),
            "main edit\n"
        );
        assert!(run(conflict, &["status", "--porcelain"]).is_empty());
    }

    #[test]
    fn missing_main_and_session_refs_are_reported() {
        let directory = tempfile::tempdir().unwrap();
        run(directory.path(), &["init", "--initial-branch=other"]);
        let outcome =
            merge_session_into_main(directory.path(), "antislop/session/missing").unwrap();
        assert_eq!(
            outcome,
            SessionMergeOutcome::MissingInputs {
                inputs: vec!["local main branch (refs/heads/main)".into()],
            }
        );
    }

    #[test]
    fn missing_project_directory_is_reported_as_missing_input() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("missing-project");
        assert_eq!(
            merge_session_into_main(&missing, "antislop/session/missing").unwrap(),
            SessionMergeOutcome::MissingInputs {
                inputs: vec![format!("project directory {}", missing.display())],
            }
        );
    }

    #[test]
    fn dirty_main_worktree_is_left_untouched() {
        let directory = repo();
        let path = directory.path();
        let base = run(path, &["rev-parse", "HEAD"]);
        run(path, &["branch", "antislop/session/dirty", &base]);
        run(path, &["checkout", "antislop/session/dirty"]);
        std::fs::write(path.join("session.txt"), "session result\n").unwrap();
        run(path, &["add", "session.txt"]);
        run(path, &["commit", "-m", "session result"]);
        run(path, &["checkout", "main"]);
        std::fs::write(path.join("base.txt"), "user edit\n").unwrap();
        run(path, &["add", "base.txt"]);
        let main_before = run(path, &["rev-parse", "HEAD"]);

        let outcome = merge_session_into_main(path, "antislop/session/dirty").unwrap();
        let SessionMergeOutcome::Failed { details } = outcome else {
            panic!("expected dirty worktree failure, got {outcome:?}");
        };
        assert!(details.contains("user changes"), "{details}");
        assert_eq!(run(path, &["rev-parse", "HEAD"]), main_before);
        assert_eq!(
            std::fs::read_to_string(path.join("base.txt")).unwrap(),
            "user edit\n"
        );
        assert_eq!(run(path, &["status", "--porcelain"]), "M  base.txt");
    }
}

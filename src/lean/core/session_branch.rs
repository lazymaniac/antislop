use crate::lean::*;
use rusqlite::OptionalExtension;

#[derive(PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionBranch {
    project: String,
    branch: String,
    worktree: PathBuf,
}

impl Core {
    pub(super) fn validate_session_project(project: &Path) -> Result<String> {
        local_main(&session_project(project)?)
    }

    /// Ensure the session-owned line exists before any task is allocated.
    /// Missing legacy records and interrupted creation use the same path as new sessions.
    pub(super) fn create_session_branch(
        &self,
        conversation: &str,
        project: &Path,
    ) -> Result<String> {
        let project = project.canonicalize()?;
        let orchestrator_active = self
            .activity
            .running
            .lock()
            .expect("running mutex")
            .contains_key(conversation);
        let work_is_active = orchestrator_active
            || self.activity.plan_running(conversation)
            || self.activity.review_running(conversation);
        let branch_path = self.session_branch_path(conversation);
        if work_is_active && branch_path.try_exists()? {
            let record = self.load_session_branch(conversation)?;
            require(
                Path::new(&record.project) == project
                    && record.worktree == self.session_worktree_path(conversation),
                "persisted session integration branch belongs to a different project",
            )?;
            return Ok(record.branch);
        }
        let branch_lock = self.locks.turn(conversation, "session-branch");
        let _branches = branch_lock.lock().expect("session branch mutex");
        let legacy_branch = session_branch_name(conversation, &project)?;
        let branch_path = self.session_branch_path(conversation);
        let persisted = if branch_path.try_exists()? {
            Some(self.load_session_branch(conversation)?)
        } else {
            None
        };
        let had_record = persisted.is_some();
        let historical_event_branch: Option<String> = self
            .store
            .db()
            .query_row(
                "SELECT json_extract(payload,'$.branch') FROM events WHERE conversation_id=?1 AND kind='session_started' ORDER BY seq DESC LIMIT 1",
                [conversation],
                |row| row.get(0),
            )
            .optional()?;
        let historical_branch = historical_event_branch.filter(|branch| {
            branch.starts_with("antislop/session/") && branch.ends_with(conversation)
        });
        let has_durable_name = had_record || historical_branch.is_some();
        let worktree = self
            .state
            .join("session-worktrees")
            .join(crate::hash(conversation.as_bytes()));
        let branch = if let Some(record) = &persisted {
            record.branch.clone()
        } else {
            let legacy_ref = format!("refs/heads/{legacy_branch}");
            let legacy_exists =
                git::git(&project, &["show-ref", "--verify", "--quiet", &legacy_ref])?;
            require(
                legacy_exists.status.success() || legacy_exists.status.code() == Some(1),
                &format!(
                    "cannot inspect legacy session branch: {}",
                    String::from_utf8_lossy(&legacy_exists.stderr).trim()
                ),
            )?;
            if let Some(branch) = historical_branch {
                branch
            } else if worktree.exists() {
                require(
                    git::worktree_project(&worktree)?.canonicalize()?
                        == git::worktree_project(&project)?.canonicalize()?,
                    "session integration worktree belongs to a different project",
                )?;
                let actual = git::worktree_branch(&worktree)?;
                require(
                    actual.starts_with("antislop/session/") && actual.ends_with(conversation),
                    "session integration worktree is on an unexpected branch",
                )?;
                actual
            } else if legacy_exists.status.success() {
                legacy_branch
            } else {
                let summary = self.store.workflow(conversation)?.summary;
                let request = if summary.trim().is_empty() {
                    self.store.conversation_goal(conversation)?
                } else {
                    summary
                };
                let title = request
                    .lines()
                    .find(|line| !line.trim().is_empty())
                    .map(str::trim)
                    .filter(|line| !line.is_empty());
                match title {
                    Some(title) => named_session_branch(conversation, title)?,
                    // Older conversations can have tasks but no retained goal
                    // or scope summary. Keep their established naming scheme.
                    None => legacy_branch,
                }
            }
        };
        let record = persisted.unwrap_or(SessionBranch {
            project: project.to_string_lossy().into_owned(),
            branch: branch.clone(),
            worktree: worktree.clone(),
        });
        self.validate_session_record(conversation, &project, &record)?;
        let reference = format!("refs/heads/{}", record.branch);
        let found = git::git(&project, &["show-ref", "--verify", "--quiet", &reference])?;
        require(
            found.status.success() || found.status.code() == Some(1),
            &format!(
                "cannot inspect session integration branch: {}",
                String::from_utf8_lossy(&found.stderr).trim()
            ),
        )?;
        if found.status.success() {
            git::ok(
                &project,
                &["rev-parse", "--verify", &format!("{reference}^{{commit}}")],
            )?;
        } else {
            if worktree.exists() {
                require(
                    git::worktree_project(&worktree)?.canonicalize()?
                        == git::worktree_project(&project)?.canonicalize()?,
                    "session integration worktree belongs to a different project",
                )?;
                require(
                    git::worktree_branch(&worktree)? == record.branch,
                    "session integration worktree is on an unexpected branch",
                )?;
                git::head(&worktree)?;
            }
            require(
                !has_durable_name,
                &format!(
                    "persisted session integration branch {} is missing; restore its Git ref before retrying",
                    record.branch
                ),
            )?;
            let base = local_main(&project)?;
            git::ok(&project, &["branch", &record.branch, &base])?;
        }
        if !had_record {
            self.save_session_branch(conversation, &record)?;
        }
        self.ensure_session_started_event(conversation, &record.branch)?;
        Ok(record.branch)
    }

    /// Return the session target, recreating a missing checkout at its preserved branch tip.
    pub(in crate::lean) fn session_worktree(
        &self,
        conversation: &str,
        project: &Path,
    ) -> Result<PathBuf> {
        self.create_session_branch(conversation, project)?;
        let project = project.canonicalize()?;
        let record = self.load_session_branch(conversation)?;
        self.ensure_session_worktree(conversation, &project, &record)?;
        Ok(record.worktree)
    }

    fn ensure_session_worktree(
        &self,
        conversation: &str,
        project: &Path,
        record: &SessionBranch,
    ) -> Result<()> {
        let branch_lock = self.locks.turn(conversation, "session-branch");
        let _branches = branch_lock.lock().expect("session branch mutex");
        self.validate_session_record(conversation, project, record)?;
        let worktree = &record.worktree;
        if worktree.exists() {
            require(
                git::worktree_project(worktree)?.canonicalize()?
                    == git::worktree_project(project)?.canonicalize()?,
                "session integration worktree belongs to a different project",
            )?;
            require(
                git::worktree_branch(worktree)? == record.branch,
                "session integration worktree is on an unexpected branch",
            )?;
            git::head(worktree)?;
            return Ok(());
        }

        let reference = format!("refs/heads/{}", record.branch);
        let found = git::git(project, &["show-ref", "--verify", "--quiet", &reference])?;
        require(
            found.status.success(),
            &format!(
                "persisted session integration branch {} is missing; restore its Git ref before retrying",
                record.branch
            ),
        )?;
        let base = git::ok(
            project,
            &["rev-parse", "--verify", &format!("{reference}^{{commit}}")],
        )?;
        git::add_worktree(project, worktree, &record.branch, &base)
    }

    fn validate_session_record(
        &self,
        conversation: &str,
        project: &Path,
        record: &SessionBranch,
    ) -> Result<()> {
        git::ok(project, &["check-ref-format", "--branch", &record.branch])?;
        require(
            record.worktree == self.session_worktree_path(conversation),
            "persisted session integration worktree path is invalid",
        )?;
        require(
            Path::new(&record.project) == project,
            "session integration branch belongs to a different project",
        )
    }

    pub(super) fn restore_session_branches(&self) -> Result<()> {
        for conversation in self.store.conversations()? {
            let path = self.session_branch_path(&conversation.id);
            if path.try_exists()? {
                let restored = self
                    .load_session_branch(&conversation.id)
                    .and_then(|record| {
                        self.ensure_session_started_event(&conversation.id, &record.branch)
                    });
                if let Err(error) = restored {
                    self.notice(
                        &conversation.id,
                        &format!("could not restore session branch metadata: {error}"),
                    );
                }
            } else {
                let restored =
                    self.session_worktree(&conversation.id, Path::new(&conversation.project));
                if let Err(error) = restored {
                    self.notice(
                        &conversation.id,
                        &format!("could not recover session integration branch: {error}"),
                    );
                }
            }
        }
        Ok(())
    }

    fn ensure_session_started_event(&self, conversation: &str, branch: &str) -> Result<()> {
        use rusqlite::OptionalExtension;

        let latest: Option<String> = self
            .store
            .db()
            .query_row(
                "SELECT json_extract(payload,'$.branch') FROM events WHERE conversation_id=?1 AND kind='session_started' ORDER BY seq DESC LIMIT 1",
                [conversation],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        if latest.as_deref() != Some(branch) {
            self.store.append_event(
                conversation,
                None,
                None,
                "session_started",
                &json!({"branch": branch}),
            )?;
        }
        Ok(())
    }

    fn session_branch_path(&self, conversation: &str) -> PathBuf {
        self.state
            .join("session-branches")
            .join(crate::hash(conversation.as_bytes()))
    }

    fn session_worktree_path(&self, conversation: &str) -> PathBuf {
        self.state
            .join("session-worktrees")
            .join(crate::hash(conversation.as_bytes()))
    }

    fn load_session_branch(&self, conversation: &str) -> Result<SessionBranch> {
        let path = self.session_branch_path(conversation);
        let text = std::fs::read_to_string(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                Error::Invalid(format!(
                    "session {conversation} has no persisted integration branch"
                ))
            } else {
                error.into()
            }
        })?;
        serde_json::from_str(&text).map_err(|error| {
            Error::Invalid(format!(
                "invalid session branch record {}: {error}",
                path.display()
            ))
        })
    }

    fn save_session_branch(&self, conversation: &str, record: &SessionBranch) -> Result<()> {
        let path = self.session_branch_path(conversation);
        std::fs::create_dir_all(path.parent().expect("session branch parent"))?;
        if path.exists() {
            let existing = self.load_session_branch(conversation)?;
            require(
                existing.branch == record.branch
                    && existing.project == record.project
                    && existing.worktree == record.worktree,
                "session already has a different integration branch",
            )?;
            return Ok(());
        }
        use std::io::Write;
        let mut temporary =
            tempfile::NamedTempFile::new_in(path.parent().expect("session branch parent"))?;
        temporary.write_all(&serde_json::to_vec(record)?)?;
        temporary.as_file().sync_all()?;
        match temporary.persist_noclobber(&path) {
            Ok(_) => {}
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                require(
                    self.load_session_branch(conversation)? == *record,
                    "session already has a different integration branch",
                )?;
            }
            Err(error) => return Err(error.error.into()),
        }
        std::fs::File::open(path.parent().expect("session branch parent"))?.sync_all()?;
        Ok(())
    }
}

fn session_project(project: &Path) -> Result<PathBuf> {
    let project = project.canonicalize()?;
    require(
        git::is_repository(&project),
        "cannot start a session integration branch: project is not a Git repository",
    )?;
    require(
        PathBuf::from(git::ok(&project, &["rev-parse", "--show-toplevel"])?).canonicalize()?
            == project,
        "cannot start a session integration branch: project must be the Git worktree root",
    )?;
    Ok(project)
}

fn session_branch_name(conversation: &str, project: &Path) -> Result<String> {
    let label = project
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("project");
    named_session_branch(conversation, label)
}

fn named_session_branch(conversation: &str, label: &str) -> Result<String> {
    let id = crate::lean::core::worktree::paths::task_path_component(conversation)?;
    let label: String = label
        .chars()
        .take(80)
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_owned();
    let label = if label.is_empty() {
        "project".to_owned()
    } else {
        label
    };
    Ok(format!("antislop/session/{label}-{id}"))
}

fn local_main(project: &Path) -> Result<String> {
    let reference = git::git(
        project,
        &["show-ref", "--verify", "--quiet", "refs/heads/main"],
    )?;
    require(
        reference.status.success() || reference.status.code() == Some(1),
        &format!(
            "cannot inspect local main branch: {}",
            String::from_utf8_lossy(&reference.stderr).trim()
        ),
    )?;
    require(
        reference.status.success(),
        &format!(
            "cannot create a session integration branch for {}: local branch `main` does not exist",
            project.display()
        ),
    )?;
    git::ok(
        project,
        &["rev-parse", "--verify", "refs/heads/main^{commit}"],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repository(path: &Path, branch: &str) {
        std::fs::create_dir_all(path).unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q", "-b", branch])
                .current_dir(path)
                .status()
                .unwrap()
                .success()
        );
        std::fs::write(path.join("source.txt"), "base\n").unwrap();
        git::commit_all(path, git::CommitPurpose::TestFixture).unwrap();
    }

    fn config() -> Config {
        Config::from_toml(include_str!("../../../examples/antislop.toml")).unwrap()
    }

    fn started_session(core: &Core, project: &Path) -> Conversation {
        let conversation = core.start(project).unwrap();
        core.store
            .interview_summary(
                &conversation.id,
                "Session branch recovery and verification",
                0,
            )
            .unwrap();
        conversation
    }

    #[test]
    fn blank_ordinary_message_establishes_session_branch_before_wake() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        repository(&project, "main");
        let core = Core::open(&directory.path().join("state"), config()).unwrap();
        let conversation = started_session(&core, &project);

        // Keep the wake path from spawning an external agent; submit still
        // exercises the same pre-wake branch creation used for a fresh session.
        let _running = core.activity.start_orchestrator(&conversation.id).unwrap();
        core.submit_addressed(&conversation.id, "", None).unwrap();

        let record = core.load_session_branch(&conversation.id).unwrap();
        assert!(
            record
                .branch
                .starts_with("antislop/session/session-branch-recovery-and-verification-")
        );
        assert_eq!(
            git::ok(
                &project,
                &[
                    "rev-parse",
                    "--verify",
                    &format!("refs/heads/{}", record.branch)
                ]
            )
            .unwrap(),
            git::ok(&project, &["rev-parse", "refs/heads/main"]).unwrap()
        );
        assert!(!record.worktree.exists());
        let worktree = core.session_worktree(&conversation.id, &project).unwrap();
        assert_eq!(git::worktree_branch(&worktree).unwrap(), record.branch);
        assert_eq!(
            core.store.messages_after(&conversation.id, 0).unwrap()[0].text,
            ""
        );
    }

    fn git_output(path: &Path, args: &[&str]) -> String {
        git::ok(path, args).unwrap()
    }

    #[test]
    fn startup_git_work_is_bounded_by_repositories_instead_of_conversations() {
        const CHILD: &str = "ANTISLOP_TEST_STARTUP_GIT_STATE";
        if let Some(state) = std::env::var_os(CHILD) {
            let _core = Core::open(Path::new(&state), config()).unwrap();
            return;
        }
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        repository(&project, "main");
        let real_git = crate::lean::doctor::find_executable(Path::new("git")).unwrap();
        let tools = directory.path().join("tools");
        std::fs::create_dir(&tools).unwrap();
        let wrapper = tools.join("git");
        std::fs::write(&wrapper,"#!/bin/sh\nprintf 'git\\n' >> \"$ANTISLOP_TEST_GIT_LOG\"\nexec \"$ANTISLOP_TEST_REAL_GIT\" \"$@\"\n").unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut counts = Vec::new();
        for count in [1, 50] {
            let state = directory.path().join(format!("state-{count}"));
            let core = Core::open(&state, config()).unwrap();
            for _ in 0..count {
                let conversation = core
                    .store
                    .create_conversation(project.to_str().unwrap())
                    .unwrap();
                core.save_session_branch(
                    &conversation.id,
                    &SessionBranch {
                        project: project.to_string_lossy().into_owned(),
                        branch: format!("legacy/{}", conversation.id),
                        worktree: core.session_worktree_path(&conversation.id),
                    },
                )
                .unwrap();
            }
            drop(core);
            let log = directory.path().join(format!("git-{count}.log"));
            let path = std::env::join_paths(std::iter::once(tools.clone()).chain(
                std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
            ))
            .unwrap();
            let output=std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact","lean::core::session_branch::tests::startup_git_work_is_bounded_by_repositories_instead_of_conversations"])
                .env(CHILD,&state).env("PATH",path).env("ANTISLOP_TEST_GIT_LOG",&log).env("ANTISLOP_TEST_REAL_GIT",&real_git).output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            counts.push(
                std::fs::read_to_string(&log)
                    .unwrap_or_default()
                    .lines()
                    .count(),
            );
        }
        eprintln!(
            "startup Git calls for 1 / 50 conversations: {} / {}",
            counts[0], counts[1]
        );
        assert!(counts[0] > 0);
        assert_eq!(counts[0], counts[1]);
    }

    #[test]
    fn legacy_session_without_record_or_ref_recovers_before_allocation() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        repository(&project, "main");
        let main = git::head(&project).unwrap();
        git::ok(&project, &["checkout", "-qb", "user-work"]).unwrap();
        std::fs::write(project.join("source.txt"), "user draft\n").unwrap();
        let state = directory.path().join("state");
        let core = Core::open(&state, config()).unwrap();
        let conversation = core
            .store
            .create_conversation(project.to_str().unwrap())
            .unwrap();
        core.store
            .append_event(
                &conversation.id,
                None,
                None,
                "session_started",
                &json!({"branch":"main"}),
            )
            .unwrap();
        let spec: TaskContract = serde_json::from_value(json!({"id":"session-context","title":"Context","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Read source","change_kind":"none","kind":"context"})).unwrap();
        let mut task = core
            .store
            .add_plan(&conversation.id, &[serde_json::to_value(&spec).unwrap()])
            .unwrap()
            .remove(0);
        task.status = status::FAILED.into();
        task.round = 2;
        task.report = Some("retained checkpoint report".into());
        task.base = Some(main.clone());
        task.result = Some(main.clone());
        core.store.update_task(&task).unwrap();
        assert!(!core.session_branch_path(&conversation.id).exists());
        drop(core);

        let core = Core::open(&state, config()).unwrap();
        let tree = core.session_worktree(&conversation.id, &project).unwrap();
        assert_eq!(git::head(&tree).unwrap(), main);
        assert_eq!(
            core.writer_base(&task.plan_id, &spec, &project).unwrap(),
            main
        );
        let restored = core.store.plan(&task.plan_id).unwrap().remove(0);
        assert_eq!(restored.report, task.report);
        assert_eq!(restored.base, task.base);
        assert_eq!(restored.result, task.result);
        assert_eq!(restored.status, status::FAILED);
        assert_eq!(restored.round, 2);
        assert_eq!(git::session_branch(&project).unwrap(), "user-work");
        assert_eq!(
            std::fs::read_to_string(project.join("source.txt")).unwrap(),
            "user draft\n"
        );
        let branches: Vec<_> = core
            .store
            .events_since(&conversation.id, 0, 100)
            .unwrap()
            .into_iter()
            .filter(|e| e.kind == "session_started")
            .collect();
        assert_eq!(branches.len(), 2);
        assert_eq!(branches[0].payload["branch"], "main");
        assert_eq!(
            branches[1].payload["branch"],
            git::worktree_branch(&tree).unwrap()
        );
        let mut view = crate::lean::tui::view::View::default();
        for event in &branches {
            view.apply(&json!({"seq":event.seq,"kind":event.kind,"payload":event.payload}));
        }
        assert_eq!(
            view.session_branch.as_deref(),
            branches[1].payload["branch"].as_str()
        );
        drop(core);
        let reopened = Core::open(&state, config()).unwrap();
        assert_eq!(
            reopened
                .store
                .events_since(&conversation.id, 0, 100)
                .unwrap()
                .iter()
                .filter(|e| e.kind == "session_started")
                .count(),
            2
        );
    }

    #[test]
    fn interrupted_session_creation_preserves_tip_and_serializes_recovery() {
        for removed in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let project = directory.path().join("project");
            repository(&project, "main");
            let main = git::head(&project).unwrap();
            let state = directory.path().join("state");
            let core = Core::open(&state, config()).unwrap();
            let conversation = core
                .store
                .create_conversation(project.to_str().unwrap())
                .unwrap();
            let branch = session_branch_name(&conversation.id, &project).unwrap();
            let tree = core.session_worktree_path(&conversation.id);
            git::add_worktree(&project, &tree, &branch, &main).unwrap();
            std::fs::write(tree.join("retained.txt"), "retained result\n").unwrap();
            let tip = git::commit_all(&tree, git::CommitPurpose::TestFixture).unwrap();
            if removed {
                git::remove_worktree_keep_branch(&project, &tree).unwrap();
                git::ok(&project, &["branch", "-m", "main", "user-work"]).unwrap();
                // A preserved session ref is sufficient even if main was subsequently removed.
                std::thread::scope(|scope| {
                    for _ in 0..6 {
                        scope.spawn(|| {
                            assert_eq!(
                                core.session_worktree(&conversation.id, &project).unwrap(),
                                tree
                            )
                        });
                    }
                });
            }
            drop(core);
            let core = Core::open(&state, config()).unwrap();
            assert_eq!(
                core.session_worktree(&conversation.id, &project).unwrap(),
                tree
            );
            assert_eq!(git::head(&tree).unwrap(), tip);
            assert_eq!(
                std::fs::read_to_string(tree.join("retained.txt")).unwrap(),
                "retained result\n"
            );
            assert_eq!(git::head(&project).unwrap(), main);
            assert_eq!(
                core.store
                    .events_since(&conversation.id, 0, 100)
                    .unwrap()
                    .iter()
                    .filter(|e| e.kind == "session_started")
                    .count(),
                1
            );
        }
    }

    #[test]
    fn legacy_bootstrap_errors_preserve_foreign_checkouts_and_missing_refs() {
        for obstacle in [
            "missing-main",
            "foreign-project",
            "wrong-branch",
            "lost-owned-ref",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let project = directory.path().join("project");
            repository(
                &project,
                if obstacle == "missing-main" {
                    "trunk"
                } else {
                    "main"
                },
            );
            let state = directory.path().join("state");
            let core = Core::open(&state, config()).unwrap();
            let conversation = core
                .store
                .create_conversation(project.to_str().unwrap())
                .unwrap();
            let tree = core.session_worktree_path(&conversation.id);
            let record = core.session_branch_path(&conversation.id);
            let mut expected_record = None;
            match obstacle {
                "foreign-project" => {
                    let foreign = directory.path().join("foreign");
                    repository(&foreign, "main");
                    git::add_worktree(&foreign, &tree, "foreign-checkout", "main").unwrap();
                }
                "wrong-branch" => {
                    git::add_worktree(&project, &tree, "user-checkout", "main").unwrap();
                }
                "lost-owned-ref" => {
                    let branch = core
                        .create_session_branch(&conversation.id, &project)
                        .unwrap();
                    expected_record = Some(std::fs::read(&record).unwrap());
                    git::remove_worktree_keep_branch(&project, &tree).unwrap();
                    git::ok(&project, &["branch", "-D", &branch]).unwrap();
                }
                _ => {}
            }
            let error = core
                .session_worktree(&conversation.id, &project)
                .unwrap_err()
                .to_string();
            let expected = match obstacle {
                "missing-main" => "local branch `main` does not exist",
                "foreign-project" => "belongs to a different project",
                "wrong-branch" => "unexpected branch",
                _ => "restore its Git ref",
            };
            assert!(error.contains(expected), "{obstacle}: {error}");
            assert_eq!(record.exists(), expected_record.is_some());
            if let Some(bytes) = expected_record {
                assert_eq!(std::fs::read(&record).unwrap(), bytes);
            }
            if obstacle == "missing-main" {
                drop(core);
                let core = Core::open(&state, config()).unwrap();
                assert!(!record.exists());
                let error = core
                    .session_worktree(&conversation.id, &project)
                    .unwrap_err()
                    .to_string();
                assert!(error.contains(expected), "{error}");
            } else if tree.exists() {
                assert_eq!(
                    git::worktree_branch(&tree).unwrap(),
                    if obstacle == "foreign-project" {
                        "foreign-checkout"
                    } else {
                        "user-checkout"
                    }
                );
            }
        }
    }

    #[test]
    fn start_creates_descriptive_branch_from_local_main_and_reopens_it() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("field-notes");
        repository(&project, "main");
        let main = git::head(&project).unwrap();
        git::ok(&project, &["checkout", "-qb", "user-work"]).unwrap();
        let state = directory.path().join("state");
        let core = Core::open(&state, config()).unwrap();
        let conversation = started_session(&core, &project);
        let tree = core.session_worktree(&conversation.id, &project).unwrap();
        let branch = core.load_session_branch(&conversation.id).unwrap().branch;
        assert!(branch.starts_with("antislop/session/session-branch-recovery-and-verification-"));
        assert_eq!(git::head(&tree).unwrap(), main);
        assert_eq!(git::session_branch(&project).unwrap(), "user-work");
        git::remove_worktree_keep_branch(&project, &tree).unwrap();
        drop(core);

        let reopened = Core::open(&state, config()).unwrap();
        let restored = reopened
            .session_worktree(&conversation.id, &project)
            .unwrap();
        assert_eq!(git::worktree_branch(&restored).unwrap(), branch);
        assert_eq!(
            reopened
                .load_session_branch(&conversation.id)
                .unwrap()
                .branch,
            branch
        );
        assert_eq!(
            reopened
                .store
                .events_since(&conversation.id, 0, 100)
                .unwrap()
                .iter()
                .filter(|event| event.kind == "session_started")
                .count(),
            1
        );
        let event = reopened
            .store
            .history_page(&conversation.id, 0, 20)
            .unwrap();
        assert!(
            event.events.iter().any(|event| {
                event.kind == "session_started" && event.payload["branch"] == branch
            })
        );
    }

    #[test]
    fn missing_local_main_is_a_concrete_start_blocker() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("without-main");
        repository(&project, "trunk");
        let core = Core::open(&directory.path().join("state"), config()).unwrap();
        let error = core.start(&project).unwrap_err().to_string();
        assert!(
            error.contains("local branch `main` does not exist"),
            "{error}"
        );
        assert!(core.store.conversations().unwrap().is_empty());
        assert!(
            git::branches(&project, "antislop/session/")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn startup_retries_a_failed_session_branch_creation_from_local_main() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        repository(&project, "main");
        let main = git::head(&project).unwrap();
        let state = directory.path().join("state");
        let core = Core::open(&state, config()).unwrap();
        // Reproduce `start`'s durable conversation write followed by a branch
        // creation failure: local main disappears before create_session_branch.
        let conversation = core
            .store
            .create_conversation(&project.to_string_lossy())
            .unwrap();
        core.store
            .interview_summary(&conversation.id, "Recover a persisted session branch", 0)
            .unwrap();
        git::ok(&project, &["checkout", "-qb", "temporary-user-work"]).unwrap();
        git::ok(&project, &["branch", "-D", "main"]).unwrap();
        let error = core
            .create_session_branch(&conversation.id, &project)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("local branch `main` does not exist"),
            "{error}"
        );
        assert!(
            core.store
                .conversations()
                .unwrap()
                .iter()
                .any(|item| item.id == conversation.id)
        );
        drop(core);

        git::ok(&project, &["branch", "main", &main]).unwrap();
        let reopened = Core::open(&state, config()).unwrap();
        let record = reopened.load_session_branch(&conversation.id).unwrap();
        let worktree = reopened.session_worktree_path(&conversation.id);
        assert!(
            worktree.exists(),
            "startup must recreate the session checkout"
        );
        assert_eq!(git::head(&worktree).unwrap(), main);
        assert_eq!(git::worktree_branch(&worktree).unwrap(), record.branch);
        assert!(
            record
                .branch
                .starts_with("antislop/session/recover-a-persisted-session-branch-")
        );
        let events = reopened
            .store
            .events_since(&conversation.id, 0, 100)
            .unwrap();
        assert!(events.iter().any(|event| {
            event.kind == "session_started" && event.payload["branch"] == record.branch
        }));
        assert_eq!(
            events
                .iter()
                .filter(|event| event.kind == "session_started")
                .count(),
            1
        );
    }

    #[test]
    fn recorded_legacy_branch_name_is_preserved_after_scope_title_changes() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        repository(&project, "main");
        let core = Core::open(&directory.path().join("state"), config()).unwrap();
        let conversation = started_session(&core, &project);
        let branch = format!("antislop/session/project-{}", conversation.id);
        let main = git::head(&project).unwrap();
        git::add_worktree(
            &project,
            &core.session_worktree_path(&conversation.id),
            &branch,
            &main,
        )
        .unwrap();
        core.save_session_branch(
            &conversation.id,
            &SessionBranch {
                project: project
                    .canonicalize()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                branch: branch.clone(),
                worktree: core.session_worktree_path(&conversation.id),
            },
        )
        .unwrap();

        assert_eq!(
            core.create_session_branch(&conversation.id, &project)
                .unwrap(),
            branch
        );
        assert_eq!(
            git::worktree_branch(&core.session_worktree_path(&conversation.id)).unwrap(),
            branch
        );
    }

    #[test]
    fn legacy_event_binding_recovers_without_rewriting_its_branch_name() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        repository(&project, "main");
        let state = directory.path().join("state");
        let core = Core::open(&state, config()).unwrap();
        let conversation = started_session(&core, &project);
        let branch = format!("antislop/session/project-{}", conversation.id);
        git::ok(&project, &["branch", &branch, "main"]).unwrap();
        core.store
            .append_event(
                &conversation.id,
                None,
                None,
                "session_started",
                &json!({"branch": branch}),
            )
            .unwrap();
        drop(core);

        let reopened = Core::open(&state, config()).unwrap();
        let session = reopened
            .session_worktree(&conversation.id, &project)
            .unwrap();
        assert_eq!(git::worktree_branch(&session).unwrap(), branch);
        let events = reopened
            .store
            .events_since(&conversation.id, 0, 100)
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| event.kind == "session_started")
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .find(|event| event.kind == "session_started")
                .unwrap()
                .payload["branch"],
            branch
        );
    }

    #[test]
    fn legacy_repository_branch_event_is_kept_while_a_session_branch_is_added() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        repository(&project, "main");
        let core = Core::open(&directory.path().join("state"), config()).unwrap();
        let conversation = core
            .store
            .create_conversation(project.to_str().unwrap())
            .unwrap();
        core.store
            .interview_summary(
                &conversation.id,
                "Restore the session branch after restart",
                0,
            )
            .unwrap();
        core.store
            .append_event(
                &conversation.id,
                None,
                None,
                "session_started",
                &json!({"branch": "main"}),
            )
            .unwrap();

        let session = core.session_worktree(&conversation.id, &project).unwrap();
        let branch = git::worktree_branch(&session).unwrap();
        assert!(branch.starts_with("antislop/session/restore-the-session-branch-after-restart-"));
        assert_eq!(git::session_branch(&project).unwrap(), "main");
        let events = core
            .store
            .events_since(&conversation.id, 0, 100)
            .unwrap()
            .into_iter()
            .filter(|event| event.kind == "session_started")
            .collect::<Vec<_>>();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].payload["branch"], "main");
        assert_eq!(events[1].payload["branch"], branch);
    }

    #[test]
    fn an_unrelated_checkout_at_the_recorded_path_is_rejected_untouched() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        let foreign = directory.path().join("foreign");
        repository(&project, "main");
        repository(&foreign, "main");
        let foreign_head = git::head(&foreign).unwrap();
        let core = Core::open(&directory.path().join("state"), config()).unwrap();
        let conversation = started_session(&core, &project);
        let worktree = core.session_worktree_path(&conversation.id);
        std::fs::create_dir_all(worktree.parent().unwrap()).unwrap();
        std::fs::rename(&foreign, &worktree).unwrap();
        let branch = format!(
            "antislop/session/session-branch-recovery-and-verification-{}",
            conversation.id
        );
        core.save_session_branch(
            &conversation.id,
            &SessionBranch {
                project: project
                    .canonicalize()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                branch,
                worktree: worktree.clone(),
            },
        )
        .unwrap();

        let error = core
            .session_worktree(&conversation.id, &project)
            .unwrap_err()
            .to_string();
        assert!(error.contains("different project"), "{error}");
        assert_eq!(git::head(&worktree).unwrap(), foreign_head);
    }

    #[test]
    fn a_nested_directory_cannot_inherit_a_parent_checkout_as_its_project() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        repository(&project, "main");
        let nested = project.join("subproject");
        std::fs::create_dir(&nested).unwrap();
        let core = Core::open(&directory.path().join("state"), config()).unwrap();
        let error = core.start(&nested).unwrap_err().to_string();
        assert!(
            error.contains("project must be the Git worktree root"),
            "{error}"
        );
        assert!(core.store.conversations().unwrap().is_empty());
    }

    #[test]
    fn session_creation_works_from_a_linked_project_worktree() {
        let directory = tempfile::tempdir().unwrap();
        let repo = directory.path().join("repository");
        repository(&repo, "main");
        let main = git::head(&repo).unwrap();
        let project = directory.path().join("feature-checkout");
        git::ok(
            &repo,
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                "user-work",
                &project.to_string_lossy(),
                "main",
            ],
        )
        .unwrap();
        let core = Core::open(&directory.path().join("state"), config()).unwrap();
        let conversation = started_session(&core, &project);
        let session = core.session_worktree(&conversation.id, &project).unwrap();
        assert_eq!(git::head(&session).unwrap(), main);
        assert!(
            git::worktree_branch(&session)
                .unwrap()
                .starts_with("antislop/session/session-branch-recovery-and-verification-")
        );
        assert_eq!(git::session_branch(&project).unwrap(), "user-work");
    }

    #[test]
    fn session_branch_accumulates_integrations_and_rejects_conflicts_without_applying_them() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        repository(&project, "main");
        let core = Core::open(&directory.path().join("state"), config()).unwrap();
        let conversation = started_session(&core, &project);
        let session = core.session_worktree(&conversation.id, &project).unwrap();
        let base = git::head(&session).unwrap();

        let first_path = directory.path().join("first");
        git::add_worktree(&project, &first_path, "antislop/test-first", &base).unwrap();
        std::fs::write(first_path.join("first.txt"), "first\n").unwrap();
        std::fs::write(first_path.join("source.txt"), "first edit\n").unwrap();
        let first = git::commit_all(&first_path, git::CommitPurpose::TestFixture).unwrap();
        assert!(matches!(
            git::integrate(&session, &base, &first, true).unwrap(),
            git::Integration::Committed(_)
        ));
        let cumulative = git::head(&session).unwrap();
        assert!(session.join("first.txt").exists());

        let second_path = directory.path().join("second");
        git::add_worktree(&project, &second_path, "antislop/test-second", &cumulative).unwrap();
        std::fs::write(second_path.join("second.txt"), "second\n").unwrap();
        let second = git::commit_all(&second_path, git::CommitPurpose::TestFixture).unwrap();
        assert!(matches!(
            git::integrate(&session, &cumulative, &second, true).unwrap(),
            git::Integration::Committed(_)
        ));
        assert!(session.join("first.txt").exists());
        assert!(session.join("second.txt").exists());

        let conflicting_path = directory.path().join("conflicting");
        git::add_worktree(
            &project,
            &conflicting_path,
            "antislop/test-conflicting",
            &base,
        )
        .unwrap();
        std::fs::write(conflicting_path.join("source.txt"), "competing edit\n").unwrap();
        let conflicting =
            git::commit_all(&conflicting_path, git::CommitPurpose::TestFixture).unwrap();
        let before = git::head(&session).unwrap();
        assert!(matches!(
            git::integrate(&session, &base, &conflicting, true).unwrap(),
            git::Integration::Conflict(_)
        ));
        assert_eq!(git::head(&session).unwrap(), before);
        assert_eq!(
            std::fs::read_to_string(session.join("source.txt")).unwrap(),
            "first edit\n"
        );
        assert_eq!(
            git_output(&project, &["symbolic-ref", "--short", "HEAD"]),
            "main"
        );
    }

    #[test]
    fn approved_task_integration_lands_on_session_branch_and_leaves_checkout_untouched() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        repository(&project, "main");
        let config = config();
        let core = Core::open(&directory.path().join("state"), config.clone()).unwrap();
        let conversation = started_session(&core, &project);
        let request = "Create a result file";
        let _message = core
            .store
            .add_message(&conversation.id, "user", request)
            .unwrap();
        let run = core.store.create_scope(&conversation.id, request).unwrap();
        let spec: TaskContract = serde_json::from_value(json!({
            "id":"writer-one", "title":"Add delivered file", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Create result.txt",
            "change_kind":"code", "kind":"implementation",
             "reviewer":{"summary":"result"}
        }))
        .unwrap();
        let mut task = core
            .store
            .add_plan(&conversation.id, &[serde_json::to_value(&spec).unwrap()])
            .unwrap()
            .remove(0);
        core.store
            .bind_plan_scope(&conversation.id, &task.plan_id, &run.id)
            .unwrap();
        task.status = status::WORKING.into();
        let base = core.writer_base(&task.plan_id, &spec, &project).unwrap();
        let worktree =
            crate::lean::core::worktree::paths::worktree_path(&core.state, &task).unwrap();
        let branch =
            crate::lean::core::worktree::paths::worktree_branch(&core.state, &task).unwrap();
        git::add_worktree(&project, &worktree, &branch, &base).unwrap();
        std::fs::write(worktree.join("result.txt"), "approved result\n").unwrap();
        let result = git::commit_all(&worktree, git::CommitPurpose::TestFixture).unwrap();
        task.base = Some(base);
        task.result = Some(result);
        core.store.update_task(&task).unwrap();

        core.integrate(
            &conversation.id,
            &mut task,
            &spec,
            &project,
            &worktree,
            &config,
            &AtomicBool::new(false),
        )
        .unwrap();

        let session = core.session_worktree(&conversation.id, &project).unwrap();
        assert_eq!(task.status, status::APPROVED);
        assert_eq!(
            std::fs::read_to_string(session.join("result.txt")).unwrap(),
            "approved result\n"
        );
        assert!(!project.join("result.txt").exists());
        assert_eq!(
            git::head(&project).unwrap(),
            git::ok(&project, &["rev-parse", "main"]).unwrap()
        );
    }
}

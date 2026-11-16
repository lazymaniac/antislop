use crate::lean::core::task::feedback_helpers::land;
use crate::lean::*;

pub(super) struct Integration<'a> {
    core: &'a Arc<Core>,
    conversation: &'a str,
    task: &'a mut Task,
    spec: &'a TaskContract,
    project: &'a Path,
    worktree: &'a Path,
    config: &'a Config,
    cancel: &'a AtomicBool,
}

impl<'a> Integration<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        core: &'a Arc<Core>,
        conversation: &'a str,
        task: &'a mut Task,
        spec: &'a TaskContract,
        project: &'a Path,
        worktree: &'a Path,
        config: &'a Config,
        cancel: &'a AtomicBool,
    ) -> Self {
        Self {
            core,
            conversation,
            task,
            spec,
            project,
            worktree,
            config,
            cancel,
        }
    }

    pub(super) fn run(mut self, lock: std::sync::MutexGuard<'_, ()>) -> Result<()> {
        // The keyed conversation source scope protects Git; no global lock is held.
        let base = self.task.base.clone().unwrap_or_default();
        let result = git::head(self.worktree)?;
        if result == base {
            return self.finish_no_changes();
        }
        let change = self.create_change(&base)?;
        let outcome = self.apply_change(&base, &change)?;
        if matches!(
            outcome,
            git::Integration::Committed(_) | git::Integration::Applied
        ) {
            self.core
                .record_plan_landing(&self.task.plan_id, &self.task.id, &change)?;
        }
        if self.repair_conflict(lock, &change, &outcome)? {
            return Ok(());
        }
        self.finish_outcome(&base, &change, outcome)
    }

    fn finish_no_changes(self) -> Result<()> {
        self.core.store.assert_task_inputs(self.task)?;
        crate::lean::task_failure::append_integration_report_entry(
            self.task,
            "integration",
            "no file changes",
        );
        land(
            self.task,
            Outcome::NoChanges,
            "run",
            "no file changes",
            None,
            None,
            &[],
        );
        self.core
            .set_status(self.conversation, self.task, status::APPROVED)
    }

    fn create_change(&mut self, base: &str) -> Result<String> {
        let change = git::squash(self.worktree, base, git::CommitPurpose::TaskResult)?;
        self.task.result = Some(change.clone());
        Ok(change)
    }

    fn apply_change(&self, base: &str, change: &str) -> Result<git::Integration> {
        self.core.store.assert_task_inputs(self.task)?;
        self.spec
            .validate_change_paths(&git::changed_files(self.worktree, base, change)?)?;
        if self.spec.delivery == Some(tasks::DeliveryMode::Untracked) {
            return self.core.deliver_artifact_change(
                self.task,
                self.spec,
                self.project,
                base,
                change,
                self.cancel,
            );
        }
        let session_tree = self
            .core
            .session_worktree(self.conversation, self.project)?;
        git::integrate(
            &session_tree,
            base,
            change,
            self.config.policy.local_commits,
        )
    }

    fn repair_conflict(
        &mut self,
        lock: std::sync::MutexGuard<'_, ()>,
        change: &str,
        outcome: &git::Integration,
    ) -> Result<bool> {
        let Some((kind, files, onto)) = self.register_conflict_repair(change, outcome)? else {
            return Ok(false);
        };
        drop(lock);
        self.core.repair_integration(
            self.conversation,
            self.task,
            self.spec,
            self.worktree,
            &onto,
            change,
            kind,
            &files,
            self.cancel,
        )?;
        Ok(true)
    }

    fn register_conflict_repair(
        &self,
        change: &str,
        outcome: &git::Integration,
    ) -> Result<Option<(Outcome, Vec<String>, String)>> {
        if self.spec.delivery == Some(tasks::DeliveryMode::Untracked) {
            return Ok(None);
        }
        let Some((kind, files)) = conflict_with_files(outcome) else {
            return Ok(None);
        };
        let session_tree = self
            .core
            .session_worktree(self.conversation, self.project)?;
        let onto = git::snapshot(&session_tree, git::CommitPurpose::ConflictSnapshot)?;
        let basis = crate::hash(serde_json::to_vec(&(
            git::tree_identity(&session_tree, self.task.base.as_deref().unwrap_or(change))?,
            git::tree_identity(&session_tree, change)?,
            git::tree_identity(&session_tree, &onto)?,
        ))?);
        let fresh = self.core.store.db().execute(
            "INSERT OR IGNORE INTO integration_attempts(task_id,generation,basis) VALUES(?1,?2,?3)",
            rusqlite::params![self.task.id, self.task.input_generation, basis],
        )?;
        if fresh == 0 {
            return Ok(None);
        }
        Ok(Some((kind, files.to_vec(), onto)))
    }

    fn finish_outcome(self, base: &str, change: &str, outcome: git::Integration) -> Result<()> {
        let target = if self.spec.delivery == Some(tasks::DeliveryMode::Untracked) {
            self.project.to_path_buf()
        } else {
            self.core
                .session_worktree(self.conversation, self.project)?
        };
        let landed = git::changed_files(&target, base, change)?;
        let landing = super::landing::record(
            self.core,
            self.conversation,
            self.task,
            self.spec,
            &outcome,
            &landed,
        )?;
        if landing.status == status::APPROVED {
            self.record_result_documents(&target, change, &landed)?;
        }
        self.complete_landing(landing)
    }

    fn record_result_documents(
        &self,
        target: &Path,
        source_commit: &str,
        landed: &[String],
    ) -> Result<()> {
        let landed = landed.iter().collect::<std::collections::HashSet<_>>();
        for path in &self.spec.outputs {
            if !landed.contains(path) {
                continue;
            }
            let file = git::artifact_file(target, source_commit, path)?;
            let Some(prepared) = self.core.store.prepare_result_document(file)? else {
                continue;
            };
            self.core.store.publish_result_document(
                self.conversation,
                &self.task.id,
                path,
                source_commit,
                prepared,
            )?;
        }
        Ok(())
    }

    fn complete_landing(self, landing: super::landing::Landing) -> Result<()> {
        crate::lean::task_failure::append_integration_report_entry(
            self.task,
            "integration",
            &landing.text,
        );
        if landing.status == status::APPROVED {
            return self
                .core
                .set_status(self.conversation, self.task, landing.status);
        }
        self.core.stop(
            self.conversation,
            self.task,
            landing.status,
            Some(&landing.text),
        )
    }
}

fn conflict_with_files(outcome: &git::Integration) -> Option<(Outcome, &[String])> {
    match outcome {
        git::Integration::Conflict(files) => Some((Outcome::Conflict, files)),
        git::Integration::UserEdits(files) => Some((Outcome::UserEdits, files)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successful_output_selection_uses_only_landed_declared_text_files() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        let state = directory.path().join("state");
        std::fs::create_dir_all(&project).unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q"])
                .current_dir(&project)
                .status()
                .unwrap()
                .success()
        );
        std::fs::write(project.join("source.txt"), "base").unwrap();
        let base = git::commit_all(&project, git::CommitPurpose::TestFixture).unwrap();
        let config =
            Config::from_toml(include_str!("../../../../../examples/antislop.toml")).unwrap();
        let core = Core::open(&state, config.clone()).unwrap();
        let conversation = core
            .store
            .create_conversation(project.to_str().unwrap())
            .unwrap();
        core.create_session_branch(&conversation.id, &project)
            .unwrap();
        let mut task = core
            .store
            .add_plan(
                &conversation.id,
                &[json!({
                    "id":"writer",
                    "title":"Write outputs",
                    "goal":"Create the declared text file",
                    "kind":"writing",
                    "change_kind":"artifact",
                    "outputs":["brief.md", "image.md"],
                    "reviewer":{"summary":"Check outputs"}
                })],
            )
            .unwrap()
            .remove(0);
        let spec: TaskContract = serde_json::from_value(task.spec.clone()).unwrap();
        let worktree = directory.path().join("writer");
        git::add_worktree(&project, &worktree, "test-writer", &base).unwrap();
        std::fs::write(worktree.join("brief.md"), "Deliverable café").unwrap();
        std::fs::write(worktree.join("image.md"), b"\0\xff\x01").unwrap();
        std::fs::write(worktree.join("agent-report.txt"), "Internal worker report").unwrap();
        let change = git::commit_all(&worktree, git::CommitPurpose::TestFixture).unwrap();
        let session_tree = core.session_worktree(&conversation.id, &project).unwrap();
        assert!(matches!(
            git::integrate(
                &session_tree,
                &git::head(&session_tree).unwrap(),
                &change,
                false
            )
            .unwrap(),
            git::Integration::Applied | git::Integration::Committed(_)
        ));
        task.base = Some(base);
        task.result = Some(change.clone());
        let cancel = AtomicBool::new(false);
        Integration::new(
            &core,
            &conversation.id,
            &mut task,
            &spec,
            &project,
            &worktree,
            &config,
            &cancel,
        )
        .finish_outcome(
            &git::head(&project).unwrap(),
            &change,
            git::Integration::Applied,
        )
        .unwrap();

        let documents = core.store.result_documents(&conversation.id).unwrap();
        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].path, "brief.md");
        assert_eq!(documents[0].source_commit, task.result.clone().unwrap());
        assert_eq!(
            core.store
                .result_document_chunk(&conversation.id, &documents[0].id, 0)
                .unwrap()
                .text,
            "Deliverable café"
        );
    }

    #[test]
    fn changed_files_error_prevents_landing_from_being_approved() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        let state = directory.path().join("state");
        std::fs::create_dir_all(&project).unwrap();
        let config =
            Config::from_toml(include_str!("../../../../../examples/antislop.toml")).unwrap();
        let core = Core::open(&state, config.clone()).unwrap();
        let conversation = core
            .store
            .create_conversation(project.to_str().unwrap())
            .unwrap();
        let mut task = core
            .store
            .add_plan(
                &conversation.id,
                &[json!({
                    "id":"writer",
                    "title":"Write output",
                    "goal":"Create the declared text file",
                    "kind":"writing",
                    "change_kind":"artifact",
                    "delivery":"untracked",
                    "outputs":["brief.md"],
                    "reviewer":{"summary":"Check output"}
                })],
            )
            .unwrap()
            .remove(0);
        let spec: TaskContract = serde_json::from_value(task.spec.clone()).unwrap();
        let cancel = AtomicBool::new(false);
        let integration = Integration {
            core: &core,
            conversation: &conversation.id,
            task: &mut task,
            spec: &spec,
            project: &project,
            worktree: &project,
            config: &config,
            cancel: &cancel,
        };

        let result = integration.finish_outcome("base", "change", git::Integration::Applied);

        assert!(result.is_err());
        assert_ne!(task.status, status::APPROVED);
        assert!(
            core.store
                .result_documents(&conversation.id)
                .unwrap()
                .is_empty()
        );
    }
}

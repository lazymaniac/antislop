use super::super::core_feedback_helpers::land;
use super::super::*;

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
        super::super::task_failure::append_integration_report_entry(
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
        let change = git::squash(self.worktree, base, &self.spec.title)?;
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
            );
        }
        git::integrate(self.project, base, change, self.config.policy.local_commits)
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
        let onto = git::snapshot(self.project, "Current project for conflict resolution")?;
        let basis = crate::hash(serde_json::to_vec(&(
            git::tree_identity(self.project, self.task.base.as_deref().unwrap_or(change))?,
            git::tree_identity(self.project, change)?,
            git::tree_identity(self.project, &onto)?,
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
        let landed = git::changed_files(self.project, base, change).unwrap_or_default();
        let landing = super::landing::record(
            self.core,
            self.conversation,
            self.task,
            self.spec,
            &outcome,
            &landed,
        )?;
        self.complete_landing(landing)
    }

    fn complete_landing(self, landing: super::landing::Landing) -> Result<()> {
        super::super::task_failure::append_integration_report_entry(
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

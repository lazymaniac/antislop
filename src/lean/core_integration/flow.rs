use super::super::core_feedback_helpers::land;
use super::super::core_path_helpers::worktree_branch;
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

struct Landing {
    status: &'static str,
    text: String,
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
        if !self.config.policy.local_write {
            return self.leave_on_branch();
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
        add_report_note(self.task, "no file changes");
        git::remove_worktree(
            self.project,
            self.worktree,
            &worktree_branch(&self.core.state, self.task)?,
        )?;
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

    fn leave_on_branch(self) -> Result<()> {
        let branch = worktree_branch(&self.core.state, self.task)?;
        let text = format!("local writes are not allowed; the change is on branch {branch}");
        add_report_note(self.task, &text);
        land(
            self.task,
            Outcome::LeftOnBranch,
            "run",
            &text,
            None,
            Some(&branch),
            &[],
        );
        self.core.stop(
            self.conversation,
            self.task,
            status::NEEDS_USER,
            Some(&text),
        )
    }

    fn create_change(&mut self, base: &str) -> Result<String> {
        let change = git::squash(self.worktree, base, &self.spec.title)?;
        self.task.result = Some(change.clone());
        Ok(change)
    }

    fn apply_change(&self, base: &str, change: &str) -> Result<git::Integration> {
        self.core.store.assert_task_inputs(self.task)?;
        git::integrate(self.project, base, change, self.config.policy.local_commits)
    }

    fn repair_conflict(
        &mut self,
        lock: std::sync::MutexGuard<'_, ()>,
        change: &str,
        outcome: &git::Integration,
    ) -> Result<bool> {
        let Some((kind, files)) = conflict_with_files(outcome) else {
            return Ok(false);
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
            return Ok(false);
        }
        drop(lock);
        self.core.repair_integration(
            self.conversation,
            self.task,
            self.spec,
            self.worktree,
            &onto,
            change,
            kind,
            files,
            self.cancel,
        )?;
        Ok(true)
    }

    fn finish_outcome(mut self, base: &str, change: &str, outcome: git::Integration) -> Result<()> {
        let landed = git::changed_files(self.project, base, change).unwrap_or_default();
        let landing = self.record_landing(&outcome, &landed)?;
        self.append_event(&landing);
        add_report_note(self.task, &landing.text);
        if landing.status == status::APPROVED {
            git::remove_worktree(
                self.project,
                self.worktree,
                &worktree_branch(&self.core.state, self.task)?,
            )?;
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

    fn record_landing(&mut self, outcome: &git::Integration, files: &[String]) -> Result<Landing> {
        match outcome {
            git::Integration::Committed(commit) => Ok(self.committed(commit, files)),
            git::Integration::Applied => Ok(self.applied(files)),
            git::Integration::Conflict(files) => self.unresolved_conflict(files),
            git::Integration::UserEdits(files) => self.unresolved_edits(files),
        }
    }

    fn committed(&mut self, commit: &str, files: &[String]) -> Landing {
        let text = format!("committed as {commit}");
        land(
            self.task,
            Outcome::Committed,
            "run",
            &text,
            Some(commit),
            None,
            files,
        );
        Landing {
            status: status::APPROVED,
            text,
        }
    }

    fn applied(&mut self, files: &[String]) -> Landing {
        let text = "applied to the working tree, not committed".to_owned();
        land(self.task, Outcome::Applied, "run", &text, None, None, files);
        Landing {
            status: status::APPROVED,
            text,
        }
    }

    fn unresolved_conflict(&mut self, files: &[String]) -> Result<Landing> {
        let branch = worktree_branch(&self.core.state, self.task)?;
        let text = format!(
            "conflicts with the current branch in {}; the change is on branch {}",
            files.join(", "),
            branch
        );
        land(
            self.task,
            Outcome::Conflict,
            "run",
            &text,
            None,
            Some(&branch),
            files,
        );
        Ok(Landing {
            status: status::NEEDS_USER,
            text,
        })
    }

    fn unresolved_edits(&mut self, files: &[String]) -> Result<Landing> {
        let branch = worktree_branch(&self.core.state, self.task)?;
        let text = format!(
            "you have uncommitted edits in {}; the change is on branch {}",
            files.join(", "),
            branch
        );
        land(
            self.task,
            Outcome::UserEdits,
            "run",
            &text,
            None,
            Some(&branch),
            files,
        );
        Ok(Landing {
            status: status::NEEDS_USER,
            text,
        })
    }

    fn append_event(&self, landing: &Landing) {
        let _ = self.core.store.append_event(
            self.conversation,
            None,
            Some(&self.task.plan_id),
            "integration",
            &json!({
                "id": self.spec.id,
                "title": self.spec.title,
                "status": landing.status,
                "detail": tasks::bound(&landing.text, tasks::DETAIL_LIMIT)
            }),
        );
    }
}

fn conflict_with_files(outcome: &git::Integration) -> Option<(Outcome, &[String])> {
    match outcome {
        git::Integration::Conflict(files) => Some((Outcome::Conflict, files)),
        git::Integration::UserEdits(files) => Some((Outcome::UserEdits, files)),
        _ => None,
    }
}

fn add_report_note(task: &mut Task, text: &str) {
    let report = task.report.take().unwrap_or_default();
    task.report = Some(format!("{report}\n\n[integration] {text}"));
}

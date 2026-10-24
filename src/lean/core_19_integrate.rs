use super::core_feedback_helpers::land;
use super::core_path_helpers::worktree_branch;
use super::*;

impl Core {
    /// Lands an approved writer's work on the user's branch as one change.
    /// Merge against the writer's starting snapshot, preserving concurrent
    /// edits. Conflicts go back to the writer before asking the user.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::lean) fn integrate(
        self: &Arc<Self>,
        conversation: &str,
        task: &mut Task,
        spec: &TaskSpec,
        project: &Path,
        worktree: &Path,
        config: &Config,
        cancel: &AtomicBool,
    ) -> Result<()> {
        let _integration = self.integrating.lock().expect("integration mutex");
        if self.defer_agent_work(conversation, task)? {
            return Ok(());
        }
        let base = task.base.clone().unwrap_or_default();
        let result = git::head(worktree)?;
        let note = |task: &mut Task, text: String| {
            let report = task.report.take().unwrap_or_default();
            task.report = Some(format!("{report}\n\n[integration] {text}"));
        };
        if result == base {
            self.store.assert_task_inputs(task)?;
            note(task, "no file changes".into());
            git::remove_worktree(project, worktree, &worktree_branch(&self.state, task)?)?;
            land(
                task,
                Outcome::NoChanges,
                "run",
                "no file changes",
                None,
                None,
                &[],
            );
            return self.set_status(conversation, task, status::APPROVED);
        }
        let policy = &config.policy;
        if !policy.local_write {
            let branch = worktree_branch(&self.state, task)?;
            let text = format!(
                "local writes are not allowed; the change is on branch {}",
                branch
            );
            note(task, text.clone());
            land(
                task,
                Outcome::LeftOnBranch,
                "run",
                &text,
                None,
                Some(&branch),
                &[],
            );
            return self.stop(conversation, task, status::NEEDS_USER, Some(&text));
        }
        let change = git::squash(worktree, &base, &spec.title)?;
        task.result = Some(change.clone());
        let outcome = {
            self.store.assert_task_inputs(task)?;
            git::integrate(project, &base, &change, policy.local_commits)?
        };
        let conflict = match &outcome {
            git::Integration::Conflict(files) => Some((Outcome::Conflict, files)),
            git::Integration::UserEdits(files) => Some((Outcome::UserEdits, files)),
            _ => None,
        };
        if let Some((kind, files)) = conflict
            && task
                .feedback
                .as_ref()
                .and_then(|feedback| feedback["integration_repairs"].as_u64())
                .unwrap_or(0)
                < u64::from(tasks::MAX_ROUNDS)
        {
            let onto = git::snapshot(project, "Current project for conflict resolution")?;
            // No integration lock while the agent works. A later project edit
            // is merged against this snapshot when the repaired result lands.
            drop(_integration);
            return self.repair_integration(
                conversation,
                task,
                spec,
                worktree,
                &onto,
                &change,
                kind,
                files,
                cancel,
            );
        }
        let landed = git::changed_files(project, &base, &change).unwrap_or_default();
        let (status, text) = match outcome {
            git::Integration::Committed(commit) => {
                land(
                    task,
                    Outcome::Committed,
                    "run",
                    &format!("committed as {commit}"),
                    Some(&commit),
                    None,
                    &landed,
                );
                (status::APPROVED, format!("committed as {commit}"))
            }
            git::Integration::Applied => {
                land(
                    task,
                    Outcome::Applied,
                    "run",
                    "applied to the working tree, not committed",
                    None,
                    None,
                    &landed,
                );
                (
                    status::APPROVED,
                    "applied to the working tree, not committed".into(),
                )
            }
            git::Integration::Conflict(files) => {
                let branch = worktree_branch(&self.state, task)?;
                let text = format!(
                    "conflicts with the current branch in {}; the change is on branch {}",
                    files.join(", "),
                    branch
                );
                land(
                    task,
                    Outcome::Conflict,
                    "run",
                    &text,
                    None,
                    Some(&branch),
                    &files,
                );
                (status::NEEDS_USER, text)
            }
            git::Integration::UserEdits(files) => {
                let branch = worktree_branch(&self.state, task)?;
                let text = format!(
                    "you have uncommitted edits in {}; the change is on branch {}",
                    files.join(", "),
                    branch
                );
                land(
                    task,
                    Outcome::UserEdits,
                    "run",
                    &text,
                    None,
                    Some(&branch),
                    &files,
                );
                (status::NEEDS_USER, text)
            }
        };
        let _ = self.store.append_event(
            conversation,
            None,
            Some(&task.plan_id),
            "integration",
            // The event carries the bounded line; `note` below keeps the
            // whole text in the report, like every other landing detail.
            &json!({"id": spec.id, "title": spec.title, "status": status,
                    "detail": tasks::bound(&text, tasks::DETAIL_LIMIT)}),
        );
        note(task, text.clone());
        if status == status::APPROVED {
            git::remove_worktree(project, worktree, &worktree_branch(&self.state, task)?)?;
            return self.set_status(conversation, task, status);
        }
        self.stop(conversation, task, status, Some(&text))
    }
}

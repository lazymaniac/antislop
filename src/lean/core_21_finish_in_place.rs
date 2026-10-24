use super::core_feedback_helpers::land;
use super::*;

impl Core {
    /// An approved in-place change is already in the project; with commit
    /// permission exactly its files are committed.
    pub(in crate::lean) fn finish_in_place(
        &self,
        conversation: &str,
        task: &mut Task,
        spec: &TaskSpec,
        project: &Path,
        config: &Config,
    ) -> Result<()> {
        let _integration = self.integrating.lock().expect("integration mutex");
        if self.defer_agent_work(conversation, task)? {
            return Ok(());
        }
        self.store.assert_task_inputs(task)?;
        let (Some(base), Some(result)) = (task.base.clone(), task.result.clone()) else {
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
        };
        let files = git::changed_files(project, &base, &result)?;
        let commit = if files.is_empty() {
            None
        } else if config.policy.local_commits {
            Some(git::commit_paths(project, &files, &spec.title)?)
        } else {
            None
        };
        let text = if files.is_empty() {
            "no file changes".to_owned()
        } else if let Some(commit) = &commit {
            format!("committed in place as {commit}")
        } else {
            format!("applied in place, not committed: {}", files.join(", "))
        };
        let outcome = if files.is_empty() {
            Outcome::NoChanges
        } else if commit.is_some() {
            Outcome::InPlaceCommitted
        } else {
            Outcome::InPlaceApplied
        };
        land(task, outcome, "run", &text, commit.as_deref(), None, &files);
        let _ = self.store.append_event(
            conversation,
            None,
            Some(&task.plan_id),
            "integration",
            &json!({"id": spec.id, "title": spec.title, "status": status::APPROVED,
                    "detail": tasks::bound(&text, tasks::DETAIL_LIMIT)}),
        );
        let report = task.report.take().unwrap_or_default();
        task.report = Some(format!("{report}\n\n[in place] {text}"));
        self.set_status(conversation, task, status::APPROVED)
    }

    /// Ends a task whose agent turn failed or was cancelled, keeping why.
    pub(in crate::lean) fn stop_task(
        &self,
        conversation: &str,
        task: &mut Task,
        role: &str,
        cancel: &AtomicBool,
    ) -> Result<()> {
        let error = self
            .store
            .last_turn(conversation, role)?
            .and_then(|turn| turn.error)
            .unwrap_or_else(|| "no reason recorded".into());
        let next = if cancel.load(Ordering::SeqCst) {
            status::QUEUED
        } else if tasks::needs_user_action(&error) {
            status::WAITING
        } else {
            status::FAILED
        };
        let who = role.split(':').next().unwrap_or(role);
        let report = task.report.take().unwrap_or_default();
        task.report = Some(
            format!("{report}\n\n[{next}] the {who}'s turn ended: {error}")
                .trim_start()
                .to_owned(),
        );
        self.stop(
            conversation,
            task,
            next,
            Some(&format!("the {who}'s turn ended: {error}")),
        )
    }
}

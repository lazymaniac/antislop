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
            return self.finish_empty_in_place(conversation, task);
        };
        let landing = make_in_place_landing(project, spec, config, &base, &result)?;
        self.record_in_place_landing(conversation, task, spec, landing)
    }

    fn finish_empty_in_place(&self, conversation: &str, task: &mut Task) -> Result<()> {
        land(
            task,
            Outcome::NoChanges,
            "run",
            "no file changes",
            None,
            None,
            &[],
        );
        self.set_status(conversation, task, status::APPROVED)
    }

    fn record_in_place_landing(
        &self,
        conversation: &str,
        task: &mut Task,
        spec: &TaskSpec,
        landing: InPlaceLanding,
    ) -> Result<()> {
        land(
            task,
            landing.outcome,
            "run",
            &landing.text,
            landing.commit.as_deref(),
            None,
            &landing.files,
        );
        let _ = self.store.append_event(
            conversation,
            None,
            Some(&task.plan_id),
            "integration",
            &json!({"id": spec.id, "title": spec.title, "status": status::APPROVED,
                    "detail": tasks::bound(&landing.text, tasks::DETAIL_LIMIT)}),
        );
        let report = task.report.take().unwrap_or_default();
        task.report = Some(format!("{report}\n\n[in place] {}", landing.text));
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
        let next = failed_turn_status(&error, cancel);
        let who = role.split(':').next().unwrap_or(role);
        self.record_failed_turn(conversation, task, next, who, &error)
    }

    fn record_failed_turn(
        &self,
        conversation: &str,
        task: &mut Task,
        next: &str,
        role: &str,
        error: &str,
    ) -> Result<()> {
        append_failed_turn(task, next, role, error);
        self.stop(
            conversation,
            task,
            next,
            Some(&format!("the {role}'s turn ended: {error}")),
        )
    }
}

struct InPlaceLanding {
    files: Vec<String>,
    commit: Option<String>,
    text: String,
    outcome: Outcome,
}

fn make_in_place_landing(
    project: &Path,
    spec: &TaskSpec,
    config: &Config,
    base: &str,
    result: &str,
) -> Result<InPlaceLanding> {
    let files = git::changed_files(project, base, result)?;
    let commit = create_in_place_commit(project, spec, config, &files)?;
    let text = in_place_landing_text(&files, commit.as_deref());
    let outcome = in_place_outcome(&files, commit.is_some());
    Ok(InPlaceLanding {
        files,
        commit,
        text,
        outcome,
    })
}

fn create_in_place_commit(
    project: &Path,
    spec: &TaskSpec,
    config: &Config,
    files: &[String],
) -> Result<Option<String>> {
    if !files.is_empty() && config.policy.local_commits {
        Ok(Some(git::commit_paths(project, files, &spec.title)?))
    } else {
        Ok(None)
    }
}

fn in_place_landing_text(files: &[String], commit: Option<&str>) -> String {
    if files.is_empty() {
        "no file changes".into()
    } else if let Some(commit) = commit {
        format!("committed in place as {commit}")
    } else {
        format!("applied in place, not committed: {}", files.join(", "))
    }
}

fn in_place_outcome(files: &[String], committed: bool) -> Outcome {
    if files.is_empty() {
        Outcome::NoChanges
    } else if committed {
        Outcome::InPlaceCommitted
    } else {
        Outcome::InPlaceApplied
    }
}

fn failed_turn_status(error: &str, cancel: &AtomicBool) -> &'static str {
    if cancel.load(Ordering::SeqCst) {
        status::QUEUED
    } else if tasks::needs_user_action(error) {
        status::WAITING
    } else {
        status::FAILED
    }
}

fn append_failed_turn(task: &mut Task, status: &str, role: &str, error: &str) {
    let report = task.report.take().unwrap_or_default();
    task.report = Some(
        format!("{report}\n\n[{status}] the {role}'s turn ended: {error}")
            .trim_start()
            .to_owned(),
    );
}

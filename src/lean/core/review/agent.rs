//! Explicit user review requests use normal review context and durable delivery.
use crate::lean::*;

impl Core {
    pub(in crate::lean) fn resume_agent_reviews(
        self: &Arc<Self>,
        conversation: &str,
    ) -> Result<()> {
        let Some(cancel) = self.register_agent_review(conversation)? else {
            return Ok(());
        };
        self.spawn_agent_review_worker(conversation, cancel);
        Ok(())
    }

    fn register_agent_review(&self, conversation: &str) -> Result<Option<Arc<AtomicBool>>> {
        // Make the active slot visible before deletion can pass its idle check.
        let _lifecycle = self
            .locks
            .session_lifecycle
            .lock()
            .expect("session lifecycle mutex");
        self.store.conversation(conversation)?;
        let mut jobs = self
            .activity
            .agent_reviews
            .lock()
            .expect("agent reviews mutex");
        if jobs.contains_key(conversation) {
            return Ok(None);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        jobs.insert(conversation.into(), cancel.clone());
        Ok(Some(cancel))
    }

    fn spawn_agent_review_worker(self: &Arc<Self>, conversation: &str, cancel: Arc<AtomicBool>) {
        let core = self.clone();
        let conversation = conversation.to_owned();
        std::thread::spawn(move || run_review_queue(core, conversation, cancel));
    }

    fn clear_agent_review(&self, conversation: &str) {
        self.activity
            .agent_reviews
            .lock()
            .expect("agent reviews mutex")
            .remove(conversation);
    }
}

fn run_review_queue(core: Arc<Core>, conversation: String, cancel: Arc<AtomicBool>) {
    while let Some(role) = next_review_role(&core, &conversation, &cancel) {
        if let Err(error) = review_mail(&core, &conversation, role, &cancel) {
            core.notice(&conversation, &format!("{role}: {error}"));
            core.clear_agent_review(&conversation);
            break;
        }
    }
}

fn next_review_role(core: &Core, conversation: &str, cancel: &AtomicBool) -> Option<&'static str> {
    let mut jobs = core
        .activity
        .agent_reviews
        .lock()
        .expect("agent reviews mutex");
    let role = [PLAN_REVIEWER, AUDITOR].into_iter().find(|role| {
        core.store
            .pending_mail(conversation, role)
            .is_ok_and(|mail| !mail.is_empty())
    });
    if role.is_none() || cancel.load(Ordering::SeqCst) {
        jobs.remove(conversation);
        return None;
    }
    role
}

fn review_mail(
    core: &Arc<Core>,
    conversation: &str,
    role: &str,
    cancel: &AtomicBool,
) -> Result<()> {
    review_role(core, conversation, role, cancel)?;
    require(
        core.store.pending_mail(conversation, role)?.is_empty(),
        "review did not consume the request; automatic retry disabled",
    )
}

fn review_role(
    core: &Arc<Core>,
    conversation: &str,
    role: &str,
    cancel: &AtomicBool,
) -> Result<()> {
    let tasks = core.store.latest_plan(conversation)?;
    let plan = tasks.first().map(|task| task.plan_id.as_str());
    let project = PathBuf::from(core.store.conversation(conversation)?.project);
    if role == AUDITOR {
        return review_audit(core, conversation, plan, &project, cancel);
    }
    review_plan(core, conversation, role, plan, &tasks, &project, cancel)
}

fn review_audit(
    core: &Arc<Core>,
    conversation: &str,
    plan: Option<&str>,
    project: &Path,
    cancel: &AtomicBool,
) -> Result<()> {
    if let Some(plan) = plan {
        core.plan_report(conversation, plan, cancel)?;
        return Ok(());
    }
    core.review(
        conversation,
        AUDITOR,
        None,
        project,
        cancel,
        |_, request| tasks::audit_prompt(&project.to_string_lossy(), request, "No plan exists."),
    )?
    .map_err(Error::Invalid)?;
    Ok(())
}

fn review_plan(
    core: &Arc<Core>,
    conversation: &str,
    role: &str,
    plan: Option<&str>,
    tasks: &[Task],
    project: &Path,
    cancel: &AtomicBool,
) -> Result<()> {
    let specs = task_contracts(tasks)?;
    core.review(
        conversation,
        role,
        plan,
        project,
        cancel,
        |initial, request| plan_review_prompt(project, request, &specs, initial),
    )?
    .map_err(Error::Invalid)?;
    Ok(())
}

fn task_contracts(tasks: &[Task]) -> Result<Vec<TaskContract>> {
    tasks
        .iter()
        .map(|task| serde_json::from_value(task.spec.clone()))
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Error::from)
}

fn plan_review_prompt(
    project: &Path,
    request: &str,
    specs: &[TaskContract],
    initial: bool,
) -> String {
    if initial {
        tasks::plan_review_prompt(&project.to_string_lossy(), request, specs)
    } else {
        tasks::plan_review_followup(request, specs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delayed_agent_review_registration_cannot_start_after_deletion() {
        let directory = tempfile::tempdir().unwrap();
        let config = Config::from_toml(include_str!("../../../../examples/antislop.toml")).unwrap();
        let core = Core::open(&directory.path().join("state"), config).unwrap();
        let session = core.store.create_conversation(".").unwrap();

        // Hold the same lifecycle gate as deletion, queue a review resume,
        // then remove the session before allowing registration to continue.
        let lifecycle = core
            .locks
            .session_lifecycle
            .lock()
            .expect("session lifecycle mutex");
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let review_core = Arc::clone(&core);
        let conversation = session.id.clone();
        let review = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            review_core.resume_agent_reviews(&conversation)
        });
        started_rx.recv().unwrap();

        core.store.delete_session(&session.id).unwrap();
        drop(lifecycle);
        let error = review.join().unwrap().unwrap_err().to_string();

        assert!(error.contains("no conversation"), "{error}");
        assert!(core.store.conversation(&session.id).is_err());
        assert!(!core.activity.agent_review_running(&session.id));
    }
}

use super::core_prompt_helpers::blocked_report;
use super::*;

impl Core {
    /// Caller holds the integration/scope mutex, including recovery paths.
    pub(in crate::lean) fn start_plan_with_scope_lock(
        self: &Arc<Self>,
        conversation: &str,
        plan_id: &str,
        queue: impl FnOnce(&Store) -> Result<()>,
    ) -> Result<Vec<Task>> {
        self.ensure_plan_can_start(conversation)?;
        let mut plans = self.plans.lock().expect("plans mutex");
        require(
            !plans.contains_key(conversation),
            "a plan is already running",
        )?;
        self.validate_plan_inputs(conversation, plan_id)?;
        queue(&self.store)?;
        self.blocked_seen
            .lock()
            .expect("blocked mutex")
            .remove(conversation);
        let tasks = self.store.plan(plan_id)?;
        self.announce_plan_tasks(conversation, &tasks);
        let cancel = Arc::new(AtomicBool::new(false));
        plans.insert(conversation.to_owned(), cancel.clone());
        drop(plans);
        self.spawn_plan_runner(conversation, plan_id, cancel, &tasks);
        Ok(tasks)
    }

    fn ensure_plan_can_start(&self, conversation: &str) -> Result<()> {
        require(
            self.store
                .workflow(conversation)?
                .is_none_or(|workflow| workflow.phase != "interview"),
            "finish the interview and accept the reviewed plan before starting implementation",
        )
    }

    fn validate_plan_inputs(&self, conversation: &str, plan_id: &str) -> Result<()> {
        let specs = self
            .store
            .plan_specs(plan_id)?
            .into_iter()
            .map(serde_json::from_value)
            .collect::<std::result::Result<Vec<TaskSpec>, _>>()?;
        tasks::validate(&specs)?;
        let run = self.store.plan_run(plan_id)?.ok_or_else(|| {
            Error::Invalid("integrity error: plan has no requirements run".into())
        })?;
        self.validate_ledger_plan(conversation, plan_id, &run)?;
        for task in self.store.plan(plan_id)? {
            self.store.assert_task_inputs(&task)?;
        }
        Ok(())
    }

    fn announce_plan_tasks(&self, conversation: &str, tasks: &[Task]) {
        for task in tasks {
            self.announce_task(conversation, task, None);
        }
    }

    fn spawn_plan_runner(
        self: &Arc<Self>,
        conversation: &str,
        plan_id: &str,
        cancel: Arc<AtomicBool>,
        tasks: &[Task],
    ) {
        let tasks_write = tasks.iter().any(|task| task.spec["writes"] == true);
        let core = self.clone();
        let conversation = conversation.to_owned();
        let plan_id = plan_id.to_owned();
        std::thread::spawn(move || core.plan_runner(&conversation, &plan_id, &cancel, tasks_write));
    }

    fn plan_runner(
        self: &Arc<Self>,
        conversation: &str,
        plan_id: &str,
        cancel: &AtomicBool,
        tasks_write: bool,
    ) {
        self.warn_of_uncommitted(conversation, tasks_write);
        loop {
            self.run_plan(conversation, plan_id, cancel);
            let _graph_guard = self.graph_updates.lock().expect("graph update mutex");
            if !cancel.load(Ordering::SeqCst) && self.has_runnable_dependents(plan_id) {
                continue;
            }
            if cancel.load(Ordering::SeqCst) {
                self.clear_running_plan(conversation);
                return;
            }
            self.finish_plan_run(conversation, plan_id, cancel);
            break;
        }
    }

    fn has_runnable_dependents(&self, plan_id: &str) -> bool {
        self.store.plan(plan_id).is_ok_and(|tasks| {
            tasks
                .iter()
                .any(|task| task.status == status::QUEUED && dependencies_approved(task, &tasks))
        })
    }

    fn clear_running_plan(&self, conversation: &str) {
        self.plans.lock().expect("plans mutex").remove(conversation);
    }

    fn finish_plan_run(self: &Arc<Self>, conversation: &str, plan_id: &str, cancel: &AtomicBool) {
        let tasks = self.store.plan(plan_id).unwrap_or_default();
        if has_blocked_tasks(&tasks) {
            self.finish_blocked_plan(conversation, &tasks);
        } else {
            self.finish_audited_plan(conversation, plan_id, cancel);
        }
    }

    fn finish_blocked_plan(self: &Arc<Self>, conversation: &str, tasks: &[Task]) {
        let autonomous = self.autonomous(conversation).unwrap_or(false);
        let mut plans = self.plans.lock().expect("plans mutex");
        let report = blocked_report(tasks, autonomous);
        let told = self
            .blocked_seen
            .lock()
            .expect("blocked mutex")
            .insert(conversation.to_owned(), report.clone());
        if told.as_deref() != Some(report.as_str())
            && self
                .store
                .add_message(conversation, RUNTIME, &report)
                .is_ok()
        {
            self.wake(conversation);
        }
        plans.remove(conversation);
    }

    fn finish_audited_plan(
        self: &Arc<Self>,
        conversation: &str,
        plan_id: &str,
        cancel: &AtomicBool,
    ) {
        let report = self.plan_report(conversation, plan_id, cancel);
        let mut plans = self.plans.lock().expect("plans mutex");
        if let Ok(report) = report
            && self
                .store
                .add_message(conversation, RUNTIME, &report)
                .is_ok()
        {
            self.wake(conversation);
        }
        plans.remove(conversation);
    }

    /// Tell the user that worktree writers inherit their local changes.
    pub(in crate::lean) fn warn_of_uncommitted(&self, conversation: &str, writes: bool) {
        let config = self.config_snapshot();
        if !writes || config.workspace.mode == WorkspaceMode::InPlace {
            return;
        }
        let Some(text) = self.uncommitted_notice(conversation) else {
            return;
        };
        let _ = self
            .store
            .append_event(conversation, None, None, "notice", &json!({"text": text}));
    }

    fn uncommitted_notice(&self, conversation: &str) -> Option<String> {
        let project = PathBuf::from(self.store.conversation(conversation).ok()?.project);
        if !git::is_repository(&project) {
            return None;
        }
        let files = git::uncommitted(&project).ok()?;
        if files.is_empty() {
            return None;
        }
        Some(format_uncommitted_notice(&files))
    }

    pub(in crate::lean) fn wake(self: &Arc<Self>, conversation: &str) {
        let mut running = self.running.lock().expect("running mutex");
        if running.contains_key(conversation) {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        running.insert(conversation.to_owned(), cancel.clone());
        drop(running);
        let core = self.clone();
        let conversation = conversation.to_owned();
        std::thread::spawn(move || core.orchestrate(&conversation, &cancel));
    }

    pub(in crate::lean) fn orchestrate(self: &Arc<Self>, conversation: &str, cancel: &AtomicBool) {
        loop {
            let Some(upto) = self.next_orchestration_upto(conversation, cancel) else {
                return;
            };
            if self.orchestrator_turn(conversation, upto, cancel).is_err() {
                // The store itself failed; retrying would spin. The next
                // message starts a new loop.
                self.stop_orchestrator(conversation);
                return;
            }
        }
    }

    fn next_orchestration_upto(&self, conversation: &str, cancel: &AtomicBool) -> Option<u64> {
        // Decide under the lock so a message either is seen here or starts a new loop.
        let mut running = self.running.lock().expect("running mutex");
        if self
            .orchestrator_restarts
            .lock()
            .expect("orchestrator restart mutex")
            .remove(conversation)
            && let Err(error) = self.store.forget_orchestrator_sessions(conversation)
        {
            self.notice(
                conversation,
                &format!("orchestrator restart failed: {error}"),
            );
            running.remove(conversation);
            return None;
        }
        cancel.store(false, Ordering::SeqCst);
        match self.unanswered(conversation) {
            Ok(Some(upto)) => Some(upto),
            _ => {
                running.remove(conversation);
                None
            }
        }
    }

    fn stop_orchestrator(&self, conversation: &str) {
        self.running
            .lock()
            .expect("running mutex")
            .remove(conversation);
    }
}

fn format_uncommitted_notice(files: &[String]) -> String {
    let shown: Vec<&str> = files.iter().take(10).map(String::as_str).collect();
    let more = files.len().saturating_sub(shown.len());
    format!(
        "uncommitted changes in {}{}: writers include these local changes in their starting snapshot; \
         later edits remain protected during integration",
        shown.join(", "),
        if more > 0 {
            format!(" and {more} more")
        } else {
            String::new()
        }
    )
}

fn dependencies_approved(task: &Task, tasks: &[Task]) -> bool {
    task.spec["depends_on"]
        .as_array()
        .is_some_and(|dependencies| {
            dependencies.iter().all(|dependency| {
                tasks.iter().any(|upstream| {
                    upstream.spec["id"] == *dependency && upstream.status == status::APPROVED
                })
            })
        })
}

fn has_blocked_tasks(tasks: &[Task]) -> bool {
    tasks.iter().any(|task| {
        matches!(
            task.status.as_str(),
            status::STALE | status::NEEDS_USER | status::WAITING | status::FAILED
        )
    })
}

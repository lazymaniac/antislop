use super::*;

#[path = "runner/reporting.rs"]
mod reporting;
use reporting::has_blocked_tasks;

impl Core {
    pub(super) fn plan_runner(
        self: &Arc<Self>,
        conversation: &str,
        plan_id: &str,
        cancel: &AtomicBool,
        tasks_write: bool,
    ) {
        self.warn_of_uncommitted(conversation, tasks_write);
        while !self.run_plan_cycle(conversation, plan_id, cancel) {}
    }

    fn run_plan_cycle(
        self: &Arc<Self>,
        conversation: &str,
        plan_id: &str,
        cancel: &AtomicBool,
    ) -> bool {
        self.run_plan(conversation, plan_id, cancel);
        self.wait_for_revision_jobs(conversation, cancel);
        let graph = self.graph_updates.lock().expect("graph update mutex");
        if cancel.load(Ordering::SeqCst) {
            self.clear_running_plan(conversation);
            return true;
        }
        if self.has_runnable_dependents(plan_id) {
            return false;
        }
        drop(graph);
        self.finish_plan_run(conversation, plan_id, cancel)
    }

    fn wait_for_revision_jobs(&self, conversation: &str, cancel: &AtomicBool) {
        while self
            .revision_jobs
            .lock()
            .expect("revision jobs mutex")
            .contains_key(conversation)
            && !cancel.load(Ordering::SeqCst)
        {
            std::thread::sleep(std::time::Duration::from_millis(25));
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

    fn finish_plan_run(
        self: &Arc<Self>,
        conversation: &str,
        plan_id: &str,
        cancel: &AtomicBool,
    ) -> bool {
        let graph = self.graph_updates.lock().expect("graph update mutex");
        if self.has_runnable_dependents(plan_id) {
            return false;
        }
        let tasks = self.store.plan(plan_id).unwrap_or_default();
        if has_blocked_tasks(&tasks) {
            self.finish_blocked_plan(conversation, &tasks);
            true
        } else {
            drop(graph);
            self.finish_audited_plan(conversation, plan_id, cancel)
        }
    }

    fn finish_audited_plan(
        self: &Arc<Self>,
        conversation: &str,
        plan_id: &str,
        cancel: &AtomicBool,
    ) -> bool {
        let snapshot = self.audited_plan_snapshot(plan_id);
        let report = self.plan_report(conversation, plan_id, cancel);
        let _graph = self.graph_updates.lock().expect("graph update mutex");
        if self.audited_plan_changed(conversation, plan_id, cancel, &snapshot) {
            return false;
        }
        self.publish_plan_report(conversation, report);
        true
    }

    fn audited_plan_snapshot(&self, plan_id: &str) -> AuditedPlanSnapshot {
        let revision = self.store.plan_revision(plan_id).ok();
        let basis = self
            .store
            .plan(plan_id)
            .map(|tasks| plan_scope_hash(&tasks))
            .ok();
        AuditedPlanSnapshot { revision, basis }
    }

    fn audited_plan_changed(
        &self,
        conversation: &str,
        plan_id: &str,
        cancel: &AtomicBool,
        snapshot: &AuditedPlanSnapshot,
    ) -> bool {
        !cancel.load(Ordering::SeqCst)
            && (self
                .revision_jobs
                .lock()
                .expect("revision jobs mutex")
                .contains_key(conversation)
                || self.store.plan_revision(plan_id).ok() != snapshot.revision
                || self
                    .store
                    .plan(plan_id)
                    .map(|tasks| plan_scope_hash(&tasks))
                    .ok()
                    != snapshot.basis
                || self.has_runnable_dependents(plan_id))
    }

    fn publish_plan_report(self: &Arc<Self>, conversation: &str, report: Result<String>) {
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
        if !writes {
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
            if self
                .next_orchestration_turn(conversation, upto, cancel)
                .is_err()
            {
                // The store itself failed; retrying would spin. The next
                // message starts a new loop.
                self.stop_orchestrator(conversation);
                return;
            }
        }
    }

    /// The first plan is drafted by the planner; every other turn is the orchestrator's.
    fn next_orchestration_turn(
        self: &Arc<Self>,
        conversation: &str,
        upto: u64,
        cancel: &AtomicBool,
    ) -> Result<()> {
        if self.planner_owns_turn(conversation, upto)? {
            self.planner_turn(conversation, upto, cancel)
        } else {
            self.orchestrator_turn(conversation, upto, cancel)
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

struct AuditedPlanSnapshot {
    revision: Option<i64>,
    basis: Option<String>,
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

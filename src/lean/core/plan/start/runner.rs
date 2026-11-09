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
        loop {
            match self.run_plan_cycle(conversation, plan_id, cancel) {
                Ok(false) => {}
                Ok(true) => break,
                Err(error) => {
                    self.notice(conversation, &format!("plan {plan_id} stopped: {error}"));
                    self.clear_running_plan(conversation);
                    break;
                }
            }
        }
    }

    fn run_plan_cycle(
        self: &Arc<Self>,
        conversation: &str,
        plan_id: &str,
        cancel: &AtomicBool,
    ) -> Result<bool> {
        self.run_plan(conversation, plan_id, cancel);
        self.wait_for_revision_jobs(conversation, cancel);
        let graph = self.locks.graph();
        if cancel.load(Ordering::SeqCst) {
            self.clear_running_plan(conversation);
            return Ok(true);
        }
        if self.has_runnable_dependents(plan_id)? {
            return Ok(false);
        }
        drop(graph);
        self.finish_plan_run(conversation, plan_id, cancel)
    }

    fn wait_for_revision_jobs(&self, conversation: &str, cancel: &AtomicBool) {
        while self
            .revisions
            .jobs
            .lock()
            .expect("revision jobs mutex")
            .contains_key(conversation)
            && !cancel.load(Ordering::SeqCst)
        {
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
    }

    pub(in crate::lean) fn has_runnable_dependents(&self, plan_id: &str) -> Result<bool> {
        let tasks = self.store.plan(plan_id)?;
        Ok(tasks.iter().any(|task| {
            task.status == status::QUEUED
                && task.spec["deferred"] != true
                && dependencies_approved(task, &tasks)
        }))
    }

    fn clear_running_plan(&self, conversation: &str) {
        self.activity
            .plans
            .lock()
            .expect("plans mutex")
            .remove(conversation);
    }

    fn finish_plan_run(
        self: &Arc<Self>,
        conversation: &str,
        plan_id: &str,
        cancel: &AtomicBool,
    ) -> Result<bool> {
        let graph = self.locks.graph();
        if self.has_runnable_dependents(plan_id)? {
            return Ok(false);
        }
        let tasks = self.store.plan(plan_id)?;
        if has_blocked_tasks(&tasks) {
            self.finish_blocked_plan(conversation, &tasks, cancel);
            Ok(true)
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
    ) -> Result<bool> {
        let snapshot = self.audited_plan_snapshot(plan_id)?;
        let report = self.plan_report(conversation, plan_id, cancel);
        let _graph = self.locks.graph();
        if self.audited_plan_changed(conversation, plan_id, cancel, &snapshot)? {
            return Ok(false);
        }
        self.publish_plan_report(conversation, report, cancel);
        Ok(true)
    }

    fn audited_plan_snapshot(&self, plan_id: &str) -> Result<AuditedPlanSnapshot> {
        let revision = self.store.plan_revision(plan_id)?;
        let basis = task_state_digest(&self.store.plan(plan_id)?);
        Ok(AuditedPlanSnapshot { revision, basis })
    }

    fn audited_plan_changed(
        &self,
        conversation: &str,
        plan_id: &str,
        cancel: &AtomicBool,
        snapshot: &AuditedPlanSnapshot,
    ) -> Result<bool> {
        Ok(!cancel.load(Ordering::SeqCst)
            && (self
                .revisions
                .jobs
                .lock()
                .expect("revision jobs mutex")
                .contains_key(conversation)
                || self.store.plan_revision(plan_id)? != snapshot.revision
                || task_state_digest(&self.store.plan(plan_id)?) != snapshot.basis
                || self.has_runnable_dependents(plan_id)?))
    }

    fn publish_plan_report(
        self: &Arc<Self>,
        conversation: &str,
        report: Result<String>,
        cancel: &AtomicBool,
    ) {
        let mut plans = self.activity.plans.lock().expect("plans mutex");
        // Cancellation and the plan-to-orchestrator handoff use this same
        // lock order, so a cancelled plan cannot wake a new uncancelled turn.
        if cancel.load(Ordering::SeqCst) {
            plans.remove(conversation);
            return;
        }
        match report.and_then(|report| self.store.add_message(conversation, RUNTIME, &report)) {
            Ok(_) => self.wake(conversation),
            Err(error) => self.notice(
                conversation,
                &format!("plan report could not be published: {error}"),
            ),
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
        let _lifecycle = self
            .locks
            .session_lifecycle
            .lock()
            .expect("session lifecycle mutex");
        crate::lean::wake_locked(self, conversation);
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

    /// The planner drafts and corrects plans; other turns belong to the orchestrator.
    fn next_orchestration_turn(
        self: &Arc<Self>,
        conversation: &str,
        upto: u64,
        cancel: &AtomicBool,
    ) -> Result<()> {
        // A registered source may outlive an interrupted planning pass. Refresh
        // its coverage before coordinating, without forcing a replacement plan.
        if let Err(error) = self.refresh_requirements_if_needed(conversation, cancel) {
            self.notice(
                conversation,
                &format!("could not refresh requirements source coverage: {error}"),
            );
            return Err(error);
        }
        if self.planner_owns_turn(conversation, upto)? {
            self.planner_turn(conversation, upto, cancel)
        } else {
            self.orchestrator_turn(conversation, upto, cancel)
        }
    }

    fn next_orchestration_upto(&self, conversation: &str, cancel: &AtomicBool) -> Option<u64> {
        // Decide under the lock so a message either is seen here or starts a new loop.
        let mut running = self.activity.running.lock().expect("running mutex");
        if self
            .activity
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
            Ok(None) => {
                running.remove(conversation);
                None
            }
            Err(error) => {
                self.notice(
                    conversation,
                    &format!("could not read pending conversation messages: {error}"),
                );
                running.remove(conversation);
                None
            }
        }
    }

    fn stop_orchestrator(&self, conversation: &str) {
        self.activity
            .running
            .lock()
            .expect("running mutex")
            .remove(conversation);
    }
}

struct AuditedPlanSnapshot {
    revision: i64,
    basis: String,
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
        .is_none_or(|dependencies| {
            dependencies.iter().all(|dependency| {
                tasks.iter().any(|upstream| {
                    upstream.spec["id"] == *dependency
                        && upstream.status == status::APPROVED
                        && upstream.spec["deferred"] != true
                })
            })
        })
}

#[cfg(test)]
mod deferred_tests {
    use super::*;
    #[test]
    fn a_plan_read_error_is_reported_and_releases_the_running_plan() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::open(
            &dir.path().join("state"),
            Config::from_toml(include_str!("../../../../../examples/antislop.toml")).unwrap(),
        )
        .unwrap();
        let conversation = core
            .store
            .create_conversation(dir.path().to_str().unwrap())
            .unwrap()
            .id;
        let task = core
            .store
            .add_plan(
                &conversation,
                &[json!({"id":"bad","title":"Bad","goal":"Do work","change_kind": "none", "kind":"analysis"})],
            )
            .unwrap()
            .remove(0);
        core.store
            .db()
            .execute(
                "UPDATE tasks SET spec='invalid-json',status='queued' WHERE id=?1",
                [&task.id],
            )
            .unwrap();
        let error = core
            .has_runnable_dependents(&task.plan_id)
            .unwrap_err()
            .to_string();
        core.activity
            .plans
            .lock()
            .unwrap()
            .insert(conversation.clone(), Arc::new(AtomicBool::new(false)));
        core.plan_runner(&conversation, &task.plan_id, &AtomicBool::new(false), false);
        assert!(!core.activity.plan_running(&conversation));
        let events = core.store.events_since(&conversation, 0, 100).unwrap();
        assert!(
            events.iter().any(|e| e.kind == "notice"
                && e.payload["text"]
                    .as_str()
                    .is_some_and(|s| s.contains(&error))),
            "raw error must remain actionable"
        );
        assert!(
            core.store
                .messages_by(&conversation, RUNTIME, 0)
                .unwrap()
                .is_empty(),
            "failed reads must never produce a successful plan report"
        );
    }

    #[test]
    fn held_work_returns_control_instead_of_spinning_the_plan_runner() {
        let dir = tempfile::tempdir().unwrap();
        let config =
            Config::from_toml(include_str!("../../../../../examples/antislop.toml")).unwrap();
        let core = Core::open(&dir.path().join("state"), config).unwrap();
        let conversation = core
            .store
            .create_conversation(dir.path().to_str().unwrap())
            .unwrap()
            .id;
        let mut tasks = core.store.add_plan(&conversation, &[json!({"id":"future","title":"Future","goal":"Preserve future work","change_kind": "code", "kind":"implementation","deferred":true})]).unwrap();
        tasks[0].status = status::QUEUED.into();
        core.store.update_task(&tasks[0]).unwrap();
        // Suppress only notification delivery; no real provider is started by the test.
        core.activity
            .running
            .lock()
            .unwrap()
            .insert(conversation.clone(), Arc::new(AtomicBool::new(false)));
        assert!(!core.has_runnable_dependents(&tasks[0].plan_id).unwrap());
        assert!(
            core.run_plan_cycle(&conversation, &tasks[0].plan_id, &AtomicBool::new(false))
                .unwrap()
        );
        let reports = core.store.messages_by(&conversation, RUNTIME, 0).unwrap();
        assert!(
            reports
                .iter()
                .any(|report| report.text.contains("\"deferred\":true"))
        );
        assert!(
            core.store
                .last_turn(&conversation, &format!("worker:{}", tasks[0].id))
                .unwrap()
                .is_none()
        );
    }
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;

    #[test]
    fn cancelled_plan_does_not_publish_or_wake_orchestration() {
        let directory = tempfile::tempdir().unwrap();
        let mut configuration = Config::from_toml(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/examples/antislop.toml"
        )))
        .unwrap();
        for provider in configuration.providers.values_mut() {
            provider.binary = directory.path().join("must-not-launch");
            provider.command = None;
        }
        let core = Core::open(&directory.path().join("state"), configuration).unwrap();
        let conversation = core
            .store
            .create_conversation(directory.path().to_str().unwrap())
            .unwrap()
            .id;
        for blocked in [false, true] {
            let cancel = Arc::new(AtomicBool::new(true));
            core.activity
                .plans
                .lock()
                .unwrap()
                .insert(conversation.clone(), cancel.clone());
            if blocked {
                core.finish_blocked_plan(&conversation, &[], &cancel);
            } else {
                core.publish_plan_report(
                    &conversation,
                    Ok("must not restart cancelled work".into()),
                    &cancel,
                );
            }
            assert!(core.is_idle(&conversation));
            assert!(
                core.store
                    .messages_after(&conversation, 0)
                    .unwrap()
                    .is_empty()
            );
            assert!(
                core.store
                    .last_turn(&conversation, ORCHESTRATOR)
                    .unwrap()
                    .is_none()
            );
        }
    }
}

#[cfg(test)]
mod send_deletion_tests {
    use super::*;

    #[test]
    fn send_registration_is_serialized_with_deletion() {
        let directory = tempfile::tempdir().unwrap();
        let config =
            Config::from_toml(include_str!("../../../../../examples/antislop.toml")).unwrap();
        let core = Core::open(&directory.path().join("state"), config).unwrap();
        let session = core.store.create_conversation(".").unwrap();

        // Queue deletion behind the lifecycle gate, then perform the exact
        // write-and-wake section used by send while still holding that gate.
        let lifecycle = core
            .locks
            .session_lifecycle
            .lock()
            .expect("session lifecycle mutex");
        let deleting_core = core.clone();
        let conversation = session.id.clone();
        let deleting = std::thread::spawn(move || deleting_core.delete_session(&conversation));
        while core.activity.plans.try_lock().is_ok() {
            std::thread::yield_now();
        }

        super::super::record_message_and_wake_locked(
            &core,
            &session.id,
            "Keep this accepted request",
        )
        .unwrap();
        drop(lifecycle);

        let error = deleting.join().unwrap().unwrap_err().to_string();
        assert!(error.contains("stop this session's work"), "{error}");
        assert_eq!(core.store.messages_after(&session.id, 0).unwrap().len(), 1);
    }
}

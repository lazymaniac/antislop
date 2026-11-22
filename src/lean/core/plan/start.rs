mod runner;
#[cfg(test)]
mod tests;

use crate::lean::*;

impl Core {
    /// Prepare outside shared locks, then publish one revalidated start.
    pub(in crate::lean) fn start_plan_with_scope_lock(
        self: &Arc<Self>,
        conversation: &str,
        plan_id: &str,
        queue: impl FnOnce(&Store, &AtomicBool) -> Result<Vec<Task>>,
    ) -> Result<Vec<Task>> {
        let operation = self.operation(conversation)?;
        let starting = self.locks.turn(conversation, "plan-start");
        let _starting = mail::lock_cancellable(&starting, &operation.cancel)
            .ok_or_else(|| Error::Invalid("plan start cancelled".into()))?;
        require(
            !self.activity.plan_running(conversation),
            "a plan is already running",
        )?;
        // Repository and CLI validation run with a tracked reservation and no
        // global maps or graph/integration guards held.
        self.preflight_plan_start(conversation, plan_id)?;
        let graph = self.locks.graph();
        let integration = self.locks.integration();
        require(
            !operation.cancel.load(Ordering::SeqCst),
            "plan start cancelled",
        )?;
        self.require_accepting()?;
        self.ensure_plan_can_start(conversation)?;
        self.validate_plan_inputs(conversation, plan_id)?;
        require(
            !self.activity.plan_running(conversation),
            "a plan is already running",
        )?;
        let tasks = queue(&self.store, &operation.cancel)?;

        self.activity
            .blocked_seen
            .lock()
            .expect("blocked mutex")
            .remove(conversation);
        self.announce_plan_tasks(conversation, &tasks);
        // Cancellation after queue's commit belongs to the runner too. Keep
        // the preparation lease until the runner slot has been published.
        let cancel = operation.cancel.clone();
        let mut plans = self.activity.plans.lock().expect("plans mutex");
        plans.insert(conversation.to_owned(), cancel.clone());
        drop(plans);
        drop(integration);
        drop(graph);
        self.spawn_plan_runner(conversation, plan_id, cancel, &tasks);
        Ok(tasks)
    }

    pub(in crate::lean) fn preflight_plan_start(
        &self,
        conversation: &str,
        plan: &str,
    ) -> Result<()> {
        require(
            !crate::lean::daemon::activation::pending(self),
            "runtime activation is pending; retry after the successor confirms readiness",
        )?;
        self.ensure_plan_can_start(conversation)?;
        self.ensure_profiles_launch(plan)?;
        self.validate_plan_inputs(conversation, plan)?;
        let project = PathBuf::from(self.store.conversation(conversation)?.project);
        if git::is_repository(&project) {
            self.create_session_branch(conversation, &project)?;
        }
        Ok(())
    }

    /// Refuses to start a team whose CLIs cannot be launched, naming the
    /// profiles that could, instead of failing task by task once it runs.
    fn ensure_profiles_launch(&self, plan_id: &str) -> Result<()> {
        let kinds = self
            .store
            .plan_specs(plan_id)?
            .into_iter()
            .map(serde_json::from_value::<TaskContract>)
            .collect::<std::result::Result<Vec<_>, _>>()?
            .into_iter()
            .map(|spec| spec.kind);
        let problems = crate::lean::doctor::launch_problems(&self.config_snapshot(), kinds);
        require(problems.is_empty(), &problems.join("; "))
    }

    fn ensure_plan_can_start(&self, conversation: &str) -> Result<()> {
        require(
            !self.store.workflow(conversation)?.interviewing(),
            "finish the interview and accept the reviewed plan before starting implementation",
        )
    }

    fn validate_plan_inputs(&self, conversation: &str, plan_id: &str) -> Result<()> {
        let specs = self
            .store
            .plan_specs(plan_id)?
            .into_iter()
            .map(serde_json::from_value)
            .collect::<std::result::Result<Vec<TaskContract>, _>>()?;
        tasks::validate(&specs)?;
        let run = self
            .store
            .plan_scope_id(plan_id)?
            .ok_or_else(|| Error::Invalid("integrity error: plan has no scope".into()))?;
        self.store.scope(conversation, &run)?;
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
        let tasks_write = tasks.iter().any(|task| {
            task.spec["change_kind"]
                .as_str()
                .is_some_and(|kind| kind != "none")
        });
        let core = self.clone();
        let conversation = conversation.to_owned();
        let plan_id = plan_id.to_owned();
        std::thread::spawn(move || core.plan_runner(&conversation, &plan_id, &cancel, tasks_write));
    }
}

mod runner;

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
        let authorized: bool = self.store.db().query_row("SELECT EXISTS(SELECT 1 FROM plan_authorizations WHERE plan_id=?1 AND conversation_id=?2)",rusqlite::params![plan_id,conversation],|r|r.get(0))?;
        require(authorized, "plan implementation has not been authorized")?;

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
}

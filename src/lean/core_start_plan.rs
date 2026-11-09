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
        let mut plans = self.activity.plans.lock().expect("plans mutex");
        require(
            !plans.contains_key(conversation),
            "a plan is already running",
        )?;
        self.validate_plan_inputs(conversation, plan_id)?;
        self.ensure_profiles_launch(plan_id)?;
        queue(&self.store)?;
        let activated: bool = self.store.db().query_row(
            "SELECT EXISTS(SELECT 1 FROM plan_activations WHERE plan_id=?1 AND conversation_id=?2)",
            rusqlite::params![plan_id, conversation],
            |r| r.get(0),
        )?;
        require(activated, "plan is not active")?;

        self.activity
            .blocked_seen
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
        let problems = super::doctor::launch_problems(&self.config_snapshot(), kinds);
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

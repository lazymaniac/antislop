use super::super::*;

type InFlightWriters = HashMap<String, Vec<String>>;

const PLAN_POLL: std::time::Duration = std::time::Duration::from_secs(1);

pub(super) fn run(core: &Arc<Core>, conversation: &str, plan_id: &str, cancel: &AtomicBool) {
    std::thread::scope(|scope| {
        let mut scheduler = Scheduler::new(core, conversation, plan_id, cancel);
        scheduler.run(scope);
    });
}

struct Scheduler<'a> {
    core: &'a Arc<Core>,
    conversation: &'a str,
    plan_id: &'a str,
    cancel: &'a AtomicBool,
    done_tx: std::sync::mpsc::Sender<String>,
    done_rx: std::sync::mpsc::Receiver<String>,
    in_flight: std::collections::HashSet<String>,
    writers: InFlightWriters,
}

impl<'a> Scheduler<'a> {
    fn new(
        core: &'a Arc<Core>,
        conversation: &'a str,
        plan_id: &'a str,
        cancel: &'a AtomicBool,
    ) -> Self {
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        Self {
            core,
            conversation,
            plan_id,
            cancel,
            done_tx,
            done_rx,
            in_flight: Default::default(),
            writers: Default::default(),
        }
    }

    fn run<'scope, 'env>(&mut self, scope: &'scope std::thread::Scope<'scope, 'env>)
    where
        'a: 'scope,
    {
        loop {
            let settled = self.schedule_ready(scope);
            if settled {
                continue;
            }
            if self.in_flight.is_empty() {
                break;
            }
            if !self.cancel.load(Ordering::SeqCst) {
                self.core
                    .report_stopped_tasks(self.conversation, self.plan_id);
            }
            if !self.receive_completion() {
                break;
            }
        }
    }

    fn schedule_ready<'scope, 'env>(
        &mut self,
        scope: &'scope std::thread::Scope<'scope, 'env>,
    ) -> bool
    where
        'a: 'scope,
    {
        let _graph_guard = self.core.graph_updates.lock().expect("graph update mutex");
        let Ok(mut tasks) = self.core.store.plan(self.plan_id) else {
            return false;
        };
        let state = task_states(&tasks);
        let mut settled = false;
        for task in &mut tasks {
            settled |= self.schedule_one(scope, task, &state);
        }
        settled
    }

    fn schedule_one<'scope, 'env>(
        &mut self,
        scope: &'scope std::thread::Scope<'scope, 'env>,
        task: &mut Task,
        state: &HashMap<String, String>,
    ) -> bool
    where
        'a: 'scope,
    {
        let id = task.spec["id"].as_str().unwrap_or_default().to_owned();
        if self
            .core
            .revising
            .lock()
            .expect("revision mutex")
            .contains(&task.id)
            || !self.is_runnable(task, &id)
        {
            return false;
        }
        if task.status == status::SKIPPED {
            let _ = self
                .core
                .set_status(self.conversation, task, status::QUEUED);
            return true;
        }
        if self.cancel.load(Ordering::SeqCst) || !dependencies_approved(task, state) {
            return false;
        }
        self.schedule_if_available(scope, task, id);
        false
    }

    fn is_runnable(&self, task: &Task, id: &str) -> bool {
        matches!(task.status.as_str(), status::QUEUED | status::SKIPPED)
            && !self.in_flight.contains(id)
    }

    fn schedule_if_available<'scope, 'env>(
        &mut self,
        scope: &'scope std::thread::Scope<'scope, 'env>,
        task: &Task,
        id: String,
    ) where
        'a: 'scope,
    {
        if self.at_capacity() || self.conflicts_with_writer(task) {
            return;
        }
        self.reserve(task, &id);
        self.spawn_task(scope, task.clone(), id);
    }

    fn at_capacity(&self) -> bool {
        self.in_flight.len() >= self.core.config_snapshot().policy.max_parallel
    }

    fn conflicts_with_writer(&self, task: &Task) -> bool {
        let writes = task.spec["writes"].as_bool().unwrap_or(false);
        let scope: Vec<String> =
            serde_json::from_value(task.spec["scope"].clone()).unwrap_or_default();
        writes
            && (self
                .writers
                .values()
                .any(|active| tasks::scopes_overlap(&scope, active)))
    }

    fn reserve(&mut self, task: &Task, id: &str) {
        self.in_flight.insert(id.to_owned());
        if task.spec["writes"].as_bool().unwrap_or(false) {
            let scope = serde_json::from_value(task.spec["scope"].clone()).unwrap_or_default();
            self.writers.insert(id.to_owned(), scope);
        }
    }

    fn spawn_task<'scope, 'env>(
        &self,
        scope: &'scope std::thread::Scope<'scope, 'env>,
        task: Task,
        id: String,
    ) where
        'a: 'scope,
    {
        let tx = self.done_tx.clone();
        let task_cancel = Arc::new(AtomicBool::new(self.cancel.load(Ordering::SeqCst)));
        self.core
            .task_cancels
            .lock()
            .expect("task cancel mutex")
            .insert(
                task.id.clone(),
                (self.conversation.into(), task_cancel.clone()),
            );
        let core = self.core;
        let conversation = self.conversation;
        let plan_cancel = self.cancel;
        scope.spawn(move || {
            execute_task(core, conversation, task, task_cancel, plan_cancel);
            let _ = tx.send(id);
        });
    }

    /// Waits for a task to finish, but not past `PLAN_POLL`: a plan revised
    /// while tasks run can requeue others, and they start without waiting
    /// for the running ones.
    fn receive_completion(&mut self) -> bool {
        let finished = match self.done_rx.recv_timeout(PLAN_POLL) {
            Ok(finished) => finished,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => return true,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return false,
        };
        self.release(&finished);
        while let Ok(more) = self.done_rx.try_recv() {
            self.release(&more);
        }
        true
    }

    fn release(&mut self, id: &str) {
        self.in_flight.remove(id);
        self.writers.remove(id);
    }
}

fn task_states(tasks: &[Task]) -> HashMap<String, String> {
    tasks
        .iter()
        .map(|task| {
            (
                task.spec["id"].as_str().unwrap_or_default().to_owned(),
                task.status.clone(),
            )
        })
        .collect()
}

fn dependencies_approved(task: &Task, states: &HashMap<String, String>) -> bool {
    let dependencies: Vec<String> =
        serde_json::from_value(task.spec["depends_on"].clone()).unwrap_or_default();
    dependencies
        .iter()
        .all(|id| states.get(id).map(String::as_str) == Some(status::APPROVED))
}

fn execute_task(
    core: &Arc<Core>,
    conversation: &str,
    mut task: Task,
    cancel: Arc<AtomicBool>,
    _plan_cancel: &AtomicBool,
) {
    fail_task(core, conversation, &mut task, &cancel);
    restart_agent(core, conversation, &task);
}

fn fail_task(core: &Arc<Core>, conversation: &str, task: &mut Task, cancel: &AtomicBool) {
    if let Err(error) = core.run_task(conversation, task, cancel) {
        append_failure(core, conversation, task, &error);
    }
}

fn append_failure(core: &Arc<Core>, conversation: &str, task: &mut Task, error: &Error) {
    let report = task.report.take().unwrap_or_default();
    task.report = Some(
        format!("{report}\n\n[runtime] {error}")
            .trim_start()
            .to_owned(),
    );
    let _ = core.stop(conversation, task, status::FAILED, Some(&error.to_string()));
}

fn restart_agent(core: &Arc<Core>, conversation: &str, task: &Task) {
    let restart = {
        let mut active = core.task_cancels.lock().expect("task cancel mutex");
        active.remove(&task.id);
        core.task_restarts
            .lock()
            .expect("task restart mutex")
            .remove(&task.id)
    };
    if let Some(mut request) = restart {
        let result = core
            .task_freshness_token(conversation, &request.plan, &request.task)
            .and_then(|token| {
                request.freshness_token = token;
                core.recover_task(request)
            });
        if let Err(error) = result {
            core.notice(conversation, &format!("agent restart failed: {error}"));
        }
    }
}

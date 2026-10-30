use super::*;

mod scheduler;

impl Core {
    /// Stores one check the verifier reported and tells frontends, when it
    /// changed what they show.
    pub(in crate::lean) fn record_check(
        &self,
        conversation: &str,
        task: &Task,
        update: &tasks::CheckUpdate,
    ) {
        if self.store.assert_task_inputs(task).is_err() {
            return;
        }
        let state = if update.status.is_some() {
            "settled"
        } else {
            "checking"
        };
        let changed = self.store.set_check(
            &task.id,
            task.round,
            &update.id,
            state,
            update.status.as_deref(),
            &update.note,
        );
        if changed.unwrap_or(false) {
            self.publish_check(conversation, task, update, state);
        }
    }

    fn publish_check(
        &self,
        conversation: &str,
        task: &Task,
        update: &tasks::CheckUpdate,
        state: &str,
    ) {
        let _ = self.store.append_event(
            conversation,
            None,
            Some(&task.plan_id),
            "check",
            &json!({"id": task.spec["id"], "task": task.id, "round": task.round,
                    "check": update.id, "state": state, "status": update.status}),
        );
    }

    /// Dynamically schedules and runs every task whose dependencies are approved
    /// and whose write scope does not conflict with currently running writers.
    /// Tasks start immediately as soon as their dependencies finish rather than
    /// waiting for unrelated slow tasks in the same wave.
    pub(in crate::lean) fn run_plan(
        self: &Arc<Self>,
        conversation: &str,
        plan_id: &str,
        cancel: &AtomicBool,
    ) {
        scheduler::run(self, conversation, plan_id, cancel);
    }
}

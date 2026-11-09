//! Superseded execution leases stop before their scheduler slots are reused.
use crate::lean::*;

impl Core {
    /// Caller holds the graph and integration locks across the input update.
    pub(in crate::lean) fn cancel_changed_task_inputs(&self, before: &[Task], after: &[Task]) {
        let active = self
            .activity
            .task_cancels
            .lock()
            .expect("task cancel mutex");
        for task in after {
            if before
                .iter()
                .any(|old| old.id == task.id && old.input_generation != task.input_generation)
            {
                self.queue_worktree_cleanup(&task.id);
                if let Some((_, cancel)) = active.get(&task.id) {
                    cancel.store(true, Ordering::SeqCst);
                }
            }
        }
    }

    pub(in crate::lean) fn invalidate_task_dependents(
        &self,
        conversation: &str,
        task: &Task,
    ) -> Result<()> {
        let _graph = self.locks.graph();
        let _integration = self.locks.integration();
        self.store.assert_task_inputs(task)?;
        let before = self.store.plan(&task.plan_id)?;
        self.store.invalidate_task_dependents(conversation, task)?;
        let after = self.store.plan(&task.plan_id)?;
        self.cancel_changed_task_inputs(&before, &after);
        for changed in after.iter().filter(|current| {
            before
                .iter()
                .any(|old| old.id == current.id && old.input_generation != current.input_generation)
        }) {
            self.announce_task(conversation, changed, Some("dependency inputs changed"));
        }
        Ok(())
    }
}

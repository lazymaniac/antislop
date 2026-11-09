use crate::lean::*;

pub(in crate::lean) mod reviewer_setup;
pub(in crate::lean) mod task_execution;

impl Core {
    /// Runs the task-specific worker and reviewer lifecycle.
    #[cfg(test)]
    pub(in crate::lean) fn run_task(
        self: &Arc<Self>,
        conversation: &str,
        task: &mut Task,
        cancel: &AtomicBool,
    ) -> Result<()> {
        let lease = self.task_worktree_lease(task)?;
        self.run_task_with_lease(conversation, task, cancel, lease)
    }

    pub(in crate::lean) fn run_task_with_lease(
        self: &Arc<Self>,
        conversation: &str,
        task: &mut Task,
        cancel: &AtomicBool,
        lease: Option<crate::lean::core::worktree::cleanup::WorktreeLease<'_>>,
    ) -> Result<()> {
        task_execution::Execution::new(self, conversation, task, cancel, lease)?.run()
    }
}

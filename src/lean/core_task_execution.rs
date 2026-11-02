use super::*;

pub(in crate::lean) mod reviewer_setup;
pub(in crate::lean) mod task_execution;

impl Core {
    /// Runs the task-specific worker and reviewer lifecycle.
    pub(in crate::lean) fn run_task(
        self: &Arc<Self>,
        conversation: &str,
        task: &mut Task,
        cancel: &AtomicBool,
    ) -> Result<()> {
        task_execution::Execution::new(self, conversation, task, cancel)?.run()
    }
}

use super::*;

mod task_execution;

impl Core {
    /// Runs the task-specific worker and verifier lifecycle.
    pub(in crate::lean) fn run_task(
        self: &Arc<Self>,
        conversation: &str,
        task: &mut Task,
        cancel: &AtomicBool,
    ) -> Result<()> {
        task_execution::Execution::new(self, conversation, task, cancel)?.run()
    }
}

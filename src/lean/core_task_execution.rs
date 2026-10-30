use super::*;

mod task_execution;
pub(in crate::lean) mod verifier_setup;

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

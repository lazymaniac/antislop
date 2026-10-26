use super::*;

mod repair;

impl Core {
    /// Resolve in the allocated worktree, then run the independent verifier
    /// again. A semantic choice is the resolver's explicit user question.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::lean) fn repair_integration(
        self: &Arc<Self>,
        conversation: &str,
        task: &mut Task,
        spec: &TaskContract,
        worktree: &Path,
        onto: &str,
        change: &str,
        outcome: Outcome,
        files: &[String],
        cancel: &AtomicBool,
    ) -> Result<()> {
        repair::Repair::new(repair::RepairParts {
            core: self,
            conversation,
            task,
            spec,
            worktree,
            onto,
            change,
            files,
            outcome,
            cancel,
        })
        .run()
    }
}

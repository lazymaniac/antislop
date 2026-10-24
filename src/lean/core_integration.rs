use super::*;

mod flow;

impl Core {
    /// Land an approved writer's change on the project branch.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::lean) fn integrate(
        self: &Arc<Self>,
        conversation: &str,
        task: &mut Task,
        spec: &TaskSpec,
        project: &Path,
        worktree: &Path,
        config: &Config,
        cancel: &AtomicBool,
    ) -> Result<()> {
        let lock = self.integrating.lock().expect("integration mutex");
        if self.defer_agent_work(conversation, task)? {
            return Ok(());
        }
        flow::Integration::new(
            self,
            conversation,
            task,
            spec,
            project,
            worktree,
            config,
            cancel,
        )
        .run(lock)
    }
}

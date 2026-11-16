use crate::lean::*;

mod flow;
mod landing;

impl Core {
    /// Land an approved writer's change on the project branch.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::lean) fn integrate(
        self: &Arc<Self>,
        conversation: &str,
        task: &mut Task,
        spec: &TaskContract,
        project: &Path,
        worktree: &Path,
        config: &Config,
        cancel: &AtomicBool,
    ) -> Result<()> {
        let operation = self.operation(conversation)?;
        operation.bind_task(&task.id);
        let source = self.locks.source(conversation);
        let lock = mail::lock_cancellable(&source, cancel)
            .ok_or_else(|| Error::Invalid("integration cancelled".into()))?;
        require(
            !operation.cancel.load(Ordering::SeqCst) && !cancel.load(Ordering::SeqCst),
            "integration cancelled",
        )?;
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

#[cfg(test)]
mod tests;

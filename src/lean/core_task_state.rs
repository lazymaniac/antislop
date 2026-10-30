use super::*;

impl Core {
    /// The task's state for frontends; `reason` says why it stopped.
    /// `feedback` is the card's bounded result for the announced round:
    /// `null` when that round has produced none yet.
    pub(in crate::lean) fn announce_task(
        &self,
        conversation: &str,
        task: &Task,
        reason: Option<&str>,
    ) {
        let scope_hash = task_scope_hash(task);
        // The event's reason is the card's bounded line; the whole reason
        // is already in the task's report or in the conversation, so a cut
        // here never loses evidence the user or the orchestrator can read.
        let reason = reason.map(|reason| tasks::bound(reason, tasks::DETAIL_LIMIT));
        let _ = self.store.append_event(
            conversation,
            None,
            Some(&task.plan_id),
            "task",
            &json!({"id": task.spec["id"], "task": task.id, "title": task.spec["title"],
                    "status": task.status, "round": task.round, "reason": reason,
                    "scope_hash": scope_hash, "feedback": task.feedback}),
        );
    }

    pub(in crate::lean) fn set_status(
        &self,
        conversation: &str,
        task: &mut Task,
        status: &str,
    ) -> Result<()> {
        self.stop(conversation, task, status, None)
    }

    /// Sets a status the task stops in, saying why (design §8.4).
    pub(in crate::lean) fn stop(
        &self,
        conversation: &str,
        task: &mut Task,
        status: &str,
        reason: Option<&str>,
    ) -> Result<()> {
        task.status = status.into();
        if let Some(reason) = reason {
            let feedback = task
                .feedback
                .get_or_insert_with(|| json!({"round":task.round}));
            feedback["reason"] = json!(reason);
        }
        self.store.update_task(task)?;
        if matches!(
            status,
            status::COORDINATING | status::NEEDS_USER | status::FAILED | status::CANCELLED
        ) {
            self.store.record_task_progress(conversation, task, None)?;
        }
        self.announce_task(conversation, task, reason);
        Ok(())
    }
}

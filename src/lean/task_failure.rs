use super::*;
impl Core {
    /// Ends a task whose agent turn failed or was cancelled, keeping why.
    pub(in crate::lean) fn stop_task(
        &self,
        conversation: &str,
        task: &mut Task,
        role: &str,
        cancel: &AtomicBool,
    ) -> Result<()> {
        let error = self
            .store
            .last_turn(conversation, role)?
            .and_then(|turn| turn.error)
            .unwrap_or_else(|| "no reason recorded".into());
        let next = failed_turn_status(&error, cancel);
        let who = role.split(':').next().unwrap_or(role);
        self.record_failed_turn(conversation, task, next, who, &error)
    }

    fn record_failed_turn(
        &self,
        conversation: &str,
        task: &mut Task,
        next: &str,
        role: &str,
        error: &str,
    ) -> Result<()> {
        append_failed_turn(task, next, role, error);
        self.stop(
            conversation,
            task,
            next,
            Some(&format!("the {role}'s turn ended: {error}")),
        )
    }
}

fn failed_turn_status(error: &str, cancel: &AtomicBool) -> &'static str {
    if cancel.load(Ordering::SeqCst) {
        status::QUEUED
    } else if tasks::needs_user_action(error) {
        status::WAITING
    } else {
        status::FAILED
    }
}

fn append_failed_turn(task: &mut Task, status: &str, role: &str, error: &str) {
    let report = task.report.take().unwrap_or_default();
    task.report = Some(
        format!("{report}\n\n[{status}] the {role}'s turn ended: {error}")
            .trim_start()
            .to_owned(),
    );
}

impl Core {
    pub(in crate::lean) fn defer_agent_work(
        &self,
        conversation: &str,
        task: &mut Task,
    ) -> Result<bool> {
        let pending = [
            format!("worker:{}", task.id),
            format!("verifier:{}", task.id),
        ]
        .iter()
        .map(|role| self.store.pending_mail(conversation, role))
        .collect::<Result<Vec<_>>>()?
        .iter()
        .flatten()
        .any(|mail| mail.kind == "agent_work");
        if pending {
            task.verdict = None;
            task.feedback = None;
            self.set_status(conversation, task, status::QUEUED)?;
        }
        Ok(pending)
    }
}

impl Core {
    pub(in crate::lean) fn wait_for_question(
        &self,
        conversation: &str,
        task: &mut Task,
        request: &str,
        verifier: bool,
    ) -> Result<()> {
        task.verdict = None;
        let feedback = task
            .feedback
            .get_or_insert_with(|| json!({"round":task.round}));
        feedback["waiting_request"] = json!(request);
        feedback["resume_role"] = json!(if verifier { "verifier" } else { "worker" });
        feedback["reason"] = json!("awaiting orchestrator or user answer");
        self.store.update_task(task)?;
        task.status = self.store.park_task_for_question(task, request)?;
        self.announce_task(
            conversation,
            task,
            Some("awaiting orchestrator or user answer"),
        );
        Ok(())
    }
}

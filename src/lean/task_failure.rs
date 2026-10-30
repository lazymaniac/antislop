use super::*;
impl Core {
    /// Records a scheduled task execution failure with its raw cause.
    pub(super) fn fail_scheduled_task(&self, conversation: &str, task: &mut Task, error: &Error) {
        append_report_entry(task, "runtime", &error.to_string());
        let _ = self.stop(conversation, task, status::FAILED, Some(&error.to_string()));
    }

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
    append_report_entry(task, status, &format!("the {role}'s turn ended: {error}"));
}

pub(in crate::lean) fn append_report_entry(task: &mut Task, category: &str, text: &str) {
    append_report(task, category, text, true);
}

/// Appends an integration note while retaining its established blank prefix
/// when the task has no earlier report.
pub(in crate::lean) fn append_integration_report_entry(
    task: &mut Task,
    category: &str,
    text: &str,
) {
    append_report(task, category, text, false);
}

fn append_report(task: &mut Task, category: &str, text: &str, trim_leading: bool) {
    let report = task.report.take().unwrap_or_default();
    let entry = format!("{report}\n\n[{category}] {text}");
    task.report = Some(if trim_leading {
        entry.trim_start().to_owned()
    } else {
        entry
    });
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_entries_keep_prior_evidence_and_raw_failure_text() {
        let mut task = Task {
            input_generation: 0,
            id: "task".into(),
            plan_id: "plan".into(),
            spec: json!({"id":"task"}),
            status: status::FAILED.into(),
            round: 0,
            report: Some("worker report".into()),
            verdict: None,
            base: None,
            result: None,
            feedback: None,
            freshness_token: None,
            checks: None,
        };

        append_report_entry(&mut task, "runtime", "CLI exited with status 7");

        assert_eq!(
            task.report.as_deref(),
            Some("worker report\n\n[runtime] CLI exited with status 7")
        );
    }

    #[test]
    fn integration_report_entry_uses_the_shared_separator() {
        let mut task = Task {
            input_generation: 0,
            id: "task".into(),
            plan_id: "plan".into(),
            spec: json!({"id":"task"}),
            status: status::APPROVED.into(),
            round: 0,
            report: None,
            verdict: None,
            base: None,
            result: None,
            feedback: None,
            freshness_token: None,
            checks: None,
        };

        append_integration_report_entry(&mut task, "integration", "committed as abc123");

        assert_eq!(
            task.report.as_deref(),
            Some("\n\n[integration] committed as abc123")
        );
    }
}

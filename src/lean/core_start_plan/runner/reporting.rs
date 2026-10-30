use super::super::super::core_prompt_helpers::{blocked_report, stopped_report};
use super::*;

impl Core {
    pub(super) fn finish_blocked_plan(self: &Arc<Self>, conversation: &str, tasks: &[Task]) {
        let autonomous = self.autonomous(conversation).unwrap_or(false);
        let mut plans = self.plans.lock().expect("plans mutex");
        let report = blocked_report(tasks, autonomous);
        let told = self
            .blocked_seen
            .lock()
            .expect("blocked mutex")
            .insert(conversation.to_owned(), report.clone());
        if told.as_deref() != Some(report.as_str())
            && self
                .store
                .add_message(conversation, RUNTIME, &report)
                .is_ok()
        {
            self.wake(conversation);
        }
        plans.remove(conversation);
    }

    /// Lets the orchestrator address stopped tasks while other tasks continue.
    pub(in crate::lean) fn report_stopped_tasks(
        self: &Arc<Self>,
        conversation: &str,
        plan_id: &str,
    ) {
        let tasks = self.store.plan(plan_id).unwrap_or_default();
        let Some(report) = stopped_report(&tasks) else {
            return;
        };
        let told = self
            .blocked_seen
            .lock()
            .expect("blocked mutex")
            .insert(conversation.to_owned(), report.clone());
        if told.as_deref() != Some(report.as_str())
            && self
                .store
                .add_message(conversation, RUNTIME, &report)
                .is_ok()
        {
            self.wake(conversation);
        }
    }
}

pub(super) fn has_blocked_tasks(tasks: &[Task]) -> bool {
    tasks.iter().any(|task| task.status != status::APPROVED)
}

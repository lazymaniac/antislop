use super::*;
use crate::lean::core::orchestration::prompt_helpers::{blocked_report, stopped_report};

impl Core {
    pub(super) fn finish_blocked_plan(
        self: &Arc<Self>,
        conversation: &str,
        tasks: &[Task],
        cancel: &AtomicBool,
    ) {
        let mut plans = self.activity.plans.lock().expect("plans mutex");
        if cancel.load(Ordering::SeqCst) {
            plans.remove(conversation);
            return;
        }
        let report = blocked_report(tasks);
        let told = self
            .activity
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
            .activity
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
    tasks
        .iter()
        .any(|task| task.status != status::APPROVED || task.spec["deferred"] == true)
}

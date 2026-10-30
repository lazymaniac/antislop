use super::*;

#[path = "update_delivery.rs"]
mod delivery;
#[path = "update_details.rs"]
mod details;
#[path = "update_events.rs"]
mod events;
#[path = "update_records.rs"]
mod records;

impl App {
    pub(in crate::lean::tui) fn apply_update_inner(&mut self, update: Update) {
        match update {
            update @ Update::NativeInputRejected { .. } => self.apply_native_input_rejected(update),
            update @ Update::PlanHistory { .. } => self.apply_plan_history(update),
            update @ Update::AgentMessage { .. } => self.apply_agent_message(update),
            update @ Update::Ledger { .. } => self.apply_ledger(update),
            update @ Update::Config { .. } => self.apply_config(update),
            update @ Update::Events { .. } => self.apply_events(update),
            update @ Update::Conversation { .. } => self.apply_conversation(update),
            update @ Update::ProjectReady { .. } => self.apply_project_ready(update),
            update @ Update::ProjectTrust { .. } => self.apply_project_trust(update),
            update @ Update::ProjectStatus { .. } => self.apply_project_status(update),
            update @ Update::Notice { .. } => self.apply_notice(update),
            update @ Update::TaskFailed { .. } => self.apply_task_failed(update),
            update @ Update::Task { .. } => self.apply_task(update),
            update @ Update::Diff { .. } => self.apply_diff(update),
            update @ Update::InterviewResult { .. } => self.apply_interview_result(update),
            update @ Update::SendResult { .. } => self.apply_send_result(update),
            update @ Update::CommentResult { .. } => self.apply_comment_result(update),
        }
    }
}

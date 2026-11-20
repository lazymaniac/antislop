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
            Update::SessionAction {
                conversation,
                action,
                result,
            } => {
                if conversation == self.conversation {
                    self.view.set_session_action_result(action, result);
                }
            }
            Update::ResultDocument {
                conversation,
                generation,
                key,
                selection,
                id,
                result,
            } => {
                if conversation == self.conversation
                    && generation == self.generation
                    && let Some(details) = self.details.as_mut().filter(|details| {
                        details.key == key && details.result_metadata.contains_key(&id)
                    })
                {
                    if details.result_loading.as_ref().is_some_and(
                        |(loading_id, loading_selection, loading_generation)| {
                            loading_id == &id
                                && *loading_selection == selection
                                && *loading_generation == generation
                        },
                    ) {
                        details.result_loading = None;
                    }
                    match result {
                        Ok(document) => {
                            details.documents.insert(id, document);
                        }
                        Err(error)
                            if details.result_selection_generation == selection
                                && details.selected_result_document.as_deref() == Some(&id) =>
                        {
                            details.result_error = Some((id, error));
                        }
                        Err(_) => {}
                    }
                }
            }
            update @ Update::NativeInputRejected { .. } => self.apply_native_input_rejected(update),
            update @ Update::PlanHistory { .. } => self.apply_plan_history(update),
            update @ Update::AgentMessage { .. } => self.apply_agent_message(update),
            update @ Update::Config { .. } => self.apply_config(update),
            update @ Update::Events { .. } => self.apply_events(update),
            update @ Update::Conversation { .. } => self.apply_conversation(update),
            update @ Update::ConversationCreated { .. } => self.apply_conversation_created(update),
            update @ Update::ProjectReady { .. } => self.apply_project_ready(update),
            update @ Update::ProjectStatus { .. } => self.apply_project_status(update),
            update @ Update::Notice { .. } => self.apply_notice(update),
            update @ Update::TaskFailed { .. } => self.apply_task_failed(update),
            update @ Update::Task { .. } => self.apply_task(update),
            update @ Update::Diff { .. } => self.apply_diff(update),
            update @ Update::InterviewResult { .. } => self.apply_interview_result(update),
            update @ Update::SendResult { .. } => self.apply_send_result(update),
            update @ Update::CommentResult { .. } => self.apply_comment_result(update),
            update @ Update::PlanRevisionResult { .. } => self.apply_plan_revision_result(update),
        }
    }
}

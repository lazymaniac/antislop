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
            Update::Conversations {
                state_dir,
                request_id,
                result,
            } => self.apply_conversations(state_dir, request_id, result),
            Update::ConversationDeleted {
                project,
                id,
                result,
            } => self.apply_conversation_deleted(project, id, result),
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
            Update::WorkbenchSessions {
                conversation,
                generation,
                request_id,
                result,
            } => {
                if self.workbench_sessions_inflight.as_ref()
                    == Some(&(conversation.clone(), generation, request_id))
                {
                    self.workbench_sessions_inflight = None;
                }
                if conversation == self.conversation
                    && generation == self.generation
                    && request_id == self.workbench_sessions_request_id
                    && !self.workbench_sessions_pending
                {
                    match result {
                        Ok(sessions) => {
                            self.workbench_sessions_after = self.workbench_sessions_fetch_after;
                            self.workbench_sessions = sessions.clone();
                            if let Some(details) = &mut self.details {
                                details.sessions = sessions;
                            }
                        }
                        Err(error) => {
                            self.notice =
                                Some(format!("Could not load CLI session owners: {error}"))
                        }
                    }
                }
                if self.workbench_sessions_inflight.is_none() && self.workbench_sessions_pending {
                    self.fetch_workbench_sessions();
                }
            }
            update @ Update::PlanHistory { .. } => self.apply_plan_history(update),
            update @ Update::AgentMessage { .. } => self.apply_agent_message(update),
            update @ Update::Config { .. } => self.apply_config(update),
            update @ Update::Events { .. } => self.apply_events(update),
            update @ Update::Conversation { .. } => self.apply_conversation(update),
            update @ Update::ConversationCreated { .. } => self.apply_conversation_created(update),
            update @ Update::ProjectReady { .. } => self.apply_project_ready(update),
            update @ Update::ProjectStatus { .. } => self.apply_project_status(update),
            update @ Update::Notice { .. } => self.apply_notice(update),
            Update::RecoveryConsent {
                conversation,
                generation,
                plan,
                task: _,
                result,
            } => {
                if conversation == self.conversation
                    && generation == self.generation
                    && self.view.plan.as_deref() == Some(&plan)
                {
                    match result {
                        Ok(consent) => self.recovery_consent = Some(consent),
                        Err(error) => self.notice = Some(format!("✗ {error}")),
                    }
                }
            }
            update @ Update::TaskFailed { .. } => self.apply_task_failed(update),
            update @ Update::Task { .. } => self.apply_task(update),
            update @ Update::Diff { .. } => self.apply_diff(update),
            update @ Update::InterviewResult { .. } => self.apply_interview_result(update),
            update @ Update::ApprovalResult { .. } => self.apply_approval_result(update),
            update @ Update::SendResult { .. } => self.apply_send_result(update),
            update @ Update::CommentResult { .. } => self.apply_comment_result(update),
            update @ Update::PlanRevisionResult { .. } => self.apply_plan_revision_result(update),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(session_id: &str) -> crate::lean::store::SessionInfo {
        crate::lean::store::SessionInfo {
            turn_id: session_id.into(),
            plan_id: None,
            role: "orchestrator".into(),
            provider: "codex".into(),
            session_id: Some(session_id.into()),
            status: "running".into(),
            error: None,
            upto_message: 0,
            started_ms: 0,
            finished_ms: None,
            pid: None,
        }
    }

    #[test]
    fn new_turn_invalidates_owner_cache_and_reissues_after_older_inflight_read() {
        let mut app = App::test_app();
        app.navigation.screen = Screen::Workbench;
        app.navigation.target = WorkbenchTarget::Session;
        app.details = Some(Details::new("conversation".into()));
        app.workbench_sessions = vec![session("old")];
        app.workbench_sessions_request_id = 1;
        app.workbench_sessions_inflight = Some((app.conversation.clone(), app.generation, 1));
        app.apply_update(Update::Events { conversation: app.conversation.clone(), generation: app.generation,
            events: vec![serde_json::json!({"seq":1,"kind":"turn_started","payload":{"role":"orchestrator","provider":"codex"}})], idle:false, observer:false });
        assert!(app.workbench_sessions.is_empty());
        assert!(app.workbench_sessions_pending);
        assert_eq!(app.workbench_sessions_request_id, 1);
        for _ in 0..2 {
            app.key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char('p'),
                crossterm::event::KeyModifiers::NONE,
            ));
            assert!(app.workbench_sessions_pending);
            assert_eq!(app.workbench_sessions_request_id, 1);
        }
        app.apply_update(Update::WorkbenchSessions {
            conversation: app.conversation.clone(),
            generation: app.generation,
            request_id: 1,
            result: Ok(vec![session("stale")]),
        });
        assert!(
            app.workbench_sessions.is_empty(),
            "old snapshot must not restore a retired owner"
        );
        assert_eq!(app.workbench_sessions_request_id, 2);
        assert!(!app.workbench_sessions_pending);
        assert!(app.details.as_ref().unwrap().sessions.is_empty());
        assert_eq!(app.workbench_sessions_after, 1);
        assert_eq!(app.workbench_sessions_fetch_after, 1);
        app.apply_update(Update::WorkbenchSessions {
            conversation: app.conversation.clone(),
            generation: app.generation,
            request_id: 2,
            result: Ok(vec![session("fresh")]),
        });
        assert_eq!(
            app.workbench_sessions[0].session_id.as_deref(),
            Some("fresh")
        );
        assert_eq!(
            app.details.as_ref().unwrap().sessions[0]
                .session_id
                .as_deref(),
            Some("fresh")
        );
        assert!(app.workbench_sessions_inflight.is_none());
    }

    #[test]
    fn stale_workbench_sessions_reply_cannot_replace_latest_owner_identity() {
        let mut app = App::test_app();
        app.workbench_sessions_request_id = 2;
        app.workbench_sessions = vec![session("latest")];

        app.apply_update(Update::WorkbenchSessions {
            conversation: app.conversation.clone(),
            generation: app.generation,
            request_id: 1,
            result: Ok(vec![session("stale")]),
        });

        assert_eq!(
            app.workbench_sessions[0].session_id.as_deref(),
            Some("latest")
        );
    }

    #[test]
    fn latest_workbench_sessions_reply_updates_owner_identity() {
        let mut app = App::test_app();
        app.workbench_sessions_request_id = 2;

        app.apply_update(Update::WorkbenchSessions {
            conversation: app.conversation.clone(),
            generation: app.generation,
            request_id: 2,
            result: Ok(vec![session("current")]),
        });

        assert_eq!(
            app.workbench_sessions[0].session_id.as_deref(),
            Some("current")
        );
    }
}

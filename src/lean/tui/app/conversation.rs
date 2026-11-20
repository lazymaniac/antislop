use super::super::*;
mod poller;
use poller::Poller;

impl App {
    pub fn help_max_scroll(&self) -> usize {
        if let Some(help) = self.help {
            help.max_scroll(self.terminal_area)
        } else {
            0
        }
    }

    pub fn open(&mut self, conversation: String) {
        if let Some(project) = self.projects.get(self.active_project) {
            self.conversation_projects
                .insert(conversation.clone(), project.path.clone());
        }
        self.result_fetch.cancel_document_fetch();
        if let Some(details) = self.details.as_mut() {
            details.result_loading = None;
        }
        self.save_active_conversation();
        self.activate_conversation(conversation.clone());
        self.restore_conversation(&conversation);
        self.schedule_output_document_fetch();
        self.reset_history_recall();
        self.start_poller();
    }

    fn save_active_conversation(&mut self) {
        if self.conversation.is_empty() {
            return;
        }
        if let Some(project) = self.projects.get(self.active_project) {
            self.conversation_projects
                .insert(self.conversation.clone(), project.path.clone());
        }
        self.conversation_drafts
            .insert(self.conversation.clone(), self.input.text.clone());
        self.conversation_states
            .insert(self.conversation.clone(), ConversationState::capture(self));
    }

    fn activate_conversation(&mut self, conversation: String) {
        self.stop_poller.store(true, Ordering::SeqCst);
        self.generation += 1;
        self.decision_inbox = None;
        self.conversation = conversation;
        self.view = View::default();
        self.plan_history = None;
    }

    fn restore_conversation(&mut self, conversation: &str) {
        if let Some(state) = self.conversation_states.get(conversation).cloned() {
            state.restore(self);
        } else {
            self.restore_conversation_draft(conversation);
            self.initialize_new_conversation();
        }
    }

    fn restore_conversation_draft(&mut self, conversation: &str) {
        let draft = self
            .conversation_drafts
            .get(conversation)
            .cloned()
            .unwrap_or_default();
        self.input = InputBuffer::with_text(draft);
    }

    fn initialize_new_conversation(&mut self) {
        self.clear_conversation_overlays();
        self.reset_conversation_navigation();
        self.reset_conversation_requests();
        self.reset_conversation_delivery();
    }

    fn clear_conversation_overlays(&mut self) {
        self.details = None;
        self.diff = None;
        self.plan_view = None;
        self.editor = None;
        self.flow_editor = None;
        self.graph_link_source = None;
    }

    fn reset_conversation_navigation(&mut self) {
        self.selected = ORCHESTRATOR.into();
        self.relations = Relations::default();
        self.offset = (0, 0);
        self.scroll = 0;
        self.navigation = NavigationState::default();
        self.set_focus(Focus::Chat);
        self.graph_scope = Scope::default();
        self.graph_visibility_exception = None;
        self.graph_revealed = None;
        self.graph_collapsed.clear();
        self.graph_grouping = graph::GroupingMode::default();
        self.graph_viewport = None;
    }

    fn reset_conversation_requests(&mut self) {
        self.task_request_id = 0;
        self.diff_request_id = 0;
    }

    fn reset_conversation_delivery(&mut self) {
        self.sending = false;
        self.pending_send = None;
        self.send_request_id = 0;
        self.failed_send = None;
        self.sending_comment = false;
        self.pending_comment = None;
        self.comment_request_id = 0;
        self.failed_comment = None;
    }

    fn reset_history_recall(&mut self) {
        self.input_shown = 0;
        self.picker = None;
        self.help = None;
        self.help_scroll = 0;
        self.history_index = None;
        self.history_draft.clear();
        self.history_draft_cursor = 0;
        self.history_draft_preferred_col = None;
        self.history_item_cursors.clear();
    }

    fn start_poller(&mut self) {
        let stop = Arc::new(AtomicBool::new(false));
        self.stop_poller = stop.clone();
        Poller {
            client: self.client.clone(),
            updates: self.updates.clone(),
            conversation: self.conversation.clone(),
            generation: self.generation,
            stop,
        }
        .spawn();
    }

    pub(in crate::lean::tui) fn new_conversation(&mut self) {
        let Some(request_id) = self.new_conversation_request_id.checked_add(1) else {
            self.notice = Some("✗ too many session creation requests in this window".into());
            return;
        };
        self.new_conversation_request_id = request_id;
        let generation = self.generation;
        let client = self.client.clone();
        let state_dir = client.state_dir().to_path_buf();
        let project = self
            .projects
            .get(self.active_project)
            .map(|conn| conn.path.clone())
            .unwrap_or_default();
        let updates = self.updates.clone();
        std::thread::spawn(move || {
            let request = if project.as_os_str().is_empty() {
                crate::lean::daemon::ControlRequest::Start
            } else {
                crate::lean::daemon::ControlRequest::StartIn {
                    project: project.to_string_lossy().into_owned(),
                }
            };
            let result = client
                .call(&Request::Control(request))
                .map_err(|error| error.to_string())
                .and_then(|conversation| {
                    conversation["id"]
                        .as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| "daemon returned a session without an id".to_owned())
                });
            let update = Update::ConversationCreated {
                project,
                state_dir,
                request_id,
                generation,
                result,
            };
            let _ = updates.send(update);
        });
    }
}

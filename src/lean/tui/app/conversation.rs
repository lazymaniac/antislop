use super::super::*;
mod poller;
use poller::Poller;

impl App {
    pub fn require_trust(&mut self, project_path: impl Into<String>) {
        self.trust_prompt = Some(project_path.into());
    }

    pub fn accept_trust(&mut self) {
        if self.trust_prompt.take().is_some() {
            self.request(
                Request::Trust { trusted: true },
                Some("workspace directory trusted"),
            );
            self.notice = Some("workspace directory trusted".into());
        }
    }

    pub fn help_max_scroll(&self) -> usize {
        if let Some(help) = self.help {
            help.max_scroll(self.terminal_area)
        } else {
            0
        }
    }

    pub fn open(&mut self, conversation: String) {
        self.save_active_conversation();
        self.activate_conversation(conversation.clone());
        self.restore_conversation(&conversation);
        self.reset_history_recall();
        self.start_poller();
    }

    fn save_active_conversation(&mut self) {
        if self.conversation.is_empty() {
            return;
        }
        self.conversation_drafts
            .insert(self.conversation.clone(), self.input.text.clone());
        self.conversation_states
            .insert(self.conversation.clone(), ConversationState::capture(self));
    }

    fn activate_conversation(&mut self, conversation: String) {
        self.stop_poller.store(true, Ordering::SeqCst);
        self.generation += 1;
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
        self.ledger_view = None;
        self.editor = None;
        self.confirmation = None;
    }

    fn reset_conversation_navigation(&mut self) {
        self.selected = ORCHESTRATOR.into();
        self.filter = Filter::All;
        self.offset = (0, 0);
        self.scroll = 0;
        self.focus = Focus::Chat;
        self.graph_filter = None;
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

    pub(in crate::lean::tui) fn new_conversation(&self) {
        let client = self.client.clone();
        let updates = self.updates.clone();
        std::thread::spawn(move || {
            let update = match client.call(&Request::Start) {
                Ok(conversation) => {
                    Update::Conversation(conversation["id"].as_str().unwrap_or_default().into())
                }
                Err(error) => Update::Notice(format!("✗ {error}")),
            };
            let _ = updates.send(update);
        });
    }
}

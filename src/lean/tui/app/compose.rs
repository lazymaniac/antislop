use super::super::*;
use crate::lean::RecoveryAction;
mod send;
use send::{AgentMessageSend, AgentMessageTarget, CommentSend, comment_text, is_rejection_target};

impl App {
    pub fn submit(&mut self) {
        if self.reject_duplicate_send() || self.input.text.trim().is_empty() {
            return;
        }
        if self.submit_command() {
            return;
        }
        self.begin_chat_send();
    }

    fn reject_duplicate_send(&mut self) -> bool {
        if !self.sending {
            return false;
        }
        self.notice = Some("Sending in progress; draft preserved.".into());
        true
    }

    fn submit_command(&mut self) -> bool {
        if !self.input.text.trim().starts_with('/') {
            return false;
        }
        let command = std::mem::take(&mut self.input.text);
        self.reset_input_cursor();
        self.dispatch(command.trim());
        true
    }

    fn reset_input_cursor(&mut self) {
        self.input.cursor = 0;
        self.input.preferred_col = None;
        self.input_shown = 0;
    }

    fn begin_chat_send(&mut self) {
        self.send_request_id += 1;
        let request_id = self.send_request_id;
        let text = self.input.text.clone();
        self.record_history(&text);
        let sent_text = std::mem::take(&mut self.input.text);
        self.reset_input_cursor();
        self.scroll = 0;
        self.sending = true;
        self.pending_send = Some(sent_text.clone());
        self.failed_send = None;
        self.launch_chat_send(request_id, sent_text);
    }

    fn launch_chat_send(&self, request_id: u64, sent_text: String) {
        let client = self.client.clone();
        let updates = self.updates.clone();
        let conversation = self.conversation.clone();
        let generation = self.generation;
        std::thread::spawn(move || {
            let result = client
                .call(&Request::Conversation(
                    crate::lean::daemon::ConversationRequest::Send {
                        conversation: conversation.clone(),
                        text: sent_text.clone(),
                    },
                ))
                .map(|_| ())
                .map_err(|error| error.to_string());
            let _ = updates.send(Update::SendResult {
                conversation,
                generation,
                request_id,
                sent_text,
                result,
            });
        });
    }

    pub(in crate::lean::tui) fn submit_agent_message(&mut self) {
        let Some(target) = self.agent_message_target() else {
            return;
        };
        let Some(send) = self.prepare_agent_message(target) else {
            return;
        };
        send.spawn();
    }

    fn agent_message_target(&self) -> Option<AgentMessageTarget> {
        let details = self
            .details
            .as_ref()
            .filter(|details| details.selected_round.is_none())?;
        let (conversation, recipient) = details.composer_key();
        let task = details.task.as_ref();
        let addressed = if let Some((role, _)) = recipient.split_once(':') {
            let uuid = task.and_then(|task| task["id"].as_str())?;
            format!("{role}:{uuid}")
        } else {
            recipient.clone()
        };
        Some(AgentMessageTarget {
            conversation,
            recipient,
            addressed,
            plan: self.view.plan.clone(),
            expected_run: task
                .and_then(|task| task["_latest_run"].as_str())
                .map(str::to_owned),
            expected_round: task
                .and_then(|task| task["round"].as_u64())
                .map(|round| round as u32),
        })
    }

    fn prepare_agent_message(&mut self, target: AgentMessageTarget) -> Option<AgentMessageSend> {
        let mut drafts = self.agent_drafts.borrow_mut();
        let composer = drafts
            .entry((target.conversation.clone(), target.recipient.clone()))
            .or_default();
        if composer.sending || composer.input.text.trim().is_empty() {
            return None;
        }
        let text = composer.input.text.clone();
        if composer.sent_text.as_ref() != Some(&text) {
            composer.operation = None;
        }
        composer.sent_text = Some(text.clone());
        let operation = composer
            .operation
            .get_or_insert_with(|| uuid::Uuid::new_v4().to_string())
            .clone();
        composer.sending = true;
        composer.delivery_receipts.clear();
        composer.error = None;
        composer.address = Some(target.addressed.clone());
        Some(AgentMessageSend {
            client: self.client.clone(),
            updates: self.updates.clone(),
            target,
            operation,
            text,
        })
    }

    pub fn submit_comment(&mut self) {
        if self.reject_duplicate_comment() {
            return;
        }
        let Some(editor) = &self.editor else {
            return;
        };
        let rejecting = is_rejection_target(&editor.target);
        if editor.input.text.trim().is_empty() && !rejecting {
            return;
        }
        self.begin_comment_send(rejecting);
    }

    fn reject_duplicate_comment(&mut self) -> bool {
        if !self.sending_comment {
            return false;
        }
        self.notice = Some("Sending comment in progress; draft preserved.".into());
        true
    }

    fn begin_comment_send(&mut self, rejecting: bool) {
        self.comment_request_id += 1;
        let request_id = self.comment_request_id;
        let editor = self.editor.take().expect("editing");
        if rejecting {
            self.plan_view = None;
        }
        self.sending_comment = true;
        self.pending_comment = Some(editor.clone());
        self.failed_comment = None;
        self.prepare_comment_send(request_id, editor);
    }

    fn prepare_comment_send(&self, request_id: u64, editor: Editor) {
        let (round, run) = self.comment_context();
        let text = comment_text(&editor, &self.view);
        CommentSend {
            client: self.client.clone(),
            updates: self.updates.clone(),
            conversation: self.conversation.clone(),
            plan: self.view.plan.clone(),
            generation: self.generation,
            request_id,
            editor,
            round,
            run,
            text,
        }
        .spawn();
    }

    fn comment_context(&self) -> (Option<u32>, Option<String>) {
        let round = self
            .details
            .as_ref()
            .filter(|details| details.task_id().is_some())
            .and_then(|details| {
                details.selected_round.or_else(|| {
                    details
                        .task
                        .as_ref()
                        .and_then(|task| task["round"].as_u64())
                })
            })
            .map(|round| round as u32);
        let run = self
            .details
            .as_ref()
            .filter(|details| details.task_id().is_none())
            .and_then(|details| details.task.as_ref())
            .and_then(|task| task["_run"].as_str())
            .map(str::to_owned);
        (round, run)
    }

    pub(in crate::lean::tui) fn return_to_main(&mut self) {
        self.hide_ledger();
        self.remember_details();
        self.close_overlays();
        self.preserve_comment_editor();
        self.hide_config_editor();
    }

    fn hide_ledger(&mut self) {
        if let Some(ledger) = &mut self.ledger_view {
            ledger.visible = false;
        }
    }

    fn remember_details(&mut self) {
        if let Some(details) = self.details.take() {
            self.details_scroll_memory
                .insert((details.key.clone(), details.tab), details.scroll);
            self.details_query_memory
                .insert((details.key, details.tab), details.query);
        }
    }

    fn close_overlays(&mut self) {
        self.diff = None;
        self.plan_view = None;
        self.plan_history = None;
        self.help = None;
        self.picker = None;
        self.confirmation = None;
        self.unblock = None;
    }

    fn preserve_comment_editor(&mut self) {
        let Some(editor) = self.editor.take() else {
            return;
        };
        let key = Self::comment_key(
            &self.conversation,
            self.view.plan.as_deref(),
            &editor.target,
        );
        self.comment_draft_cursors.insert(
            key.clone(),
            (editor.input.cursor, editor.input.preferred_col),
        );
        self.comment_drafts.insert(key, editor.input.text);
    }

    fn hide_config_editor(&mut self) {
        if let Some(editor) = &mut self.config_editor {
            editor.visible = false;
        }
    }

    pub fn apply_update(&mut self, update: Update) {
        self.apply_update_inner(update);
        if let Some(view) = self.ledger_view.as_mut()
            && !view.pending
            && let Some(run) = view.review_run.as_ref()
            && self.view.requirements_approval_run.as_deref() == Some(run)
        {
            let run = view.review_run.take().expect("queued review");
            self.return_to_main();
            self.ledger_view.as_mut().unwrap().visible = true;
            self.ledger_action(ledger::Action::Review(run));
        }
        self.text_selection
            .invalidate_context(&self.selection_context());
    }
}

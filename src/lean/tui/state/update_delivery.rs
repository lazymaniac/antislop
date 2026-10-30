use super::super::*;

impl App {
    pub(super) fn apply_native_input_rejected(&mut self, update: Update) {
        match update {
            Update::NativeInputRejected {
                conversation,
                role,
                input,
                error,
            } => {
                self.recover_native_input(&conversation, &role, input, &error);
            }
            _ => unreachable!("update routed to wrong handler"),
        }
    }

    pub(super) fn apply_agent_message(&mut self, update: Update) {
        let Update::AgentMessage {
            conversation,
            recipient,
            operation,
            text,
            result,
        } = update
        else {
            unreachable!("update routed to wrong handler");
        };
        let receipt_view = (conversation == self.conversation).then_some(&self.view);
        let mut drafts = self.agent_drafts.borrow_mut();
        let composer = drafts.entry((conversation, recipient)).or_default();
        if composer.operation.as_ref() != Some(&operation) {
            return;
        }
        composer.sending = false;
        match result {
            Ok(receipt) => apply_agent_receipt(composer, &text, receipt, receipt_view),
            Err(error) => composer.error = Some(error),
        }
    }

    pub(super) fn apply_interview_result(&mut self, update: Update) {
        match update {
            Update::InterviewResult {
                conversation,
                question,
                result,
            } => {
                if let Some(form) = self
                    .interview_forms
                    .get_mut(&conversation)
                    .filter(|form| form.event == question)
                {
                    form.sending = false;
                    match result {
                        Ok(()) => {
                            form.sent = true;
                            form.error = None;
                        }
                        Err(error) => {
                            form.error = Some(error);
                            form.visible = true;
                        }
                    }
                }
            }
            _ => unreachable!("update routed to wrong handler"),
        }
    }

    pub(super) fn apply_send_result(&mut self, update: Update) {
        let Update::SendResult {
            conversation,
            request_id,
            sent_text,
            result,
            ..
        } = update
        else {
            unreachable!("update routed to wrong handler");
        };
        if conversation == self.conversation && request_id == self.send_request_id {
            self.finish_active_send(sent_text, result);
        } else {
            self.finish_background_send(&conversation, request_id, sent_text, result);
        }
    }

    fn finish_active_send(&mut self, sent_text: String, result: std::result::Result<(), String>) {
        self.sending = false;
        self.pending_send = None;
        match result {
            Ok(()) => self.failed_send = None,
            Err(error) => self.restore_active_send(sent_text, error),
        }
    }

    fn restore_active_send(&mut self, sent_text: String, error: String) {
        if self.input.is_empty() {
            self.input = InputBuffer::with_text(sent_text);
            self.notice = Some(format!("✗ {error} (draft restored)"));
        } else {
            self.failed_send = Some(sent_text);
            self.notice = Some(format!("✗ {error} (unsent message preserved)"));
        }
    }

    fn finish_background_send(
        &mut self,
        conversation: &str,
        request_id: u64,
        sent_text: String,
        result: std::result::Result<(), String>,
    ) {
        let Some(state) = self
            .conversation_states
            .get_mut(conversation)
            .filter(|state| state.send_request_id == request_id)
        else {
            return;
        };
        state.sending = false;
        state.pending_send = None;
        match result {
            Ok(()) => state.failed_send = None,
            Err(_) if state.input.is_empty() => state.input = InputBuffer::with_text(sent_text),
            Err(_) => state.failed_send = Some(sent_text),
        }
    }

    pub(super) fn apply_comment_result(&mut self, update: Update) {
        let Update::CommentResult {
            conversation,
            plan,
            request_id,
            editor,
            result,
            ..
        } = update
        else {
            unreachable!("update routed to wrong handler");
        };
        if conversation == self.conversation && request_id == self.comment_request_id {
            self.finish_active_comment(editor, result);
        } else {
            self.finish_background_comment(
                &conversation,
                plan.as_deref(),
                request_id,
                editor,
                result,
            );
        }
    }

    fn finish_active_comment(&mut self, editor: Editor, result: std::result::Result<(), String>) {
        self.sending_comment = false;
        self.pending_comment = None;
        match result {
            Ok(()) => {
                self.failed_comment = None;
                self.remove_comment_draft(&editor.target);
            }
            Err(error) => {
                self.restore_active_comment(editor);
                self.notice = Some(format!("✗ {error} (comment draft preserved)"));
            }
        }
    }

    fn restore_active_comment(&mut self, editor: Editor) {
        if let Some(active) = &mut self.editor {
            if active.target == editor.target && active.input.is_empty() {
                active.input = editor.input;
            } else {
                self.save_comment_draft(&editor.target, editor.input.clone());
                self.failed_comment = Some(editor);
            }
        } else {
            self.editor = Some(editor);
        }
    }

    fn finish_background_comment(
        &mut self,
        conversation: &str,
        plan: Option<&str>,
        request_id: u64,
        editor: Editor,
        result: std::result::Result<(), String>,
    ) {
        let Some(state) = self
            .conversation_states
            .get_mut(conversation)
            .filter(|state| state.comment_request_id == request_id)
        else {
            return;
        };
        state.sending_comment = false;
        state.pending_comment = None;
        match result {
            Ok(()) => clear_comment_failure(state, conversation, plan, &editor.target),
            Err(_) => preserve_background_comment(state, conversation, plan, editor),
        }
    }
}

fn clear_comment_failure(
    state: &mut ConversationState,
    conversation: &str,
    plan: Option<&str>,
    target: &str,
) {
    state.failed_comment = None;
    let key = App::comment_key(conversation, plan, target);
    state.comment_drafts.remove(&key);
    state.comment_draft_cursors.remove(&key);
}

fn preserve_background_comment(
    state: &mut ConversationState,
    conversation: &str,
    plan: Option<&str>,
    editor: Editor,
) {
    if state.editor.is_none() {
        state.editor = Some(editor);
        return;
    }
    let key = App::comment_key(conversation, plan, &editor.target);
    state
        .comment_drafts
        .insert(key.clone(), editor.input.text.clone());
    state
        .comment_draft_cursors
        .insert(key, (editor.input.cursor, editor.input.preferred_col));
    state.failed_comment = Some(editor);
}

fn apply_agent_receipt(
    composer: &mut details::Composer,
    text: &str,
    receipt: Value,
    view: Option<&View>,
) {
    composer.mail = receipt["mail"].as_u64();
    composer.delivery = Some(receipt["state"].as_str().unwrap_or("queued").to_owned());
    if composer
        .mail
        .is_some_and(|id| composer.delivery_receipts.contains(&id))
    {
        composer.delivery = Some("delivered".into());
    }
    if view.is_some_and(|view| receipt_is_delivered(composer, view)) {
        composer.delivery = Some("delivered".into());
    }
    if composer.input.text == text {
        composer.input.clear();
    }
    composer.operation = None;
    composer.error = pending_delivery_error(composer);
    composer.delivery_receipts.clear();
}

fn receipt_is_delivered(composer: &details::Composer, view: &View) -> bool {
    view.messages.iter().any(|message| {
        composer.mail.is_some_and(|id| message.id == id.to_string()) && message.state == "delivered"
    })
}

fn pending_delivery_error(composer: &details::Composer) -> Option<String> {
    composer
        .delivery_failure
        .as_ref()
        .filter(|(ids, _)| {
            composer.mail.is_some_and(|mail| ids.contains(&mail))
                && composer.delivery.as_deref() != Some("delivered")
        })
        .map(|(_, error)| error.clone())
}

use super::super::*;

impl App {
    pub(super) fn apply_plan_history(&mut self, update: Update) {
        match update {
            Update::PlanHistory {
                conversation,
                generation,
                offset,
                reply,
            } => {
                if conversation == self.conversation
                    && generation == self.generation
                    && let Some(browser) = self
                        .plan_history
                        .as_mut()
                        .filter(|browser| browser.offset == offset)
                {
                    match reply {
                        Ok(page) => browser.loaded(page),
                        Err(error) => {
                            browser.loading = false;
                            browser.error = Some(error);
                        }
                    }
                }
            }
            _ => unreachable!("update routed to wrong handler"),
        }
    }

    pub(super) fn apply_ledger(&mut self, update: Update) {
        let Update::Ledger {
            conversation,
            request_id,
            kind,
            reply,
        } = update
        else {
            unreachable!("update routed to wrong handler");
        };
        let active = conversation == self.conversation;
        let Some(view) = self
            .ledger_reply_view(&conversation)
            .filter(|view| view.request_id == request_id)
        else {
            return;
        };
        let decided = reply.is_ok() && matches!(kind.as_str(), "applied" | "rejected");
        let refresh = reply.is_ok() && matches!(kind.as_str(), "created" | "runs");
        view.reply(&kind, reply);
        self.finish_ledger_decision(&conversation, active, decided);
        if refresh && active {
            self.ledger_action(ledger::Action::Fetch);
        }
    }

    fn ledger_reply_view(&mut self, conversation: &str) -> Option<&mut ledger::LedgerView> {
        if conversation == self.conversation {
            self.ledger_view.as_mut()
        } else {
            self.conversation_states
                .get_mut(conversation)?
                .ledger_view
                .as_mut()
        }
    }

    fn finish_ledger_decision(&mut self, conversation: &str, active: bool, decided: bool) {
        if !decided {
            return;
        }
        if active {
            self.return_to_main();
        } else if let Some(state) = self.conversation_states.get_mut(conversation) {
            clear_background_ledger_review(state, conversation);
        }
    }

    pub(super) fn apply_config(&mut self, update: Update) {
        let Update::Config {
            project,
            save,
            reply,
        } = update
        else {
            unreachable!("update routed to wrong handler");
        };
        let active = project == self.client.state_dir().to_string_lossy();
        if save {
            self.apply_config_save(&project, active, reply);
        } else {
            self.refresh_config(&project, active, reply);
        }
    }

    fn apply_config_save(
        &mut self,
        project: &str,
        active: bool,
        reply: std::result::Result<Value, String>,
    ) {
        let editor = if active {
            self.config_editor.as_mut()
        } else {
            self.config_drafts.get_mut(project)
        };
        if let Some(editor) = editor {
            editor.pending = false;
            editor.status = save_config(editor, reply);
        }
    }

    fn refresh_config(
        &mut self,
        project: &str,
        active: bool,
        reply: std::result::Result<Value, String>,
    ) {
        match reply.and_then(config::ConfigEditor::from_reply) {
            Ok(fresh) => self.install_fresh_config(project, active, fresh),
            Err(error) => self.record_config_error(project, active, error),
        }
    }

    fn install_fresh_config(
        &mut self,
        project: &str,
        active: bool,
        mut fresh: config::ConfigEditor,
    ) {
        if active {
            if let Some(editor) = &mut self.config_editor {
                editor.refresh(fresh);
            } else {
                self.config_editor = Some(fresh);
            }
            return;
        }
        fresh.visible = false;
        if let Some(editor) = self.config_drafts.get_mut(project) {
            editor.refresh(fresh);
        } else {
            self.config_drafts.insert(project.to_owned(), fresh);
        }
    }

    fn record_config_error(&mut self, project: &str, active: bool, error: String) {
        let editor = if active {
            self.config_editor.as_mut()
        } else {
            self.config_drafts.get_mut(project)
        };
        if let Some(editor) = editor {
            editor.pending = false;
            editor.reload_requested = false;
            editor.status = error.clone();
        }
        self.notice = Some(error);
    }

    pub(super) fn apply_conversation(&mut self, update: Update) {
        match update {
            Update::Conversation(id) => self.open(id),
            _ => unreachable!("update routed to wrong handler"),
        }
    }

    pub(super) fn apply_project_ready(&mut self, update: Update) {
        match update {
            Update::ProjectReady { conn, conversation } => {
                let index = self.register_project(*conn);
                self.switch_project(index, conversation);
            }
            _ => unreachable!("update routed to wrong handler"),
        }
    }

    pub(super) fn apply_project_trust(&mut self, update: Update) {
        match update {
            Update::ProjectTrust(path) => self.project_trust = Some(path),
            _ => unreachable!("update routed to wrong handler"),
        }
    }

    pub(super) fn apply_project_status(&mut self, update: Update) {
        match update {
            Update::ProjectStatus {
                path,
                conversations,
            } => {
                if let Some(project) = self.projects.iter_mut().find(|p| p.path == path) {
                    project.conversations = conversations;
                }
            }
            _ => unreachable!("update routed to wrong handler"),
        }
    }

    pub(super) fn apply_notice(&mut self, update: Update) {
        let Update::Notice(text) = update else {
            unreachable!("update routed to wrong handler");
        };
        if text.contains("stale_scope") {
            self.mark_stale_confirmations();
        }
        self.notice = Some(text);
    }

    fn mark_stale_confirmations(&mut self) {
        if let Some(mut card) = self.confirmation.take() {
            card.status = ConfirmationStatus::Stale;
            mark_confirmation(&mut self.view.cards, &card.id);
        }
        if let Some(card) = self
            .view
            .cards
            .iter_mut()
            .rev()
            .find_map(|card| match card {
                TranscriptCard::Confirmation(card)
                    if card.status == ConfirmationStatus::Confirmed =>
                {
                    Some(card)
                }
                _ => None,
            })
        {
            card.status = ConfirmationStatus::Stale;
        }
    }
}

fn clear_background_ledger_review(state: &mut ConversationState, conversation: &str) {
    state.diff = None;
    state.plan_view = None;
    state.confirmation = None;
    if let Some(details) = state.details.take() {
        state
            .details_scroll_memory
            .insert((details.key.clone(), details.tab), details.scroll);
        state
            .details_query_memory
            .insert((details.key, details.tab), details.query);
    }
    if let Some(editor) = state.editor.take() {
        let key = App::comment_key(conversation, None, &editor.target);
        state.comment_draft_cursors.insert(
            key.clone(),
            (editor.input.cursor, editor.input.preferred_col),
        );
        state.comment_drafts.insert(key, editor.input.text);
    }
}

fn save_config(
    editor: &mut config::ConfigEditor,
    reply: std::result::Result<Value, String>,
) -> String {
    match reply.and_then(|value| apply_saved_config(editor, value.clone())) {
        Ok(value)
            if value["warnings"]
                .as_array()
                .is_some_and(|warnings| !warnings.is_empty()) =>
        {
            format!("Applied live with warnings: {}", value["warnings"])
        }
        Ok(_) => "Saved and applied live.".into(),
        Err(error) => error,
    }
}

fn apply_saved_config(
    editor: &mut config::ConfigEditor,
    value: Value,
) -> std::result::Result<Value, String> {
    let saved = config::ConfigEditor::from_reply(value.clone())?;
    if saved.path != editor.path {
        return Err("Configuration reply refers to a different file".into());
    }
    editor.applied(&saved.base_source);
    Ok(value)
}

fn mark_confirmation(cards: &mut [TranscriptCard], id: &str) {
    for card in cards.iter_mut().rev() {
        if let TranscriptCard::Confirmation(card) = card
            && card.id == id
        {
            card.status = ConfirmationStatus::Stale;
            break;
        }
    }
}

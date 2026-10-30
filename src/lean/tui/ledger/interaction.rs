use super::*;
use crate::lean::ledger::*;
use crate::lean::tui::InputBuffer;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
mod browse;
mod edit;
mod preview;

impl LedgerView {
    pub fn key(&mut self, key: KeyEvent, conversation: &str) -> Action {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if self.confirm_new_run {
            return self.confirm_run_key(key);
        }
        if self.cancel_pending_read(key) {
            return Action::None;
        }
        if self.preview.is_some() {
            return self.preview_key(key, ctrl);
        }
        if self.editing.is_some() {
            return self.edit_key(key, ctrl, conversation);
        }
        if self.pending_action(key) || self.pending_page_edit(key) {
            return Action::None;
        }
        if self.edit_filter_key(key) {
            return self.finish_filter_key(key);
        }
        self.browse_key(key)
    }

    fn confirm_run_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Esc => self.confirm_new_run = false,
            KeyCode::Enter if !self.pending => {
                self.confirm_new_run = false;
                return Action::Create;
            }
            _ => {}
        }
        Action::None
    }

    fn cancel_pending_read(&mut self, key: KeyEvent) -> bool {
        if !self.pending || !self.pending_read || key.code != KeyCode::Esc {
            return false;
        }
        self.save_rejection_draft();
        self.request_id = self.request_id.wrapping_add(1);
        self.pending = false;
        self.visible = false;
        self.preview = None;
        true
    }

    pub(in crate::lean::tui::ledger) fn save_rejection_draft(&mut self) {
        if let Some(preview) = &self.preview {
            let id = preview["proposal_id"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            self.rejection_drafts.insert(id, self.rejection.clone());
        }
    }
}

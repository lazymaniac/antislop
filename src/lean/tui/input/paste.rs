use super::*;

impl App {
    pub fn paste(&mut self, text: &str) {
        if self.paste_native_or_overlay(text) {
            return;
        }
        self.sync_interview();
        if self.paste_form(text) || self.paste_details(text) {
            return;
        }
        self.paste_picker_editor_or_chat(text);
    }

    fn paste_native_or_overlay(&mut self, text: &str) -> bool {
        if self.native_focused() {
            self.native_send(crate::lean::terminal::Input::Paste { text: text.into() });
            return true;
        }
        if self.plan_history.is_some() || self.help.is_some() || self.trust_prompt.is_some() {
            return true;
        }
        if self.ledger_view.as_ref().is_some_and(|view| view.visible) && self.picker.is_none() {
            self.ledger_view.as_mut().unwrap().paste(text);
            return true;
        }
        if let Some(editor) = self.config_editor.as_mut().filter(|editor| editor.visible) {
            editor.paste(text);
            return true;
        }
        false
    }

    fn paste_form(&mut self, text: &str) -> bool {
        if self.picker.is_some() || self.editor.is_some() || !self.interview_visible() {
            return false;
        }
        if let Some(form) = self.interview_forms.get_mut(&self.conversation) {
            form.paste(text);
        }
        true
    }

    fn paste_details(&mut self, text: &str) -> bool {
        let Some(details) = &mut self.details else {
            return false;
        };
        if details.editing && details.selected_round.is_none() {
            let key = details.composer_key();
            details
                .drafts
                .borrow_mut()
                .entry(key)
                .or_default()
                .input
                .insert_str(&sanitize_paste(text));
        }
        true
    }

    fn paste_picker_editor_or_chat(&mut self, text: &str) {
        if let Some(picker) = &mut self.picker {
            picker.query.push_str(&sanitize_single_line_paste(text));
            picker.selected = 0;
        } else if let Some(editor) = &mut self.editor {
            editor.input.insert_str(&sanitize_paste(text));
        } else if self.chat_paste_enabled() {
            self.input.insert_str(&sanitize_paste(text));
        }
    }

    fn chat_paste_enabled(&self) -> bool {
        self.focus == Focus::Chat
            && self.plan_view.is_none()
            && self.diff.is_none()
            && self.details.is_none()
    }
}

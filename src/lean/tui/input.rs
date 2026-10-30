use super::*;

mod paste;

impl App {
    pub fn comment_key(conversation: &str, plan: Option<&str>, target: &str) -> String {
        match plan {
            Some(p) => format!("{conversation}:{p}:{target}"),
            None => format!("{conversation}::{target}"),
        }
    }

    pub fn get_comment_draft(&mut self, target: &str) -> InputBuffer {
        let key = Self::comment_key(&self.conversation, self.view.plan.as_deref(), target);
        self.take_comment_draft(&key)
            .or_else(|| self.take_comment_draft(&format!("{}::{target}", self.conversation)))
            .unwrap_or_default()
    }

    fn take_comment_draft(&mut self, key: &str) -> Option<InputBuffer> {
        let text = self.comment_drafts.remove(key)?;
        let (cursor, preferred_col) = self
            .comment_draft_cursors
            .remove(key)
            .unwrap_or((text.len(), None));
        Some(restored_buffer(text, cursor, preferred_col))
    }

    pub fn save_comment_draft(&mut self, target: &str, input: InputBuffer) {
        let key = Self::comment_key(&self.conversation, self.view.plan.as_deref(), target);
        self.comment_draft_cursors
            .insert(key.clone(), (input.cursor, input.preferred_col));
        self.comment_drafts.insert(key, input.text);
    }

    pub fn remove_comment_draft(&mut self, target: &str) {
        let key = Self::comment_key(&self.conversation, self.view.plan.as_deref(), target);
        self.remove_draft(&key);
        let fallback_key = format!("{}::{target}", self.conversation);
        self.remove_draft(&fallback_key);
    }

    fn remove_draft(&mut self, key: &str) {
        self.comment_drafts.remove(key);
        self.comment_draft_cursors.remove(key);
    }

    pub fn recall_history(&mut self, older: bool) {
        if self.history.is_empty() {
            return;
        }
        self.save_history_cursor();
        let Some(index) = self.history_target(older) else {
            return;
        };
        self.history_index = index;
        self.restore_history_input(index);
    }

    fn save_history_cursor(&mut self) {
        if let Some(curr) = self.history_index {
            self.history_item_cursors.insert(curr, self.input.cursor);
        }
    }

    fn history_target(&mut self, older: bool) -> Option<Option<usize>> {
        match (self.history_index, older) {
            (None, true) => {
                self.history_draft = self.input.text.clone();
                self.history_draft_cursor = self.input.cursor;
                self.history_draft_preferred_col = self.input.preferred_col;
                Some(Some(self.history.len() - 1))
            }
            (None, false) => None,
            (Some(i), true) => Some(Some(i.saturating_sub(1))),
            (Some(i), false) if i + 1 < self.history.len() => Some(Some(i + 1)),
            (Some(_), false) => Some(None),
        }
    }

    fn restore_history_input(&mut self, index: Option<usize>) {
        match index {
            Some(i) => {
                let text = self.history[i].clone();
                let saved_cursor = self.history_item_cursors.get(&i).copied();
                self.input = restored_buffer(text, saved_cursor.unwrap_or(usize::MAX), None);
            }
            None => {
                let text = std::mem::take(&mut self.history_draft);
                self.input = restored_buffer(
                    text,
                    self.history_draft_cursor,
                    self.history_draft_preferred_col,
                );
            }
        }
    }

    /// Restores the project's input history before a conversation is opened.
    pub fn load_history(&mut self, state: &std::path::Path) {
        let path = state.join("input-history.db");
        self.history = match history::load(&path) {
            Ok(entries) => entries,
            Err(error) => {
                self.notice = Some(format!(
                    "Cannot load input history ({}): {error}",
                    path.display()
                ));
                Vec::new()
            }
        };
        self.history_path = Some(path);
    }

    pub fn record_history(&mut self, text: &str) {
        self.history_index = None;
        self.history_draft.clear();
        self.history_draft_cursor = 0;
        self.history_draft_preferred_col = None;
        self.history_item_cursors.clear();
        let trimmed = text.trim();
        if !trimmed.is_empty() && self.history.last().map(String::as_str) != Some(trimmed) {
            self.history.push(trimmed.to_owned());
            if self.history.len() > history::LIMIT {
                self.history.remove(0);
            }
            if let Some(path) = &self.history_path
                && let Err(error) = history::append(path, trimmed)
            {
                self.notice = Some(format!(
                    "Cannot save input history ({}): {error}",
                    path.display()
                ));
            }
        }
    }

    pub(super) fn chat_key(&mut self, key: event::KeyEvent) -> bool {
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Esc => self.chat_escape(),
            KeyCode::Char('a') if control => self.chat_home_or_accept(),
            KeyCode::Char('r') if control => self.chat_history_or_resume(),
            KeyCode::Char('x') if control => self.command("/cancel"),
            KeyCode::Char('u') if control && self.input.is_empty() => self.scroll += 5,
            KeyCode::Char('d') if control && self.input.is_empty() => {
                self.scroll = self.scroll.saturating_sub(5);
            }
            KeyCode::Char('s') if control => self.submit(),
            KeyCode::Enter if !alt => self.submit(),
            KeyCode::End if alt || control => self.chat_buffer_end(),
            KeyCode::PageUp => self.scroll += 10,
            KeyCode::PageDown => self.scroll = self.scroll.saturating_sub(10),
            KeyCode::Up => self.chat_up(),
            KeyCode::Down => self.chat_down(),
            KeyCode::Char('@') if !control && self.input.is_empty() => {
                self.picker = Some(Picker::new(Kind::Recipients));
            }
            _ => self.input.edit_key(key),
        }
        true
    }

    fn chat_escape(&mut self) {
        if self.history_index.is_some() {
            self.history_index = None;
            let text = std::mem::take(&mut self.history_draft);
            self.input = restored_buffer(
                text,
                self.history_draft_cursor,
                self.history_draft_preferred_col,
            );
        } else if !self.input.is_empty() {
            self.history_draft = self.input.text.clone();
            self.history_draft_cursor = self.input.cursor;
            self.history_draft_preferred_col = self.input.preferred_col;
            self.input.clear();
            self.input_shown = 0;
        }
    }

    fn chat_home_or_accept(&mut self) {
        if self.input.is_empty()
            && (self.view.plan_proposed()
                || self.view.workflow_phase.as_deref() == Some("interview"))
        {
            self.command("/accept");
        } else {
            self.input.move_home();
        }
    }

    fn chat_history_or_resume(&mut self) {
        if self.input.is_empty() && self.view.plan_resumable() {
            self.command("/resume");
        } else {
            self.picker = Some(Picker::new(Kind::History));
        }
    }

    fn chat_buffer_end(&mut self) {
        self.input.move_buffer_end();
        self.scroll = 0;
    }

    fn chat_up(&mut self) {
        let width = self.input_width.max(1);
        if self.input.is_empty() {
            if self.history.is_empty() {
                self.scroll += 1;
            } else {
                self.recall_history(true);
            }
        } else if self.input.on_first_row(width) && !self.history.is_empty() {
            self.recall_history(true);
        } else if self.input.visual_cursor(width).1 > 0 {
            self.input.move_up(width);
        }
    }

    fn chat_down(&mut self) {
        let width = self.input_width.max(1);
        if self.history_index.is_some() && self.input.on_last_row(width) {
            self.recall_history(false);
        } else if self.input.is_empty() {
            self.scroll = self.scroll.saturating_sub(1);
        } else {
            self.chat_down_with_text(width);
        }
    }

    fn chat_down_with_text(&mut self, width: usize) {
        let (_, row) = self.input.visual_cursor(width);
        if row + 1 < self.input.visual_line_count(width) {
            self.input.move_down(width);
        } else if self.scroll > 0 {
            self.scroll = self.scroll.saturating_sub(1);
        }
    }

    pub(super) fn editor_key(&mut self, key: event::KeyEvent) {
        let Some(editor) = &mut self.editor else {
            return;
        };
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Esc => {
                if let Some(editor) = self.editor.take().filter(|e| !e.input.is_empty()) {
                    self.save_comment_draft(&editor.target, editor.input);
                }
            }
            KeyCode::Char('s') if control => self.submit_comment(),
            KeyCode::Enter if !alt => self.submit_comment(),
            KeyCode::Up => editor.input.move_up(self.editor_width.max(1)),
            KeyCode::Down => editor.input.move_down(self.editor_width.max(1)),
            _ => editor.input.edit_key(key),
        }
    }
}

fn restored_buffer(text: String, cursor: usize, preferred_col: Option<u16>) -> InputBuffer {
    let mut buffer = InputBuffer::with_text(text);
    buffer.cursor = cursor;
    buffer.preferred_col = preferred_col;
    buffer.clamp_cursor();
    buffer
}

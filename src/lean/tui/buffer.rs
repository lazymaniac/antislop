use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_segmentation::UnicodeSegmentation;

/// An editable text buffer tracking content and cursor byte offset.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InputBuffer {
    pub text: String,
    pub cursor: usize, // Byte offset in text
    pub preferred_col: Option<u16>,
}

impl InputBuffer {
    /// Shared draft editing. Submission, history and vertical navigation belong
    /// to the caller, since chat and comment dialogs handle them differently.
    pub fn edit_key(&mut self, key: KeyEvent) {
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Enter if alt => self.insert_char('\n'),
            KeyCode::Char('j') if control => self.insert_char('\n'),
            KeyCode::Char('w') if control => self.delete_word_back(),
            KeyCode::Char('u') if control => self.delete_to_start(),
            KeyCode::Char('k') if control => self.delete_to_end(),
            KeyCode::Char('a') if control => self.move_home(),
            KeyCode::Char('e') if control => self.move_end(),
            KeyCode::Backspace if alt || control => self.delete_word_back(),
            KeyCode::Backspace => {
                self.backspace();
            }
            KeyCode::Delete => {
                self.delete();
            }
            KeyCode::Left if alt || control => self.move_word_left(),
            KeyCode::Right if alt || control => self.move_word_right(),
            KeyCode::Left => self.move_left(),
            KeyCode::Right => self.move_right(),
            KeyCode::Home if alt || control => self.move_buffer_start(),
            KeyCode::End if alt || control => self.move_buffer_end(),
            KeyCode::Home => self.move_home(),
            KeyCode::End => self.move_end(),
            KeyCode::Char(ch) if !control => self.insert_char(ch),
            _ => {}
        }
    }

    pub fn new() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            preferred_col: None,
        }
    }

    pub fn with_text(text: impl Into<String>) -> Self {
        let text = text.into();
        let cursor = text.len();
        Self {
            text,
            cursor,
            preferred_col: None,
        }
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.preferred_col = None;
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub(super) fn clamp_cursor(&mut self) {
        if self.cursor > self.text.len() {
            self.cursor = self.text.len();
        }
        while self.cursor > 0 && !self.text.is_char_boundary(self.cursor) {
            self.cursor -= 1;
        }
        if self.cursor < self.text.len() {
            self.cursor = self
                .text
                .grapheme_indices(true)
                .map(|(i, _)| i)
                .find(|i| *i >= self.cursor)
                .unwrap_or(self.text.len());
        }
    }

    // Core editing actions

    pub fn insert_char(&mut self, ch: char) {
        let mut buf = [0u8; 4];
        self.insert_str(ch.encode_utf8(&mut buf));
    }

    pub fn insert_str(&mut self, s: &str) {
        self.preferred_col = None;
        self.clamp_cursor();
        self.text.insert_str(self.cursor, s);
        self.cursor += s.len();
        self.cursor = self
            .text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .find(|i| *i >= self.cursor)
            .unwrap_or(self.text.len());
    }

    pub fn backspace(&mut self) -> bool {
        self.preferred_col = None;
        self.clamp_cursor();
        if self.cursor == 0 {
            return false;
        }
        let previous = self.text[..self.cursor]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(i, _)| i);
        self.text.drain(previous..self.cursor);
        self.cursor = previous;
        true
    }

    pub fn delete(&mut self) -> bool {
        self.preferred_col = None;
        self.clamp_cursor();
        if self.cursor >= self.text.len() {
            return false;
        }
        let next = self.text[self.cursor..]
            .graphemes(true)
            .next()
            .map_or(self.cursor, |g| self.cursor + g.len());
        self.text.drain(self.cursor..next);
        true
    }

    pub fn delete_word_back(&mut self) {
        self.preferred_col = None;
        self.clamp_cursor();
        if self.cursor == 0 {
            return;
        }
        let before = &self.text[..self.cursor];
        let mut graphemes: Vec<(usize, &str)> = before.grapheme_indices(true).collect();
        // Skip spaces immediately before cursor
        while let Some((_, g)) = graphemes.last() {
            if g.chars().all(char::is_whitespace) {
                graphemes.pop();
            } else {
                break;
            }
        }
        // Skip non-spaces
        while let Some((_, g)) = graphemes.last() {
            if !g.chars().all(char::is_whitespace) {
                graphemes.pop();
            } else {
                break;
            }
        }
        let new_cursor = graphemes.last().map(|(idx, g)| idx + g.len()).unwrap_or(0);
        self.text.drain(new_cursor..self.cursor);
        self.cursor = new_cursor;
    }

    pub fn delete_to_start(&mut self) {
        self.preferred_col = None;
        self.clamp_cursor();
        let before = &self.text[..self.cursor];
        let start = before.rfind('\n').map(|idx| idx + 1).unwrap_or(0);
        self.text.drain(start..self.cursor);
        self.cursor = start;
    }

    pub fn delete_to_end(&mut self) {
        self.preferred_col = None;
        self.clamp_cursor();
        if self.cursor >= self.text.len() {
            return;
        }
        let after = &self.text[self.cursor..];
        let end = match after.find('\n') {
            Some(0) => self.cursor + 1,
            Some(idx) => self.cursor + idx,
            None => self.text.len(),
        };
        self.text.drain(self.cursor..end);
    }
}

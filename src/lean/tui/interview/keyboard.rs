use super::Form;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

impl Form {
    /// Returns an answer only on explicit Enter. Selecting an option never sends.
    pub fn key(&mut self, key: KeyEvent) -> Option<String> {
        if self.key_global(key) {
            return None;
        }
        if self.sending {
            return None;
        }
        self.manual_scroll = false;
        let custom = self.question.options.len();
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        self.navigate_choice(key.code, custom);
        if let Some(answer) = self.submit_or_insert_newline(key, custom) {
            return Some(answer);
        }
        if self.edit_key(key, custom, control) {
            return None;
        }
        self.insert_character(key, custom, control);
        None
    }

    fn key_global(&mut self, key: KeyEvent) -> bool {
        if key.code == KeyCode::Esc {
            self.visible = false;
            return true;
        }
        if matches!(key.code, KeyCode::PageUp | KeyCode::PageDown) {
            self.scroll_page(key.code == KeyCode::PageDown);
            true
        } else {
            false
        }
    }

    fn scroll_page(&mut self, down: bool) {
        self.scroll = if down {
            self.scroll.saturating_add(5)
        } else {
            self.scroll.saturating_sub(5)
        };
        self.manual_scroll = true;
    }

    fn navigate_choice(&mut self, code: KeyCode, custom: usize) {
        match code {
            KeyCode::Up | KeyCode::BackTab => {
                self.selected = self.selected.checked_sub(1).unwrap_or(custom)
            }
            KeyCode::Down | KeyCode::Tab => self.selected = (self.selected + 1) % (custom + 1),
            _ => {}
        }
    }

    fn submit_or_insert_newline(&mut self, key: KeyEvent, custom: usize) -> Option<String> {
        if key.code != KeyCode::Enter {
            return None;
        }
        if key.modifiers.contains(KeyModifiers::ALT) && self.selected == custom {
            self.input.insert_char('\n');
            return None;
        }
        let answer = if self.selected == custom {
            self.input.text.trim().to_owned()
        } else {
            self.question.options[self.selected].label.clone()
        };
        if answer.is_empty() {
            self.error = Some("Enter your answer first.".into());
            None
        } else {
            Some(format!(
                "Interview answer ({})\nQuestion: {}\nAnswer: {}",
                self.question.id, self.question.question, answer
            ))
        }
    }

    fn edit_key(&mut self, key: KeyEvent, custom: usize, control: bool) -> bool {
        if control {
            match key.code {
                KeyCode::Char('a') => self.input.move_home(),
                KeyCode::Char('e') => self.input.move_end(),
                KeyCode::Char('w') => self.input.delete_word_back(),
                KeyCode::Char('u') => self.input.delete_to_start(),
                KeyCode::Char('k') => self.input.delete_to_end(),
                _ => return false,
            }
            return true;
        }
        if self.selected != custom {
            return false;
        }
        match key.code {
            KeyCode::Left => self.input.move_left(),
            KeyCode::Right => self.input.move_right(),
            KeyCode::Home => self.input.move_home(),
            KeyCode::End => self.input.move_end(),
            KeyCode::Backspace => {
                self.input.backspace();
            }
            KeyCode::Delete => {
                self.input.delete();
            }
            _ => return false,
        }
        true
    }

    fn insert_character(&mut self, key: KeyEvent, custom: usize, control: bool) {
        let KeyCode::Char(ch) = key.code else {
            return;
        };
        if control || key.modifiers.contains(KeyModifiers::ALT) {
            return;
        }
        if self.selected != custom
            && ch
                .to_digit(10)
                .is_some_and(|n| n > 0 && n as usize <= custom)
        {
            self.selected = ch.to_digit(10).unwrap() as usize - 1;
        } else {
            self.selected = custom;
            self.input.insert_char(ch);
        }
    }
}

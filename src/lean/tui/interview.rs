//! Keyboard-owned interview card with an independent, grapheme-safe draft.
use super::{InputBuffer, Theme, sanitize_paste};
use crate::lean::interview::Question;

#[derive(Debug, Clone, PartialEq)]
pub struct Form {
    pub event: u64,
    pub question: Question,
    pub selected: usize,
    pub input: InputBuffer,
    pub visible: bool,
    pub sending: bool,
    pub sent: bool,
    pub error: Option<String>,
    scroll: usize,
    manual_scroll: bool,
}

impl Form {
    pub fn new(event: u64, question: Question) -> Self {
        Self {
            event,
            question,
            selected: 0,
            input: InputBuffer::new(),
            visible: true,
            sending: false,
            sent: false,
            error: None,
            scroll: 0,
            manual_scroll: false,
        }
    }

    pub fn paste(&mut self, text: &str) {
        if !self.sending {
            self.selected = self.question.options.len();
            self.manual_scroll = false;
            self.input.insert_str(&sanitize_paste(text));
        }
    }
}

mod keyboard;
mod render;

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};

    fn form() -> Form {
        Form::new(7, serde_json::from_value(serde_json::json!({"id":"scope","context":"This determines delivery time.","question":"Which scope?","options":[{"label":"Small","description":"Earlier delivery"},{"label":"Full","description":"More complete"}]})).unwrap())
    }

    #[test]
    fn choices_and_custom_answers_require_explicit_submission() {
        let mut form = form();
        assert!(
            form.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
                .is_none()
        );
        assert!(
            form.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
                .unwrap()
                .ends_with("Answer: Full")
        );
        form.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert!(
            form.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
                .is_none()
        );
        form.paste("3 👩‍💻e\u{301}");
        form.key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(form.input.text, "3 👩‍💻");
        form.key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(form.input.text, "3 ");
        form.paste("custom answer");
        assert!(
            form.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
                .unwrap()
                .ends_with("Answer: 3 custom answer")
        );
        form.sending = true;
        form.paste("must not edit pending answer");
        assert_eq!(form.input.text, "3 custom answer");
    }

    #[test]
    fn question_context_choices_and_custom_field_render_in_a_bounded_viewport() {
        let mut form = form();
        for (width, height) in [(94, 25), (40, 12), (12, 5)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| form.draw(frame, frame.area(), &Theme::new(false, true)))
                .unwrap();
            assert_eq!(
                terminal.backend().buffer().content.len(),
                usize::from(width * height)
            );
            if width == 94 {
                let text = terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect::<String>();
                for required in [
                    "This determines delivery time.",
                    "Which scope?",
                    "Small",
                    "Full",
                    "Other / own answer:",
                ] {
                    assert!(text.contains(required), "missing {required}");
                }
            }
            form.selected = form.question.options.len();
            form.paste("A long custom answer 👩‍💻".repeat(8).as_str());
            terminal
                .draw(|frame| form.draw(frame, frame.area(), &Theme::new(false, true)))
                .unwrap();
        }
    }
}

//! Keyboard-owned interview card with an independent, grapheme-safe draft.
use super::{InputBuffer, Theme, sanitize_paste};
use crate::lean::interview::Question;

#[derive(Debug, Clone, PartialEq)]
pub struct Form {
    pub event: u64,
    pub question: Question,
    pub scope_summary: Option<String>,
    pub selected: usize,
    pub input: InputBuffer,
    pub visible: bool,
    pub sending: bool,
    pub sent: bool,
    pub error: Option<String>,
    scroll: usize,
    scroll_step: usize,
    manual_scroll: bool,
}

impl Form {
    pub fn new(event: u64, question: Question) -> Self {
        Self {
            event,
            question,
            scope_summary: None,
            selected: 0,
            input: InputBuffer::new(),
            visible: true,
            sending: false,
            sent: false,
            error: None,
            scroll: 0,
            scroll_step: 1,
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

    pub fn set_scope_summary(&mut self, summary: Option<&str>) {
        if self.scope_summary.as_deref() != summary {
            self.scope_summary = summary.map(str::to_owned);
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

    #[test]
    fn unchanged_scope_summary_keeps_its_allocation() {
        let mut form = form();
        let summary = "A potentially long scope summary".repeat(100);
        form.set_scope_summary(Some(&summary));
        let allocation = form.scope_summary.as_ref().unwrap().as_ptr();
        let capacity = form.scope_summary.as_ref().unwrap().capacity();

        form.set_scope_summary(Some(&summary));

        let stored = form.scope_summary.as_ref().unwrap();
        assert_eq!(stored.as_ptr(), allocation);
        assert_eq!(stored.capacity(), capacity);
    }

    #[test]
    fn full_scope_and_decision_choices_are_reachable_on_minimum_terminal() {
        let question = serde_json::from_value(serde_json::json!({
            "id":"scope-confirmation",
            "context":"Planning must use the approved scope.",
            "question":"Confirm this scope?",
            "options":[
                {"label":"Confirm scope","description":"Proceed with planning."},
                {"label":"Request changes","description":"Keep the interview open."}
            ]
        }))
        .unwrap();
        let mut form = Form::new(7, question);
        let scope = (0..18)
            .map(|index| format!("PART{index:02}"))
            .collect::<Vec<_>>()
            .join(" ");
        form.set_scope_summary(Some(&scope));
        let mut terminal = Terminal::new(TestBackend::new(12, 5)).unwrap();
        let mut visited = String::new();
        terminal
            .draw(|frame| form.draw(frame, frame.area(), &Theme::new(false, true)))
            .unwrap();
        while form.scroll > 0 {
            form.key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
            terminal
                .draw(|frame| form.draw(frame, frame.area(), &Theme::new(false, true)))
                .unwrap();
        }
        let mut previous_scroll = usize::MAX;
        for _ in 0..100 {
            terminal
                .draw(|frame| form.draw(frame, frame.area(), &Theme::new(false, true)))
                .unwrap();
            visited.push_str(
                &terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect::<String>(),
            );
            if form.scroll == previous_scroll {
                break;
            }
            previous_scroll = form.scroll;
            form.key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        }
        for part in (0..18).map(|index| format!("PART{index:02}")) {
            assert!(
                visited.contains(&part),
                "scope text was not reachable: {part}"
            );
        }
        for (selected, label, second_word) in [(0, "Confirm", "scope"), (1, "Request", "changes")] {
            form.selected = selected;
            form.manual_scroll = false;
            terminal
                .draw(|frame| form.draw(frame, frame.area(), &Theme::new(false, true)))
                .unwrap();
            let text = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(
                text.contains(label) && text.contains(second_word),
                "choice is not visible: {label} {second_word}; rendered={text:?}; scroll={}",
                form.scroll
            );
            let answer = form
                .key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
                .unwrap();
            assert!(answer.ends_with(&format!(
                "Answer: {}",
                if selected == 0 {
                    "Confirm scope"
                } else {
                    "Request changes"
                }
            )));
        }
    }
}

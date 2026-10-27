//! Keyboard-owned interview card with an independent, grapheme-safe draft.
use super::{InputBuffer, Theme, sanitize_paste, view::wrap};
use crate::lean::interview::Question;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::Rect,
    style::Modifier,
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph},
};

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

    /// Returns an answer only on explicit Enter. Selecting an option never sends.
    pub fn key(&mut self, key: KeyEvent) -> Option<String> {
        if key.code == KeyCode::Esc {
            self.visible = false;
            return None;
        }
        if self.sending {
            return None;
        }
        if matches!(key.code, KeyCode::PageUp | KeyCode::PageDown) {
            self.scroll = if key.code == KeyCode::PageUp {
                self.scroll.saturating_sub(5)
            } else {
                self.scroll.saturating_add(5)
            };
            self.manual_scroll = true;
            return None;
        }
        self.manual_scroll = false;
        let custom = self.question.options.len();
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Up | KeyCode::BackTab => {
                self.selected = self.selected.checked_sub(1).unwrap_or(custom)
            }
            KeyCode::Down | KeyCode::Tab => self.selected = (self.selected + 1) % (custom + 1),
            KeyCode::Enter
                if key.modifiers.contains(KeyModifiers::ALT) && self.selected == custom =>
            {
                self.input.insert_char('\n')
            }
            KeyCode::Enter => {
                let answer = if self.selected == custom {
                    self.input.text.trim().to_owned()
                } else {
                    self.question.options[self.selected].label.clone()
                };
                if answer.is_empty() {
                    self.error = Some("Enter your answer first.".into());
                } else {
                    return Some(format!(
                        "Interview answer ({})\nQuestion: {}\nAnswer: {}",
                        self.question.id, self.question.question, answer
                    ));
                }
            }
            KeyCode::Char('a') if control => self.input.move_home(),
            KeyCode::Char('e') if control => self.input.move_end(),
            KeyCode::Char('w') if control => self.input.delete_word_back(),
            KeyCode::Char('u') if control => self.input.delete_to_start(),
            KeyCode::Char('k') if control => self.input.delete_to_end(),
            KeyCode::Left if self.selected == custom => self.input.move_left(),
            KeyCode::Right if self.selected == custom => self.input.move_right(),
            KeyCode::Home if self.selected == custom => self.input.move_home(),
            KeyCode::End if self.selected == custom => self.input.move_end(),
            KeyCode::Backspace if self.selected == custom => {
                self.input.backspace();
            }
            KeyCode::Delete if self.selected == custom => {
                self.input.delete();
            }
            KeyCode::Char(ch) if !control && !key.modifiers.contains(KeyModifiers::ALT) => {
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
            _ => {}
        }
        None
    }

    pub fn draw(&mut self, frame: &mut Frame, screen: Rect, theme: &Theme) {
        let width = screen.width.min(94);
        let area = Rect::new(
            screen.x + screen.width.saturating_sub(width) / 2,
            screen.y,
            width,
            screen.height,
        );
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Question · choose an answer ")
            .border_style(theme.accent());
        let inner_width = usize::from(area.width.saturating_sub(3).max(1));
        let mut rows = Vec::new();
        let mut append = |text: &str, style| {
            rows.extend(
                wrap(&theme.clean(text), inner_width)
                    .into_iter()
                    .map(|text| Line::styled(text, style)),
            );
        };
        append(&self.question.context, theme.dim());
        append("", theme.dim());
        append(&self.question.question, theme.title());
        append("", theme.dim());
        let mut selected_row = rows.len();
        for (index, option) in self.question.options.iter().enumerate() {
            let selected = self.selected == index;
            if selected {
                selected_row = rows.len();
            }
            let style = if selected {
                theme.accent().add_modifier(Modifier::REVERSED)
            } else {
                theme.text()
            };
            rows.extend(
                wrap(
                    &theme.clean(&format!(
                        "{} {}. {}",
                        if selected { ">" } else { " " },
                        index + 1,
                        option.label
                    )),
                    inner_width,
                )
                .into_iter()
                .map(|text| Line::styled(text, style)),
            );
            rows.extend(
                wrap(
                    &theme.clean(&format!("   {}", option.description)),
                    inner_width,
                )
                .into_iter()
                .map(|text| Line::styled(text, theme.dim())),
            );
        }
        let custom = self.selected == self.question.options.len();
        if custom {
            selected_row = rows.len();
        }
        rows.push(Line::styled(
            "Other / own answer:",
            if custom { theme.accent() } else { theme.dim() },
        ));
        let input_row = rows.len();
        let input_lines = super::visual_text_lines(&self.input.text, inner_width);
        if self.input.is_empty() {
            rows.push(Line::styled("Type a different answer here…", theme.dim()));
        } else {
            rows.extend(
                input_lines
                    .iter()
                    .map(|text| Line::styled(theme.clean(text), theme.text())),
            );
        }
        if let Some(error) = &self.error {
            rows.extend(
                wrap(&theme.clean(error), inner_width)
                    .into_iter()
                    .map(|text| Line::styled(text, theme.error())),
            );
        }
        rows.push(Line::styled(
            if self.sending {
                "Sending… draft preserved"
            } else {
                "Up/Down/Tab choose · Enter send · PgUp/PgDn scroll · Esc chat"
            },
            theme.dim(),
        ));
        let height = (rows.len().saturating_add(2).min(usize::from(area.height))) as u16;
        let area = Rect::new(
            area.x,
            area.y + area.height.saturating_sub(height) / 2,
            area.width,
            height,
        );
        let viewport = usize::from(area.height.saturating_sub(2).max(1));
        let (cursor_col, cursor_row) = self.input.visual_cursor(inner_width);
        let target = if custom {
            input_row + cursor_row
        } else {
            selected_row
        };
        if !self.manual_scroll {
            if target < self.scroll {
                self.scroll = target;
            }
            if target >= self.scroll + viewport {
                self.scroll = target + 1 - viewport;
            }
        }
        self.scroll = self.scroll.min(rows.len().saturating_sub(viewport));
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(
                rows.into_iter()
                    .skip(self.scroll)
                    .take(viewport)
                    .collect::<Vec<_>>(),
            )
            .style(theme.card())
            .block(block),
            area,
        );
        if custom && !self.sending && !self.manual_scroll && area.width >= 3 && area.height >= 3 {
            frame.set_cursor_position((
                area.x + 1 + cursor_col.min(area.width.saturating_sub(3)),
                area.y
                    + 1
                    + (target.saturating_sub(self.scroll) as u16)
                        .min(area.height.saturating_sub(3)),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

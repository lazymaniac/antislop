//! Full recent messages with independent scrolling from the graph canvas.
use super::render::transcript_cards::message_card_props;
use super::theme::{CardProps, Row, Theme, card, draw_rows, scrollbar, split_row};
use super::view::{MessageEntry, View};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Frame, layout::Rect, text::Span};
use std::collections::HashMap;

mod format;

#[derive(Default)]
pub(super) struct MessageInspector {
    pub area: Rect,
    positions: HashMap<(String, String), usize>,
    context: Option<(String, String, u16, Theme)>,
    messages: Vec<MessageEntry>,
    rows: Vec<Row>,
    height: usize,
}

impl MessageInspector {
    pub fn draw(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        view: &View,
        conversation: &str,
        key: &str,
        theme: &Theme,
    ) {
        if area.width < 4 || area.height < 2 {
            return;
        }
        self.area = area;
        let hint = u16::from(area.height >= 4);
        let body = Rect::new(area.x, area.y + 1, area.width - 1, area.height - 1 - hint);
        self.height = usize::from(body.height);
        self.prepare(view, conversation, key, body.width, theme);
        let identity = (conversation.to_owned(), key.to_owned());
        let max = self.rows.len().saturating_sub(self.height);
        let top = self.positions.entry(identity).or_default();
        *top = (*top).min(max);
        let top = *top;
        let heading = split_row(
            theme.card(),
            vec![Span::styled("Recent messages", theme.accent())],
            Span::styled(
                format!(
                    "{}-{}/{}",
                    top + 1,
                    (top + self.height).min(self.rows.len()),
                    self.rows.len()
                ),
                theme.dim(),
            ),
            usize::from(area.width),
        );
        draw_rows(
            frame.buffer_mut(),
            Rect::new(area.x, area.y, area.width, 1),
            [&heading],
        );
        draw_rows(frame.buffer_mut(), body, self.rows.iter().skip(top));
        scrollbar(
            frame.buffer_mut(),
            theme,
            Rect::new(area.right() - 1, body.y, 1, body.height),
            self.rows.len(),
            top,
            self.height,
        );
        if hint > 0 {
            let hint = Row::new(
                theme.card(),
                vec![Span::styled(
                    theme.fit(
                        &theme.chrome("wheel · Shift+PgUp/PgDn"),
                        usize::from(area.width),
                    ),
                    theme.dim(),
                )],
            );
            draw_rows(
                frame.buffer_mut(),
                Rect::new(area.x, area.bottom() - 1, area.width, 1),
                [&hint],
            );
        }
    }

    fn prepare(&mut self, view: &View, conversation: &str, key: &str, width: u16, theme: &Theme) {
        let context = (conversation.to_owned(), key.to_owned(), width, *theme);
        let messages: Vec<_> = view
            .messages
            .iter()
            .rev()
            .filter(|message| view.message_involves(message, key))
            .take(3)
            .collect();
        if self.context.as_ref() == Some(&context)
            && self.messages.iter().eq(messages.iter().copied())
        {
            return;
        }
        let anchor = self.messages.first().map(|message| message.id.as_str());
        let inserted = if self.context.as_ref() == Some(&context) {
            messages
                .iter()
                .position(|message| Some(message.id.as_str()) == anchor)
                .unwrap_or(0)
        } else {
            0
        };
        let mut added = 0;
        self.rows.clear();
        for (index, message) in messages.iter().enumerate() {
            let rows = formatted_message(message, width, theme);
            if index < inserted {
                added += rows.len();
            }
            self.rows.extend(rows);
        }
        if self.rows.is_empty() {
            self.rows.push(Row::new(
                theme.card(),
                vec![Span::styled("No messages yet.", theme.dim())],
            ));
        }
        if let Some(top) = self
            .positions
            .get_mut(&(conversation.to_owned(), key.to_owned()))
            && *top > 0
        {
            *top = top.saturating_add(added);
        }
        self.messages = messages.into_iter().cloned().collect();
        self.context = Some(context);
    }

    pub fn matches(&self, conversation: &str, key: &str) -> bool {
        !self.area.is_empty()
            && self
                .context
                .as_ref()
                .is_some_and(|context| context.0 == conversation && context.1 == key)
    }

    pub fn scroll(&mut self, down: bool, amount: usize) {
        let Some((conversation, key, ..)) = &self.context else {
            return;
        };
        let top = self
            .positions
            .entry((conversation.clone(), key.clone()))
            .or_default();
        *top = if down {
            top.saturating_add(amount)
                .min(self.rows.len().saturating_sub(self.height))
        } else {
            top.saturating_sub(amount)
        };
    }

    pub fn key(&mut self, key: KeyEvent) -> bool {
        if key.modifiers != KeyModifiers::SHIFT || self.area.is_empty() {
            return false;
        }
        let (down, amount) = match key.code {
            KeyCode::PageUp => (false, self.height.saturating_sub(1).max(1)),
            KeyCode::PageDown => (true, self.height.saturating_sub(1).max(1)),
            KeyCode::Home => (false, usize::MAX),
            KeyCode::End => (true, usize::MAX),
            _ => return false,
        };
        self.scroll(down, amount);
        true
    }
}

fn formatted_message(message: &MessageEntry, width: u16, theme: &Theme) -> Vec<Row> {
    formatted_card(theme, message_card_props(message, false, width, theme))
}

pub(in crate::lean::tui) fn formatted_card(theme: &Theme, props: CardProps<'_>) -> Vec<Row> {
    let body = props.body;
    let width = props.width;
    let fill = props.fill;
    let accent = props.accent;
    let mut rows = card(theme, CardProps { body: "", ..props });
    rows.pop();
    for line in format::body(body, usize::from(width.saturating_sub(3)), theme) {
        let mut spans = vec![
            Span::styled(theme.glyphs().bar, fill.patch(accent)),
            Span::styled(" ", fill),
        ];
        spans.extend(
            line.spans
                .into_iter()
                .map(|span| Span::styled(span.content, fill.patch(span.style))),
        );
        rows.push(Row::new(fill, spans));
    }
    rows.push(Row::gap());
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    #[test]
    fn full_markdown_messages_scroll_to_their_end_and_keep_graphemes() {
        let mut view = View::default();
        view.messages.push(MessageEntry {
            id: "message".into(), sent_at: None, sender:"user".into(), target:"orchestrator".into(),
            text: format!("# Heading\n**Bold** and `code`\n\n```rust\nlet emoji = \"👩‍💻é\";\n```\n{}\nCOMPLETE TAIL", "long message 👨‍👩‍👧‍👦 ".repeat(120)), state:"sent".into(), kind:"message".into()
        });
        let mut inspector = MessageInspector::default();
        let theme = Theme::new(true, false);
        let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
        terminal
            .draw(|frame| inspector.draw(frame, frame.area(), &view, "c", "orchestrator", &theme))
            .unwrap();
        let text = inspector
            .rows
            .iter()
            .map(|row| row.line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            text.contains("Heading") && text.contains("Bold") && text.contains("COMPLETE TAIL")
        );
        assert!(!text.contains("**Bold**") && !text.contains("```"));
        assert!(text.contains("👩‍💻é") && text.contains("👨‍👩‍👧‍👦"));
        inspector.key(KeyEvent::new(KeyCode::End, KeyModifiers::SHIFT));
        terminal
            .draw(|frame| inspector.draw(frame, frame.area(), &view, "c", "orchestrator", &theme))
            .unwrap();
        let screen = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(screen.contains("COMPLETE TAIL"));
        inspector.scroll(false, usize::MAX);
        assert_eq!(inspector.positions[&("c".into(), "orchestrator".into())], 0);
    }
}

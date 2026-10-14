//! Grapheme-aware editing. Byte indices never escape this module.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, Default)]
pub struct Editor {
    pub text: String,
    cursor: usize,
    preferred_column: Option<usize>,
}
impl Editor {
    pub fn cursor(&self) -> usize {
        self.cursor
    }
    pub fn set(&mut self, text: String) {
        self.cursor = text.len();
        self.text = text;
        self.preferred_column = None;
    }
    pub fn insert(&mut self, text: &str) {
        self.text.insert_str(self.cursor, text);
        self.cursor += text.len();
        // A combining character can merge with the preceding grapheme.
        self.cursor = self
            .text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .find(|i| *i >= self.cursor)
            .unwrap_or(self.text.len());
        self.preferred_column = None;
    }
    pub fn position(&self) -> (usize, usize) {
        let before = &self.text[..self.cursor];
        (
            before.matches('\n').count(),
            before.rsplit('\n').next().unwrap_or("").width(),
        )
    }
    fn previous(&self) -> usize {
        self.text[..self.cursor]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(i, _)| i)
    }
    fn next(&self) -> usize {
        self.text[self.cursor..]
            .graphemes(true)
            .next()
            .map_or(self.cursor, |g| self.cursor + g.len())
    }
    fn vertical(&mut self, down: bool) {
        let (row, col) = self.position();
        let desired = self.preferred_column.unwrap_or(col);
        let target = if down { row + 1 } else { row.saturating_sub(1) };
        let mut start = 0;
        if let Some(line) = self.text.split('\n').nth(target) {
            for line in self.text.split('\n').take(target) {
                start += line.len() + 1;
            }
            let mut width = 0;
            self.cursor = start;
            for g in line.graphemes(true) {
                if width + g.width() > desired {
                    break;
                }
                width += g.width();
                self.cursor += g.len();
            }
        }
        self.preferred_column = Some(desired);
    }
    pub fn key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.insert(&c.to_string())
            }
            KeyCode::Enter => self.insert("\n"),
            KeyCode::Left => {
                self.cursor = self.previous();
                self.preferred_column = None;
            }
            KeyCode::Right => {
                self.cursor = self.next();
                self.preferred_column = None;
            }
            KeyCode::Up => self.vertical(false),
            KeyCode::Down => self.vertical(true),
            KeyCode::Home => {
                self.cursor -= self.text[..self.cursor]
                    .rsplit('\n')
                    .next()
                    .unwrap_or("")
                    .len();
                self.preferred_column = None;
            }
            KeyCode::End => {
                self.cursor += self.text[self.cursor..]
                    .split('\n')
                    .next()
                    .unwrap_or("")
                    .len();
                self.preferred_column = None;
            }
            KeyCode::Backspace if self.cursor > 0 => {
                let previous = self.previous();
                self.text.replace_range(previous..self.cursor, "");
                self.cursor = previous;
                self.preferred_column = None;
            }
            KeyCode::Delete => {
                let next = self.next();
                self.text.replace_range(self.cursor..next, "");
                self.preferred_column = None;
            }
            _ => (),
        }
    }
    /// Unwrapped viewport keeps wide and combining graphemes aligned with the
    /// cursor, including when the draft is wider/taller than its panel.
    pub fn viewport(&self, width: usize, height: usize) -> (String, u16, u16) {
        if width == 0 || height == 0 {
            return (String::new(), 0, 0);
        }
        let (row, col) = self.position();
        let top = row.saturating_sub(height - 1);
        let left = col.saturating_sub(width - 1);
        let mut actual_left = left;
        let lines: Vec<_> = self
            .text
            .split('\n')
            .skip(top)
            .take(height)
            .enumerate()
            .map(|(index, line)| {
                let mut at = 0;
                let mut out = String::new();
                for grapheme in line.graphemes(true) {
                    let next = at + grapheme.width();
                    if at < left && next > left {
                        if index + top == row {
                            actual_left = next;
                        }
                    } else if at >= left && next <= left + width {
                        out.push_str(grapheme);
                    }
                    at = next;
                }
                out
            })
            .collect();
        (
            lines.join("\n"),
            col.saturating_sub(actual_left).min(width - 1) as u16,
            (row - top) as u16,
        )
    }
}

//! Grapheme-aware editing. Byte indices never escape this module.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, Default)]
pub struct Editor {
    pub text: String,
    cursor: usize,
    preferred_column: Option<usize>,
    wrap_width: Option<usize>,
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
    pub fn set_wrap_width(&mut self, width: usize) {
        let width = Some(width.max(1));
        if self.wrap_width != width {
            self.preferred_column = None;
            self.wrap_width = width;
        }
    }
    fn wrapped_lines(&self, width: usize) -> Vec<std::ops::Range<usize>> {
        let mut lines = Vec::new();
        let mut offset = 0;
        for paragraph in self.text.split('\n') {
            let mut start = 0;
            loop {
                let mut end = start;
                let mut used = 0;
                for g in paragraph[start..].graphemes(true) {
                    if used + g.width() > width && end > start {
                        break;
                    }
                    end += g.len();
                    used += g.width();
                    if used >= width {
                        break;
                    }
                }
                if end < paragraph.len() {
                    // Prefer a word boundary. Spaces at a soft break belong to
                    // the preceding row, so indentation and byte offsets survive.
                    if let Some((i, g)) = paragraph[start..end]
                        .grapheme_indices(true)
                        .rev()
                        .find(|(i, g)| *i > 0 && g.chars().all(char::is_whitespace))
                    {
                        end = start + i + g.len();
                    }
                    for g in paragraph[end..].graphemes(true) {
                        if !g.chars().all(char::is_whitespace) {
                            break;
                        }
                        end += g.len();
                    }
                }
                lines.push(offset + start..offset + end);
                if end == paragraph.len() {
                    // Leave a cell for the insertion cursor after a full row.
                    if paragraph[start..end].width() >= width {
                        lines.push(offset + end..offset + end);
                    }
                    break;
                }
                start = end;
            }
            offset += paragraph.len() + 1;
        }
        lines
    }
    fn wrapped_position(&self, lines: &[std::ops::Range<usize>]) -> (usize, usize) {
        let row = lines
            .iter()
            .rposition(|line| line.start <= self.cursor)
            .unwrap_or(0);
        (row, self.text[lines[row].start..self.cursor].width())
    }
    pub fn on_first_row(&self) -> bool {
        self.wrap_width.map_or_else(
            || self.position().0 == 0,
            |width| self.wrapped_position(&self.wrapped_lines(width)).0 == 0,
        )
    }
    pub fn on_last_row(&self) -> bool {
        self.wrap_width.map_or_else(
            || self.position().0 == self.text.matches('\n').count(),
            |width| {
                let lines = self.wrapped_lines(width);
                self.wrapped_position(&lines).0 == lines.len() - 1
            },
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
        if let Some(width) = self.wrap_width {
            let lines = self.wrapped_lines(width);
            let (row, col) = self.wrapped_position(&lines);
            let desired = self.preferred_column.unwrap_or(col.min(width - 1));
            let target = if down {
                (row + 1).min(lines.len() - 1)
            } else {
                row.saturating_sub(1)
            };
            if target != row {
                let line = &lines[target];
                self.cursor = line.start;
                let mut used = 0;
                for g in self.text[line.clone()].graphemes(true) {
                    if used + g.width() > desired
                        || (self.cursor + g.len() == line.end
                            && lines
                                .get(target + 1)
                                .is_some_and(|next| next.start == line.end))
                    {
                        break;
                    }
                    used += g.width();
                    self.cursor += g.len();
                }
            }
            self.preferred_column = Some(desired);
            return;
        }
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
    /// Word-wrapped viewport scrolls vertically to keep the cursor visible.
    pub fn viewport(&self, width: usize, height: usize) -> (String, u16, u16) {
        if width == 0 || height == 0 {
            return (String::new(), 0, 0);
        }
        let lines = self.wrapped_lines(width);
        let (row, col) = self.wrapped_position(&lines);
        let top = row.saturating_sub(height - 1);
        let visible: Vec<_> = lines
            .iter()
            .skip(top)
            .take(height)
            .map(|line| {
                let mut used = 0;
                let mut out = String::new();
                for g in self.text[line.clone()].graphemes(true) {
                    if used + g.width() > width {
                        break;
                    }
                    out.push_str(g);
                    used += g.width();
                }
                out
            })
            .collect();
        (
            visible.join("\n"),
            col.min(width - 1) as u16,
            (row - top) as u16,
        )
    }
}

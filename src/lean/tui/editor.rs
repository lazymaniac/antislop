//! Input buffer and text editing abstractions for chat and comment drafts.

use super::view::clean;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub const MAX_PASTE_BYTES: usize = 64 * 1024;

/// Normalizes paste text: replaces `\r\n` then standalone `\r` with `\n`,
/// caps length at 64 KB, and sanitizes via `clean()`.
pub fn sanitize_paste(raw: &str) -> String {
    let mut normalized = raw.replace("\r\n", "\n").replace('\r', "\n");
    if normalized.len() > MAX_PASTE_BYTES {
        let mut end = MAX_PASTE_BYTES;
        while end > 0 && !normalized.is_char_boundary(end) {
            end -= 1;
        }
        normalized.truncate(end);
    }
    clean(&normalized)
}

/// Normalizes single-line paste (e.g. for palette / picker search queries):
/// flattens newlines to single spaces and trims whitespace.
pub fn sanitize_single_line_paste(raw: &str) -> String {
    let text = sanitize_paste(raw);
    text.replace('\n', " ").trim().to_string()
}

/// Metadata about one visual line for cursor tracking and navigation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisualLineInfo {
    pub start_byte: usize,
    pub end_byte: usize,
    /// Vector of (char_byte_offset, visual_col_start, char_width)
    pub chars: Vec<(usize, u16, u16)>,
    pub width: u16,
}

/// Computes visual line wrapping conforming to `view::wrap()`.
pub fn compute_visual_lines(text: &str, width: usize) -> Vec<VisualLineInfo> {
    let width = width.max(1);
    let mut lines = vec![];

    if text.is_empty() {
        lines.push(VisualLineInfo {
            start_byte: 0,
            end_byte: 0,
            chars: vec![],
            width: 0,
        });
        return lines;
    }

    let len = text.len();
    let mut p_start = 0;

    while p_start <= len {
        let p_end = text[p_start..]
            .find('\n')
            .map(|idx| p_start + idx)
            .unwrap_or(len);
        let paragraph = &text[p_start..p_end];

        let mut current_line_start = p_start;
        let mut current_chars = vec![];
        let mut used: u16 = 0;

        let mut w_rel_start = 0;
        while w_rel_start < paragraph.len() {
            let w_rel_end = match paragraph[w_rel_start..].find(' ') {
                Some(idx) => w_rel_start + idx + 1,
                None => paragraph.len(),
            };
            let word = &paragraph[w_rel_start..w_rel_end];
            let word_abs_start = p_start + w_rel_start;
            let word_width = word.width();

            if used > 0 && (used as usize) + word_width.min(width) > width {
                lines.push(VisualLineInfo {
                    start_byte: current_line_start,
                    end_byte: word_abs_start,
                    chars: std::mem::take(&mut current_chars),
                    width: used,
                });
                current_line_start = word_abs_start;
                used = 0;
            }

            for (g_rel, g) in word.grapheme_indices(true) {
                let g_abs_start = word_abs_start + g_rel;
                let size = g.width() as u16;
                if used > 0 && (used as usize) + (size as usize) > width {
                    lines.push(VisualLineInfo {
                        start_byte: current_line_start,
                        end_byte: g_abs_start,
                        chars: std::mem::take(&mut current_chars),
                        width: used,
                    });
                    current_line_start = g_abs_start;
                    used = 0;
                }

                current_chars.push((g_abs_start, used, size));
                used += size;
            }

            w_rel_start = w_rel_end;
        }

        lines.push(VisualLineInfo {
            start_byte: current_line_start,
            end_byte: p_end,
            chars: std::mem::take(&mut current_chars),
            width: used,
        });

        if p_end == len {
            break;
        }
        p_start = p_end + 1;
        if p_start == len && text.ends_with('\n') {
            lines.push(VisualLineInfo {
                start_byte: len,
                end_byte: len,
                chars: vec![],
                width: 0,
            });
            break;
        }
    }

    lines
}

/// Returns the visual text lines corresponding to `compute_visual_lines`.
pub fn visual_text_lines(text: &str, width: usize) -> Vec<String> {
    let lines = compute_visual_lines(text, width);
    lines
        .into_iter()
        .map(|l| text[l.start_byte..l.end_byte].to_string())
        .collect()
}

/// An editable text buffer tracking content and cursor byte offset.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InputBuffer {
    pub text: String,
    pub cursor: usize, // Byte offset in text
    pub preferred_col: Option<u16>,
}

impl InputBuffer {
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

    fn clamp_cursor(&mut self) {
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

    // Navigation actions

    pub fn move_left(&mut self) {
        self.preferred_col = None;
        self.clamp_cursor();
        self.cursor = self.text[..self.cursor]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(i, _)| i);
    }

    pub fn move_right(&mut self) {
        self.preferred_col = None;
        self.clamp_cursor();
        self.cursor = self.text[self.cursor..]
            .graphemes(true)
            .next()
            .map_or(self.cursor, |g| self.cursor + g.len());
    }

    pub fn move_word_left(&mut self) {
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
        if graphemes.is_empty() {
            self.cursor = 0;
            return;
        }
        // Skip non-spaces
        let mut word_start = 0;
        while let Some((idx, g)) = graphemes.last() {
            if !g.chars().all(char::is_whitespace) {
                word_start = *idx;
                graphemes.pop();
            } else {
                break;
            }
        }
        self.cursor = word_start;
    }

    pub fn move_word_right(&mut self) {
        self.preferred_col = None;
        self.clamp_cursor();
        if self.cursor >= self.text.len() {
            return;
        }
        let after = &self.text[self.cursor..];
        let mut graphemes = after.grapheme_indices(true).peekable();
        // Skip non-spaces
        while let Some((_, g)) = graphemes.peek() {
            if !g.chars().all(char::is_whitespace) {
                graphemes.next();
            } else {
                break;
            }
        }
        // Skip spaces
        while let Some((_, g)) = graphemes.peek() {
            if g.chars().all(char::is_whitespace) {
                graphemes.next();
            } else {
                break;
            }
        }
        if let Some((idx, _)) = graphemes.peek() {
            self.cursor += idx;
        } else {
            self.cursor = self.text.len();
        }
    }

    pub fn move_home(&mut self) {
        self.preferred_col = None;
        self.clamp_cursor();
        let before = &self.text[..self.cursor];
        self.cursor = before.rfind('\n').map(|idx| idx + 1).unwrap_or(0);
    }

    pub fn move_end(&mut self) {
        self.preferred_col = None;
        self.clamp_cursor();
        let after = &self.text[self.cursor..];
        self.cursor = after
            .find('\n')
            .map(|idx| self.cursor + idx)
            .unwrap_or(self.text.len());
    }

    pub fn move_buffer_start(&mut self) {
        self.preferred_col = None;
        self.cursor = 0;
    }

    pub fn move_buffer_end(&mut self) {
        self.preferred_col = None;
        self.cursor = self.text.len();
    }

    pub fn move_up(&mut self, width: usize) {
        self.clamp_cursor();
        let (current_col, current_row) = self.visual_cursor(width);
        if current_row == 0 {
            return;
        }
        let target_col = self.preferred_col.unwrap_or(current_col);
        self.preferred_col = Some(target_col);
        let lines = compute_visual_lines(&self.text, width);
        let target_row = current_row - 1;
        let target_line = &lines[target_row];
        self.cursor = Self::find_byte_for_col(target_line, target_col, &self.text);
    }

    pub fn move_down(&mut self, width: usize) {
        self.clamp_cursor();
        let (current_col, current_row) = self.visual_cursor(width);
        let lines = compute_visual_lines(&self.text, width);
        if current_row + 1 >= lines.len() {
            return;
        }
        let target_col = self.preferred_col.unwrap_or(current_col);
        self.preferred_col = Some(target_col);
        let target_row = current_row + 1;
        let target_line = &lines[target_row];
        self.cursor = Self::find_byte_for_col(target_line, target_col, &self.text);
    }

    fn find_byte_for_col(line: &VisualLineInfo, target_col: u16, text: &str) -> usize {
        if line.chars.is_empty() || target_col == 0 {
            return line.start_byte;
        }
        let is_soft_wrapped = line.end_byte < text.len() && text.as_bytes()[line.end_byte] != b'\n';
        if target_col >= line.width {
            if is_soft_wrapped {
                return line
                    .chars
                    .last()
                    .map(|&(b, _, _)| b)
                    .unwrap_or(line.start_byte);
            } else {
                return line.end_byte;
            }
        }
        let mut best_byte = line.start_byte;
        let mut min_diff = u16::MAX;
        for &(ch_byte, ch_col, _ch_width) in &line.chars {
            if ch_col <= target_col && target_col < ch_col + _ch_width {
                return ch_byte;
            }
            let diff = target_col.abs_diff(ch_col);
            if diff < min_diff {
                min_diff = diff;
                best_byte = ch_byte;
            }
        }
        best_byte
    }

    // Cursor visual projection

    // Rows index the whole draft, which can exceed the terminal's u16 range.
    pub fn visual_cursor(&self, width: usize) -> (u16, usize) {
        let lines = compute_visual_lines(&self.text, width);
        let cursor = self.cursor.min(self.text.len());

        for (row, line) in lines.iter().enumerate() {
            let is_last = row + 1 == lines.len();
            let on_this_line = if is_last {
                true
            } else {
                let next_start = lines[row + 1].start_byte;
                if line.end_byte < next_start {
                    // Line ends with \n; cursor at end_byte belongs to this line (before \n)
                    cursor <= line.end_byte
                } else {
                    // Soft-wrapped line; next_start is the first char of next line
                    cursor < next_start
                }
            };

            if on_this_line {
                if cursor <= line.start_byte {
                    return (0, row);
                }
                if cursor >= line.end_byte {
                    return (line.width, row);
                }
                for &(ch_byte, ch_col, _ch_width) in &line.chars {
                    if cursor == ch_byte {
                        return (ch_col, row);
                    }
                    if cursor > ch_byte {
                        let g_len = self.text[ch_byte..]
                            .graphemes(true)
                            .next()
                            .map_or(1, |g| g.len());
                        if cursor < ch_byte + g_len {
                            return (ch_col, row);
                        }
                    }
                }
                return (line.width, row);
            }
        }

        (0, 0)
    }

    pub fn visual_line_count(&self, width: usize) -> usize {
        compute_visual_lines(&self.text, width).len()
    }

    pub fn on_first_row(&self, width: usize) -> bool {
        self.visual_cursor(width).1 == 0
    }

    pub fn on_last_row(&self, width: usize) -> bool {
        let (_, row) = self.visual_cursor(width);
        let lines = self.visual_line_count(width);
        row + 1 >= lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ac_edit_01_cursor_insertion_in_middle_of_text() {
        let mut buf = InputBuffer::with_text("ac");
        buf.cursor = 1; // At 'c'
        buf.insert_char('b');
        assert_eq!(buf.text, "abc");
        assert_eq!(buf.cursor, 2);
    }

    #[test]
    fn ac_edit_02_backspace_in_middle_of_text() {
        let mut buf = InputBuffer::with_text("abc");
        buf.cursor = 2; // At 'c'
        assert!(buf.backspace());
        assert_eq!(buf.text, "ac");
        assert_eq!(buf.cursor, 1);
    }

    #[test]
    fn ac_edit_03_word_jump_navigation() {
        let mut buf = InputBuffer::with_text("hello beautiful world");
        assert_eq!(buf.cursor, 21);
        buf.move_word_left();
        assert_eq!(buf.cursor, 16); // 'w' in "world"
        buf.move_word_left();
        assert_eq!(buf.cursor, 6); // 'b' in "beautiful"
        buf.move_word_left();
        assert_eq!(buf.cursor, 0); // 'h' in "hello"
        buf.move_word_right();
        assert_eq!(buf.cursor, 6); // past "hello " to 'b'
        buf.move_word_right();
        assert_eq!(buf.cursor, 16); // past "beautiful " to 'w'
        buf.move_word_right();
        assert_eq!(buf.cursor, 21);
    }

    #[test]
    fn ac_edit_04_multiline_vertical_navigation() {
        let mut buf = InputBuffer::with_text("line1\nline2");
        buf.cursor = 2; // 'n' of line1
        buf.move_down(20);
        assert_eq!(buf.cursor, 8); // 'n' of line2 (6 + 2)
        assert_eq!(buf.visual_cursor(20), (2, 1));
        buf.move_up(20);
        assert_eq!(buf.cursor, 2); // 'n' of line1
        assert_eq!(buf.visual_cursor(20), (2, 0));
    }

    #[test]
    fn ac_edit_05_visual_cursor_boundary_tracking() {
        // Text wrapping across 3 lines at width 20:
        // "1234567890 12345678 " (20 cols) -> line 0
        // "1234567890 12345678 " (20 cols) -> line 1
        // "last line" -> line 2
        let text = "1234567890 12345678 1234567890 12345678 last line";
        let mut buf = InputBuffer::with_text(text);
        assert_eq!(buf.visual_line_count(20), 3);

        // Put cursor in line 1: index 25 is '5' in the second segment
        buf.cursor = 25;
        let (col, row) = buf.visual_cursor(20);
        assert_eq!(row, 1);
        assert_eq!(col, 5);
    }

    #[test]
    fn ac_edit_06_polish_diacritic_character_editing() {
        let mut buf = InputBuffer::with_text("zółw");
        // 'z' is 1 byte (idx 0)
        // 'ó' is 2 bytes (idx 1..3)
        // 'ł' is 2 bytes (idx 3..5)
        // 'w' is 1 byte (idx 5..6)
        buf.cursor = 3; // between 'ó' and 'ł'
        assert!(buf.backspace());
        assert_eq!(buf.text, "złw");
        assert_eq!(buf.cursor, 1);
    }

    #[test]
    fn ac_paste_01_crlf_newline_normalization() {
        let pasted = sanitize_paste("a\r\nb\r\nc");
        assert_eq!(pasted, "a\nb\nc");
        let lone_cr = sanitize_paste("x\ry\rz");
        assert_eq!(lone_cr, "x\ny\nz");
    }

    #[test]
    fn ac_paste_02_middle_of_line_paste_insertion() {
        let mut buf = InputBuffer::with_text("ab");
        buf.cursor = 1;
        buf.insert_str("XYZ");
        assert_eq!(buf.text, "aXYZb");
        assert_eq!(buf.cursor, 4);
    }

    #[test]
    fn ac_paste_03_single_line_search_query_paste() {
        let sanitized = sanitize_single_line_paste("alpha\nbeta\r\ngamma");
        assert_eq!(sanitized, "alpha beta gamma");
    }

    #[test]
    fn ac_key_04_home_end_line_navigation() {
        let mut buf = InputBuffer::with_text("line1\nline2");
        buf.cursor = 8; // In middle of line 2 ('n')
        buf.move_home();
        assert_eq!(buf.cursor, 6); // start of line 2
        buf.move_end();
        assert_eq!(buf.cursor, 11); // end of line 2
    }

    #[test]
    fn ac_uni_02_double_width_cjk_width_calculation() {
        let buf = InputBuffer::with_text("漢");
        // '漢' is 3 bytes in UTF-8, width 2
        assert_eq!(buf.cursor, 3);
        let (col, row) = buf.visual_cursor(10);
        assert_eq!((col, row), (2, 0));
    }

    #[test]
    fn ac_uni_03_combining_character_safety() {
        // Base 'e' + combining acute '\u{0301}' forms a single grapheme cluster
        let mut buf = InputBuffer::with_text("e\u{0301}");
        assert_eq!(buf.len(), 3);
        assert!(buf.backspace());
        assert_eq!(buf.text, "");
        assert_eq!(buf.cursor, 0);
    }

    #[test]
    fn ac_uni_04_atomic_double_width_wrap() {
        // 1 column left on row, next char is '漢' (width 2)
        // With width 2: 'a' has width 1, so 1 column remains.
        // Next char '漢' must wrap atomically to the next line.
        let lines = compute_visual_lines("a漢", 2);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].width, 1);
        assert_eq!(lines[1].width, 2);
    }

    #[test]
    fn delete_and_kill_actions() {
        let mut buf = InputBuffer::with_text("hello world");
        buf.cursor = 5; // at space
        buf.delete_to_end();
        assert_eq!(buf.text, "hello");
        buf.cursor = 3;
        buf.delete_to_start();
        assert_eq!(buf.text, "lo");

        let mut buf2 = InputBuffer::with_text("first second third");
        buf2.cursor = 18; // end
        buf2.delete_word_back();
        assert_eq!(buf2.text, "first second ");
        assert_eq!(buf2.cursor, 13);
    }

    #[test]
    fn move_word_left_on_leading_whitespace_reaches_zero() {
        let mut buf = InputBuffer::with_text("   hello");
        buf.cursor = 2; // in leading spaces
        buf.move_word_left();
        assert_eq!(buf.cursor, 0);

        let mut buf2 = InputBuffer::with_text("   hello");
        buf2.cursor = 8; // end of hello
        buf2.move_word_left();
        assert_eq!(buf2.cursor, 3);
        buf2.move_word_left();
        assert_eq!(buf2.cursor, 0);
    }

    #[test]
    fn soft_wrapped_vertical_navigation_preserves_lines() {
        let text = "1234567890 12345678 1234567890 12345678 last line";
        let mut buf = InputBuffer::with_text(text);
        assert_eq!(buf.visual_line_count(20), 3);

        // Put cursor in line 1 at the end (col >= line.width)
        buf.cursor = 39;
        assert_eq!(buf.visual_cursor(20).1, 1);

        // move_up must move to line 0, not stay on line 1
        buf.move_up(20);
        assert_eq!(buf.visual_cursor(20).1, 0);

        // move_down must move back to line 1, not skip to line 2
        buf.move_down(20);
        assert_eq!(buf.visual_cursor(20).1, 1);

        // move_down again moves to line 2
        buf.move_down(20);
        assert_eq!(buf.visual_cursor(20).1, 2);
    }

    #[test]
    fn grapheme_zwj_and_combining_characters_editing() {
        let mut buf = InputBuffer::new();
        // Woman technologist: 👩‍💻 (\u{1F469}\u{200D}\u{1F4BB}) = 10 bytes
        let emoji = "👩‍💻";
        buf.insert_str(emoji);
        assert_eq!(buf.text, emoji);
        assert_eq!(buf.cursor, emoji.len());

        // Backspace removes the ENTIRE 10-byte ZWJ sequence in one press
        assert!(buf.backspace());
        assert_eq!(buf.text, "");
        assert_eq!(buf.cursor, 0);

        // Combining acute accent: e + \u{301}
        buf.insert_str("e\u{301}");
        assert_eq!(buf.cursor, "e\u{301}".len());
        // Left arrow moves past the whole combining character
        buf.move_left();
        assert_eq!(buf.cursor, 0);
        // Delete removes the whole combining character
        assert!(buf.delete());
        assert_eq!(buf.text, "");

        // Family emoji: 👨‍👩‍👧‍👦 = 25 bytes
        let family = "👨‍👩‍👧‍👦";
        buf.insert_str("start ");
        buf.insert_str(family);
        buf.insert_str(" end");
        buf.cursor = "start ".len() + family.len();
        buf.move_left();
        assert_eq!(buf.cursor, "start ".len());
        buf.move_right();
        assert_eq!(buf.cursor, "start ".len() + family.len());
    }

    #[test]
    fn preferred_column_memory_across_short_lines() {
        // Line 0: "0123456789" (10 chars, cols 0..10)
        // Line 1: "01" (2 chars, cols 0..2)
        // Line 2: "0123456789" (10 chars, cols 0..10)
        let text = "0123456789\n01\n0123456789";
        let mut buf = InputBuffer::with_text(text);
        buf.cursor = 8; // On '8' in line 0 (col 8, row 0)
        assert_eq!(buf.visual_cursor(20), (8, 0));

        // Move down to Line 1 (short line with max col 2)
        buf.move_down(20);
        assert_eq!(buf.visual_cursor(20), (2, 1));
        assert_eq!(buf.preferred_col, Some(8));

        // Move down to Line 2 (longer line: should jump back to col 8!)
        buf.move_down(20);
        assert_eq!(buf.visual_cursor(20), (8, 2));
        assert_eq!(buf.preferred_col, Some(8));

        // Move up back to Line 1 (clamped to col 2)
        buf.move_up(20);
        assert_eq!(buf.visual_cursor(20), (2, 1));
        assert_eq!(buf.preferred_col, Some(8));

        // Move up to Line 0 (should jump back to col 8!)
        buf.move_up(20);
        assert_eq!(buf.visual_cursor(20), (8, 0));
        assert_eq!(buf.preferred_col, Some(8));

        // Horizontal movement resets preferred col
        buf.move_left();
        assert_eq!(buf.visual_cursor(20), (7, 0));
        assert_eq!(buf.preferred_col, None);

        // Subsequent move down now uses col 7
        buf.move_down(20);
        assert_eq!(buf.visual_cursor(20), (2, 1));
        assert_eq!(buf.preferred_col, Some(7));

        buf.move_down(20);
        assert_eq!(buf.visual_cursor(20), (7, 2));
    }

    #[test]
    fn preferred_column_with_graphemes_and_double_width() {
        // Line 0: "zażółć 12345" -> col 8 is '2'
        // Line 1: "漢" (width 2)
        // Line 2: "zażółć 12345"
        let text = "zażółć 12345\n漢\nzażółć 12345";
        let mut buf = InputBuffer::with_text(text);
        // Find byte offset corresponding to visual col 8 in line 0
        buf.cursor = 0;
        while buf.visual_cursor(20).0 < 8 {
            buf.move_right();
        }
        assert_eq!(buf.visual_cursor(20), (8, 0));

        // Move down to Line 1 (clamped to CJK width 2)
        buf.move_down(20);
        assert_eq!(buf.visual_cursor(20), (2, 1));
        assert_eq!(buf.preferred_col, Some(8));

        // Move down to Line 2 (restores visual col 8)
        buf.move_down(20);
        assert_eq!(buf.visual_cursor(20), (8, 2));
        assert_eq!(buf.preferred_col, Some(8));
    }
}

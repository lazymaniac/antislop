use super::lines::{VisualLineInfo, compute_visual_lines};
use crate::lean::tui::buffer::InputBuffer;
use unicode_segmentation::UnicodeSegmentation;

impl InputBuffer {
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
            if Self::cursor_on_line(&lines, row, cursor) {
                return Self::cursor_column(line, cursor, &self.text, row);
            }
        }
        (0, 0)
    }

    pub fn visual_line_count(&self, width: usize) -> usize {
        compute_visual_lines(&self.text, width).len()
    }

    fn cursor_on_line(lines: &[VisualLineInfo], row: usize, cursor: usize) -> bool {
        let line = &lines[row];
        if row + 1 == lines.len() {
            return true;
        }
        let next_start = lines[row + 1].start_byte;
        if line.end_byte < next_start {
            cursor <= line.end_byte
        } else {
            cursor < next_start
        }
    }

    fn cursor_column(line: &VisualLineInfo, cursor: usize, text: &str, row: usize) -> (u16, usize) {
        if cursor <= line.start_byte {
            return (0, row);
        }
        if cursor >= line.end_byte {
            return (line.width, row);
        }
        for &(byte, column, _) in &line.chars {
            if cursor == byte || Self::inside_grapheme(cursor, byte, text) {
                return (column, row);
            }
        }
        (line.width, row)
    }

    fn inside_grapheme(cursor: usize, byte: usize, text: &str) -> bool {
        let length = text[byte..].graphemes(true).next().map_or(1, str::len);
        cursor > byte && cursor < byte + length
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

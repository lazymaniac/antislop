use crate::Result;
use unicode_segmentation::UnicodeSegmentation;
use wezterm_term::Terminal;

mod cluster;
use cluster::{ClusterContinuation, Tail, written_position};

/// wezterm flushes its printable run at each advance_bytes call. Retain the
/// last extended cluster across reads and update its cell when a combining
/// mark or ZWJ continuation arrives, including after a screen was published.
#[derive(Default)]
pub(super) struct GraphemeStream {
    mode: u8,
    utf8: Vec<u8>,
    tail: Option<Tail>,
    dimensions: Option<(usize, usize)>,
}

impl GraphemeStream {
    pub(super) fn feed(&mut self, term: &mut Terminal, bytes: &[u8]) -> Result<()> {
        self.sync_dimensions(term);
        let mut index = 0;
        while index < bytes.len() {
            index = self.feed_next(term, bytes, index)?;
        }
        Ok(())
    }

    fn sync_dimensions(&mut self, term: &Terminal) {
        let size = term.get_size();
        let dimensions = (size.cols, size.rows);
        if self.dimensions.is_some_and(|old| old != dimensions) {
            self.relocate_tail(term, size.cols, size.rows);
        }
        self.dimensions = Some(dimensions);
    }

    fn relocate_tail(&mut self, term: &Terminal, columns: usize, rows: usize) {
        let Some(tail) = &mut self.tail else {
            return;
        };
        let cursor = term.cursor_pos();
        let top = term.screen().scrollback_or_visible_row(0);
        tail.position = term
            .screen()
            .lines_in_phys_range(top..top + rows)
            .iter()
            .enumerate()
            .flat_map(|(y, line)| {
                line.visible_cells()
                    .filter(|cell| cell.str() == tail.text)
                    .map(move |cell| (cell.cell_index(), y as i64))
            })
            .min_by_key(|(x, y)| {
                y.abs_diff(cursor.y) as usize * columns + (x + tail.width).abs_diff(cursor.x)
            });
        if tail.position.is_none() {
            self.tail = None;
        }
    }

    fn feed_next(&mut self, term: &mut Terminal, bytes: &[u8], index: usize) -> Result<usize> {
        let byte = bytes[index];
        if self.mode != 0 {
            self.advance_control_sequence(term, byte);
            return Ok(index + 1);
        }
        if byte < 0x20 || byte == 0x7f {
            self.feed_control(term, byte)?;
            return Ok(index + 1);
        }
        if self.utf8.is_empty() && byte.is_ascii() {
            return self.feed_ascii(term, bytes, index);
        }
        self.utf8.push(byte);
        self.consume_utf8(term)?;
        Ok(index + 1)
    }

    fn advance_control_sequence(&mut self, term: &mut Terminal, byte: u8) {
        let mode = self.mode;
        term.advance_bytes([byte]);
        self.mode = match mode {
            1 if byte == b'[' => 2,
            1 if byte == b']' => 3,
            1 if (0x20..=0x2f).contains(&byte) => 1,
            3 if byte == 7 => 0,
            3 if byte == 27 => 4,
            4 if byte == b'\\' => 0,
            4 => 3,
            2 if (0x40..=0x7e).contains(&byte) => 0,
            5 if (0x40..=0x7e).contains(&byte) => 0,
            1 | 3 | 2 | 5 => mode,
            _ => 0,
        };
    }

    fn feed_control(&mut self, term: &mut Terminal, byte: u8) -> Result<()> {
        if !self.utf8.is_empty() {
            self.utf8.clear();
            self.character(term, '\u{fffd}')?;
        }
        self.tail = None;
        term.advance_bytes([byte]);
        if byte == 27 {
            self.mode = 1;
        }
        Ok(())
    }

    fn feed_ascii(&mut self, term: &mut Terminal, bytes: &[u8], index: usize) -> Result<usize> {
        let end = index
            + bytes[index..]
                .iter()
                .take_while(|b| **b >= 0x20 && **b < 0x7f)
                .count();
        let run = &bytes[index..end];
        let start = self.join_ascii_prefix(term, run)?;
        self.write_ascii_suffix(term, &run[start..])?;
        Ok(end)
    }

    fn join_ascii_prefix(&mut self, term: &mut Terminal, run: &[u8]) -> Result<usize> {
        if !run.is_empty()
            && self.tail.as_ref().is_some_and(|tail| {
                format!("{}{}", tail.text, run[0] as char)
                    .graphemes(true)
                    .count()
                    == 1
            })
        {
            self.character(term, run[0] as char)?;
            return Ok(1);
        }
        Ok(0)
    }

    fn write_ascii_suffix(&mut self, term: &mut Terminal, rest: &[u8]) -> Result<()> {
        if rest.is_empty() {
            return Ok(());
        }
        if rest.len() > 1 {
            self.tail = None;
            term.advance_bytes(&rest[..rest.len() - 1]);
        }
        self.character(term, rest[rest.len() - 1] as char)
    }

    fn consume_utf8(&mut self, term: &mut Terminal) -> Result<()> {
        loop {
            match std::str::from_utf8(&self.utf8) {
                Ok(text) => {
                    let text = text.to_owned();
                    self.utf8.clear();
                    for ch in text.chars() {
                        self.character(term, ch)?;
                    }
                    return Ok(());
                }
                Err(error) if error.error_len().is_none() => return Ok(()),
                Err(error) => {
                    let remove =
                        (error.valid_up_to() + error.error_len().unwrap_or(1)).min(self.utf8.len());
                    self.utf8.drain(..remove);
                    self.character(term, '\u{fffd}')?;
                    if self.utf8.is_empty() {
                        return Ok(());
                    }
                }
            }
        }
    }

    fn character(&mut self, term: &mut Terminal, ch: char) -> Result<()> {
        let text = match self.continue_cluster(term, ch)? {
            ClusterContinuation::Updated => return Ok(()),
            ClusterContinuation::Write(text) => text,
        };
        self.write_character(term, text)
    }

    fn write_character(&mut self, term: &mut Terminal, text: String) -> Result<()> {
        let width = wezterm_term::grapheme_column_width(
            &text,
            Some(&wezterm_term::UnicodeVersion::new(14)),
        );
        let before = term.cursor_pos();
        if width > 0 {
            term.advance_bytes(text.as_bytes());
        }
        let after = term.cursor_pos();
        let position = written_position(term, &text, width, before, after);
        self.tail = Some(Tail {
            text,
            position,
            width,
        });
        Ok(())
    }
}

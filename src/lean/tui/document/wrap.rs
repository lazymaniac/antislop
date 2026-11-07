use super::chunk::read_utf8_chunk;
use crate::Result;
use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Weak;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// A line index records byte ranges only. Last grapheme is carried between
/// chunks, including ZWJ sequences, combining marks and regional indicators.
pub(super) fn wrap_range(
    path: &Path,
    output: &mut File,
    start: u64,
    end: u64,
    width: usize,
    dim: bool,
    alive: &Weak<()>,
) -> Result<usize> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(start))?;
    let mut state = WrapState::new(start, end);
    let mut index = LineIndex {
        output,
        dim,
        count: 0,
    };
    consume_file(&mut file, width, alive, &mut state, &mut index)?;
    if state.cursor.line < end {
        index.emit(state.cursor.line, end)?;
    }
    Ok(index.count)
}

struct WrapState {
    remaining: u64,
    pending: String,
    base: u64,
    cursor: Cursor,
}

/// The row being filled: where it starts, its width so far, and the end of
/// its last space after some text, where the row breaks before a word that
/// does not fit.
struct Cursor {
    line: u64,
    columns: usize,
    space: Option<(u64, usize)>,
    ink: bool,
}

impl Cursor {
    fn start(&mut self, line: u64) {
        self.line = line;
        self.columns = 0;
        self.space = None;
        self.ink = false;
    }
}

impl WrapState {
    fn new(start: u64, end: u64) -> Self {
        Self {
            remaining: end - start,
            pending: String::new(),
            base: start,
            cursor: Cursor {
                line: start,
                columns: 0,
                space: None,
                ink: false,
            },
        }
    }

    fn consume_ready(&mut self, width: usize, index: &mut LineIndex<'_>) -> Result<()> {
        let keep = ready_length(&self.pending, self.remaining == 0);
        index.consume(&self.pending[..keep], self.base, &mut self.cursor, width)?;
        self.pending.drain(..keep);
        self.base += keep as u64;
        Ok(())
    }
}

fn consume_file(
    file: &mut File,
    width: usize,
    alive: &Weak<()>,
    state: &mut WrapState,
    index: &mut LineIndex<'_>,
) -> Result<()> {
    loop {
        crate::require(alive.strong_count() > 0, "inspection view was closed")?;
        if state.remaining > 0 {
            state
                .pending
                .push_str(&read_utf8_chunk(file, &mut state.remaining)?);
        }
        state.consume_ready(width, index)?;
        if state.remaining == 0 {
            break;
        }
    }
    Ok(())
}

fn ready_length(pending: &str, complete: bool) -> usize {
    if complete {
        return pending.len();
    }
    pending
        .grapheme_indices(true)
        .next_back()
        .map_or(0, |(index, _)| index)
}

struct LineIndex<'a> {
    output: &'a mut File,
    dim: bool,
    count: usize,
}

impl LineIndex<'_> {
    fn emit(&mut self, start: u64, end: u64) -> Result<()> {
        self.output.write_all(&start.to_le_bytes())?;
        self.output.write_all(&end.to_le_bytes())?;
        self.output.write_all(&[u8::from(self.dim)])?;
        self.count += 1;
        Ok(())
    }

    fn consume(&mut self, text: &str, base: u64, cursor: &mut Cursor, width: usize) -> Result<()> {
        for (offset, grapheme) in text.grapheme_indices(true) {
            let position = base + offset as u64;
            if grapheme == "\n" || grapheme == "\r\n" {
                self.emit(cursor.line, position)?;
                cursor.start(position + grapheme.len() as u64);
            } else {
                self.wrap_grapheme(grapheme, position, cursor, width)?;
            }
        }
        Ok(())
    }

    /// Rows break at the last space before a word that overflows them, and
    /// inside a word only when it is wider than a row; a space at the break
    /// is not drawn on either row.
    fn wrap_grapheme(
        &mut self,
        grapheme: &str,
        position: u64,
        cursor: &mut Cursor,
        width: usize,
    ) -> Result<()> {
        let cells = UnicodeWidthStr::width(grapheme);
        let blank = grapheme.chars().all(char::is_whitespace);
        let end = position + grapheme.len() as u64;
        if cursor.columns > 0 && cursor.columns + cells > width {
            if blank {
                self.emit(cursor.line, position)?;
                cursor.start(end);
                return Ok(());
            }
            if let Some((space, columns)) = cursor.space.take() {
                self.emit(cursor.line, space)?;
                cursor.line = space;
                cursor.columns -= columns;
            }
            if cursor.columns > 0 && cursor.columns + cells > width {
                self.emit(cursor.line, position)?;
                cursor.start(position);
            }
        }
        cursor.columns += cells;
        if blank && cursor.ink {
            cursor.space = Some((end, cursor.columns));
        }
        cursor.ink |= !blank;
        Ok(())
    }
}

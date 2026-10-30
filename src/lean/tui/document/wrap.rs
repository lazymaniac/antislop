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
    if state.line < end {
        index.emit(state.line, end)?;
    }
    Ok(index.count)
}

struct WrapState {
    remaining: u64,
    pending: String,
    base: u64,
    line: u64,
    columns: usize,
}

impl WrapState {
    fn new(start: u64, end: u64) -> Self {
        Self {
            remaining: end - start,
            pending: String::new(),
            base: start,
            line: start,
            columns: 0,
        }
    }

    fn consume_ready(&mut self, width: usize, index: &mut LineIndex<'_>) -> Result<()> {
        let keep = ready_length(&self.pending, self.remaining == 0);
        index.consume(
            &self.pending[..keep],
            self.base,
            &mut self.line,
            &mut self.columns,
            width,
        )?;
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

    fn consume(
        &mut self,
        text: &str,
        base: u64,
        line: &mut u64,
        columns: &mut usize,
        width: usize,
    ) -> Result<()> {
        for (offset, grapheme) in text.grapheme_indices(true) {
            let position = base + offset as u64;
            if grapheme == "\n" || grapheme == "\r\n" {
                self.emit(*line, position)?;
                *line = position + grapheme.len() as u64;
                *columns = 0;
            } else {
                self.wrap_grapheme(grapheme, position, line, columns, width)?;
            }
        }
        Ok(())
    }

    fn wrap_grapheme(
        &mut self,
        grapheme: &str,
        position: u64,
        line: &mut u64,
        columns: &mut usize,
        width: usize,
    ) -> Result<()> {
        let cells = UnicodeWidthStr::width(grapheme);
        if *columns > 0 && *columns + cells > width {
            self.emit(*line, position)?;
            *line = position;
            *columns = 0;
        }
        *columns += cells;
        Ok(())
    }
}

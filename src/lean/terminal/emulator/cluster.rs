use super::GraphemeStream;
use crate::{Result, require};
use unicode_segmentation::UnicodeSegmentation;
use wezterm_term::Terminal;

pub(super) struct Tail {
    pub(super) text: String,
    pub(super) position: Option<(usize, i64)>,
    pub(super) width: usize,
}
pub(super) enum ClusterContinuation {
    Updated,
    Write(String),
}

impl GraphemeStream {
    pub(super) fn continue_cluster(
        &mut self,
        term: &mut Terminal,
        ch: char,
    ) -> Result<ClusterContinuation> {
        let Some(previous) = self.tail.take() else {
            return Ok(ClusterContinuation::Write(ch.to_string()));
        };
        let combined = format!("{}{ch}", previous.text);
        if combined.graphemes(true).count() != 1 {
            return Ok(ClusterContinuation::Write(ch.to_string()));
        }
        require(
            combined.len() <= 4096,
            "native terminal grapheme exceeds the 4 KiB cell limit",
        )?;
        self.reprint_cluster(term, previous, combined)
    }

    fn reprint_cluster(
        &mut self,
        term: &mut Terminal,
        previous: Tail,
        text: String,
    ) -> Result<ClusterContinuation> {
        let width = wezterm_term::grapheme_column_width(
            &text,
            Some(&wezterm_term::UnicodeVersion::new(14)),
        );
        if let Some((x, y)) = previous.position {
            if width == previous.width && self.replace_cluster(term, x, y, &text, width) {
                self.tail = Some(Tail {
                    text,
                    position: Some((x, y)),
                    width,
                });
                return Ok(ClusterContinuation::Updated);
            }
            term.advance_bytes(format!("\x1b[{};{}H", y + 1, x + 1));
        }
        Ok(ClusterContinuation::Write(text))
    }

    fn replace_cluster(
        &self,
        term: &mut Terminal,
        x: usize,
        y: i64,
        text: &str,
        width: usize,
    ) -> bool {
        let attributes = term
            .screen_mut()
            .get_cell(x, y)
            .map(|cell| cell.attrs().clone());
        let Some(attributes) = attributes else {
            return false;
        };
        term.increment_seqno();
        let sequence = term.current_seqno();
        term.screen_mut().set_cell(
            x,
            y,
            &wezterm_term::Cell::new_grapheme_with_width(text, width, attributes),
            sequence,
        );
        true
    }
}

pub(super) fn written_position(
    term: &mut Terminal,
    text: &str,
    width: usize,
    before: wezterm_term::CursorPosition,
    after: wezterm_term::CursorPosition,
) -> Option<(usize, i64)> {
    let columns = term.get_size().cols;
    let mut candidates = vec![];
    if before.y == after.y && after.x >= before.x && before.x + width <= columns {
        candidates.push(before.x);
    }
    candidates.extend([
        after.x.saturating_sub(width),
        (after.x + 1).saturating_sub(width),
    ]);
    if width == 0 {
        return None;
    }
    candidates
        .into_iter()
        .filter(|x| *x < columns)
        .find(|x| {
            term.screen_mut()
                .get_cell(*x, after.y)
                .is_some_and(|cell| cell.str() == text)
        })
        .map(|x| (x, after.y))
}

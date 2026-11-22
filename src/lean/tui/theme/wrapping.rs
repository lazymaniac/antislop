use crate::lean::tui::sanitize;
use std::collections::VecDeque;
use unicode_segmentation::GraphemeCursor;
use unicode_width::UnicodeWidthStr;

/// `Split` retains the final empty paragraph; `Lines` follows `str::lines`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::lean::tui) enum Paragraphs {
    Split,
    Lines,
}

/// A consumer can stop between rows. Lookahead covers only a complete grapheme,
/// a possible list marker, and enough of a word to decide whether it fits.
/// Escape payloads and individual graphemes can themselves be arbitrarily long.
pub(in crate::lean::tui) fn wrapped_rows<'a>(
    fragments: impl IntoIterator<Item = &'a str>,
    width: usize,
    paragraphs: Paragraphs,
) -> Rows<impl Iterator<Item = char>> {
    Rows {
        input: Graphemes {
            characters: sanitize::sanitized_characters(fragments),
            cluster: String::new(),
        },
        pending: VecDeque::new(),
        width: width.max(1),
        paragraphs,
        paragraph_start: true,
        word_start: true,
        indent: String::new(),
        done: false,
    }
}

struct Graphemes<I> {
    characters: I,
    cluster: String,
}

impl<I: Iterator<Item = char>> Iterator for Graphemes<I> {
    type Item = String;

    fn next(&mut self) -> Option<String> {
        for c in self.characters.by_ref() {
            let boundary = self.cluster.len();
            self.cluster.push(c);
            if boundary > 0
                && GraphemeCursor::new(boundary, self.cluster.len(), true)
                    .is_boundary(&self.cluster, 0)
                    .expect("complete grapheme context")
            {
                let next = self.cluster.split_off(boundary);
                return Some(std::mem::replace(&mut self.cluster, next));
            }
        }
        (!self.cluster.is_empty()).then(|| std::mem::take(&mut self.cluster))
    }
}

pub(in crate::lean::tui) struct Rows<I> {
    input: Graphemes<I>,
    pending: VecDeque<String>,
    width: usize,
    paragraphs: Paragraphs,
    paragraph_start: bool,
    word_start: bool,
    indent: String,
    done: bool,
}

impl<I: Iterator<Item = char>> Rows<I> {
    /// Checks continuation without materializing another row.
    pub(in crate::lean::tui) fn has_more(&mut self) -> bool {
        !self.done
            && !(self.paragraph_start
                && self.paragraphs == Paragraphs::Lines
                && self.peek(0).is_none())
    }

    fn peek(&mut self, index: usize) -> Option<&str> {
        while self.pending.len() <= index {
            self.pending.push_back(self.input.next()?);
        }
        self.pending.get(index).map(String::as_str)
    }

    fn hanging_indent(&mut self) -> String {
        let mut prefix = String::new();
        let mut index = 0;
        // A marker that cannot leave half the row for text is not an indent.
        while self.peek(index) == Some(" ") {
            prefix.push(' ');
            index += 1;
            if index > self.width / 2 {
                return String::new();
            }
        }
        let lead = prefix.len();
        while let Some(g) = self.peek(index) {
            if g == "\n" {
                break;
            }
            prefix.push_str(g);
            index += 1;
            if g.starts_with(' ') {
                return hanging_indent(&prefix)
                    .filter(|hang| *hang <= self.width / 2)
                    .map_or_else(String::new, |hang| " ".repeat(hang));
            }
            if prefix.len() - lead > 4 {
                break;
            }
        }
        String::new()
    }

    fn word_width(&mut self) -> usize {
        let mut word = String::new();
        let mut index = 0;
        while let Some(g) = self.peek(index) {
            if g == "\n" {
                break;
            }
            word.push_str(g);
            let last = g == " ";
            let width = word.width();
            // unicode-width's Tifinagh joiner can lose one column when the
            // next cluster completes a ligature. Only an excess column proves
            // that even that contraction cannot make this word fit the row.
            if last || width > self.width {
                return width.min(self.width);
            }
            index += 1;
        }
        word.width()
    }
}

impl<I: Iterator<Item = char>> Iterator for Rows<I> {
    type Item = String;

    fn next(&mut self) -> Option<String> {
        if !self.has_more() {
            self.done = true;
            return None;
        }
        if self.paragraph_start {
            self.indent = self.hanging_indent();
        }
        let mut line = if self.paragraph_start {
            String::new()
        } else {
            self.indent.clone()
        };
        self.paragraph_start = false;
        let mut used = line.width();
        loop {
            if self.word_start {
                self.word_start = false;
                if used > self.indent.len() && used.saturating_add(self.word_width()) > self.width {
                    break;
                }
            }
            let Some(g) = self.peek(0) else {
                self.done = true;
                break;
            };
            if g == "\n" {
                self.pending.pop_front();
                self.paragraph_start = true;
                self.word_start = true;
                break;
            }
            let gw = g.width();
            let word_start = g == " ";
            if used.saturating_add(gw) > self.width && used > self.indent.len() {
                break;
            }
            self.word_start = word_start;
            line.push_str(&self.pending.pop_front().expect("peeked grapheme"));
            used = used.saturating_add(gw);
        }
        #[cfg(test)]
        super::MATERIALIZED_ROWS.with(|count| count.set(count.get() + 1));
        Some(line)
    }
}

/// The columns before a list item's text, including its marker and space.
fn hanging_indent(paragraph: &str) -> Option<usize> {
    let body = paragraph.trim_start_matches(' ');
    let lead = paragraph.len() - body.len();
    let (marker, _) = body.split_once(' ')?;
    let numbered = marker.strip_suffix('.').is_some_and(|number| {
        !number.is_empty() && number.len() <= 3 && number.bytes().all(|b| b.is_ascii_digit())
    });
    (matches!(marker, "-" | "*" | "•") || numbered).then_some(lead + marker.width() + 1)
}

#[cfg(test)]
#[path = "../../../../tests/support/wrapping_stream.rs"]
mod tests;

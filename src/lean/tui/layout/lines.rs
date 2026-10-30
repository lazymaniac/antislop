use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

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
    if text.is_empty() {
        return vec![empty_line(0)];
    }
    let len = text.len();
    let mut lines = vec![];
    let mut p_start = 0;
    while p_start <= len {
        let p_end = paragraph_end(text, p_start);
        wrap_paragraph(&mut lines, &text[p_start..p_end], p_start, p_end, width);
        if p_end == len {
            break;
        }
        p_start = p_end + 1;
        if p_start == len && text.ends_with('\n') {
            lines.push(empty_line(len));
            break;
        }
    }
    lines
}

fn empty_line(at: usize) -> VisualLineInfo {
    VisualLineInfo {
        start_byte: at,
        end_byte: at,
        chars: vec![],
        width: 0,
    }
}

fn paragraph_end(text: &str, start: usize) -> usize {
    text[start..].find('\n').map_or(text.len(), |i| start + i)
}

fn wrap_paragraph(
    lines: &mut Vec<VisualLineInfo>,
    paragraph: &str,
    start: usize,
    end: usize,
    width: usize,
) {
    let mut current = LineBuilder::new(start);
    let mut word_start = 0;
    while word_start < paragraph.len() {
        let word_end = paragraph[word_start..]
            .find(' ')
            .map_or(paragraph.len(), |i| word_start + i + 1);
        current.append_word(
            lines,
            &paragraph[word_start..word_end],
            start + word_start,
            width,
        );
        word_start = word_end;
    }
    lines.push(current.finish(end));
}

struct LineBuilder {
    start: usize,
    chars: Vec<(usize, u16, u16)>,
    width: u16,
}

impl LineBuilder {
    fn new(start: usize) -> Self {
        Self {
            start,
            chars: vec![],
            width: 0,
        }
    }

    fn append_word(
        &mut self,
        lines: &mut Vec<VisualLineInfo>,
        word: &str,
        absolute: usize,
        width: usize,
    ) {
        if self.width > 0 && self.width as usize + word.width().min(width) > width {
            lines.push(self.finish(absolute));
            *self = Self::new(absolute);
        }
        for (offset, grapheme) in word.grapheme_indices(true) {
            self.append_grapheme(lines, absolute + offset, grapheme, width);
        }
    }

    fn append_grapheme(
        &mut self,
        lines: &mut Vec<VisualLineInfo>,
        at: usize,
        grapheme: &str,
        width: usize,
    ) {
        let cells = grapheme.width() as u16;
        if self.width > 0 && self.width as usize + cells as usize > width {
            lines.push(self.finish(at));
            *self = Self::new(at);
        }
        self.chars.push((at, self.width, cells));
        self.width += cells;
    }

    fn finish(&mut self, end: usize) -> VisualLineInfo {
        VisualLineInfo {
            start_byte: self.start,
            end_byte: end,
            chars: std::mem::take(&mut self.chars),
            width: std::mem::take(&mut self.width),
        }
    }
}

/// Returns the visual text lines corresponding to `compute_visual_lines`.
pub fn visual_text_lines(text: &str, width: usize) -> Vec<String> {
    let lines = compute_visual_lines(text, width);
    lines
        .into_iter()
        .map(|l| text[l.start_byte..l.end_byte].to_string())
        .collect()
}

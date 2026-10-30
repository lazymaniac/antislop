use super::buffer::InputBuffer;
use unicode_segmentation::UnicodeSegmentation;

impl InputBuffer {
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
        skip_trailing_whitespace(&mut graphemes);
        if graphemes.is_empty() {
            self.cursor = 0;
            return;
        }
        self.cursor = trailing_word_start(&mut graphemes);
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
}

fn skip_trailing_whitespace(graphemes: &mut Vec<(usize, &str)>) {
    while graphemes
        .last()
        .is_some_and(|(_, g)| g.chars().all(char::is_whitespace))
    {
        graphemes.pop();
    }
}

fn trailing_word_start(graphemes: &mut Vec<(usize, &str)>) -> usize {
    let mut start = 0;
    while let Some((index, grapheme)) = graphemes.last() {
        if grapheme.chars().all(char::is_whitespace) {
            break;
        }
        start = *index;
        graphemes.pop();
    }
    start
}

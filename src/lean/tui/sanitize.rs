//! Terminal escape sequence sanitization.
//! Provider, tool, and agent output is data, never terminal instructions.
//! Strip CSI, OSC (including OSC52 clipboard escapes), DCS/APC/PM/SOS strings,
//! C0/C1 controls, and bidirectional overrides.
//! Preserve readable newlines and tabs (expanded to spaces).
//! An unfinished escape or control string consumes the remainder of the input.

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum State {
    #[default]
    Text,
    Escape,
    Csi,
    String,
    StringEscape,
}

/// Carries escape parsing across transport chunks so an OSC or CSI sequence
/// cannot leak into the display when it crosses a chunk boundary.
#[derive(Default)]
pub(super) struct Sanitizer {
    state: State,
}

impl Sanitizer {
    pub(super) fn push(&mut self, input: &str) -> String {
        let mut output = String::with_capacity(input.len());
        for c in input.chars() {
            append_visible(self.state, c, &mut output);
            self.state = self.state.transition(c);
        }
        output
    }
}

#[derive(Clone, Copy)]
enum Visible {
    None,
    Character(char),
    Tab,
}

impl State {
    fn transition(self, c: char) -> Self {
        match self {
            Self::Text => Self::from_text(c),
            Self::Escape => Self::from_escape(c),
            Self::Csi => Self::from_csi(c),
            Self::String => Self::from_string(c),
            Self::StringEscape => {
                if c == '\\' {
                    Self::Text
                } else {
                    Self::String
                }
            }
        }
    }

    fn from_text(c: char) -> Self {
        match c {
            '\u{1b}' => Self::Escape,
            '\u{9b}' => Self::Csi,
            '\u{90}' | '\u{9d}' | '\u{98}' | '\u{9e}' | '\u{9f}' => Self::String,
            '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{061c}' => Self::Text,
            c if c.is_control() => Self::Text,
            _ => Self::Text,
        }
    }

    fn from_escape(c: char) -> Self {
        match c {
            '[' => Self::Csi,
            ']' | 'P' | '_' | '^' | 'X' => Self::String,
            _ => Self::Text,
        }
    }

    fn from_csi(c: char) -> Self {
        if ('\u{40}'..='\u{7e}').contains(&c) {
            Self::Text
        } else {
            Self::Csi
        }
    }

    fn from_string(c: char) -> Self {
        match c {
            '\u{7}' | '\u{9c}' => Self::Text,
            '\u{1b}' => Self::StringEscape,
            _ => Self::String,
        }
    }
}

fn append_visible(state: State, c: char, output: &mut String) {
    if state != State::Text {
        return;
    }
    match visible_character(c) {
        Visible::None => {}
        Visible::Character(character) => output.push(character),
        Visible::Tab => output.push_str("    "),
    }
}

fn visible_character(c: char) -> Visible {
    match c {
        '\n' => Visible::Character('\n'),
        '\t' => Visible::Tab,
        c if c.is_control() || is_bidi_control(c) => Visible::None,
        c => Visible::Character(c),
    }
}

fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200e}' | '\u{200f}' | '\u{061c}')
}

pub fn sanitize(input: &str) -> String {
    Sanitizer::default().push(input)
}

/// [`sanitize`] collapsed to one printable line for fields a view renders
/// inside a single row (titles, ids, paths, sections): every whitespace
/// run, newlines included, becomes one space so no control reaches the
/// line and no break shifts the row. Whitespace is folded on space
/// characters between clusters, so readable text and grapheme clusters
/// (emoji families, flags, combining marks) stay whole.
pub fn sanitize_line(input: &str) -> String {
    sanitize(input)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_and_newlines_preserved() {
        assert_eq!(sanitize("Hello world!"), "Hello world!");
        assert_eq!(
            sanitize("Line 1\nLine 2\n\nLine 4"),
            "Line 1\nLine 2\n\nLine 4"
        );
        assert_eq!(sanitize("col1\tcol2"), "col1    col2");
    }

    #[test]
    fn csi_colors_and_cursor_stripped() {
        assert_eq!(sanitize("\x1b[31mRed\x1b[0m"), "Red");
        assert_eq!(sanitize("\x1b[1;32;40mBold Green\x1b[m"), "Bold Green");
        assert_eq!(sanitize("\x1b[2J\x1b[HClear"), "Clear");
        // C1 CSI
        assert_eq!(sanitize("\u{9b}31mRed\u{9b}0m"), "Red");
    }

    #[test]
    fn osc52_and_clipboard_attacks_stripped() {
        // OSC 52 set clipboard terminated by BEL
        assert_eq!(
            sanitize("\x1b]52;c;cGFzc3dvcmQ=\x07Innocent text"),
            "Innocent text"
        );
        // OSC 52 terminated by ST (\x1b\\)
        assert_eq!(
            sanitize("\x1b]52;c;cGFzc3dvcmQ=\x1b\\Innocent text"),
            "Innocent text"
        );
        // C1 OSC with C1 ST
        assert_eq!(
            sanitize("\u{9d}52;c;cGFzc3dvcmQ=\u{9c}Innocent text"),
            "Innocent text"
        );
    }

    #[test]
    fn dcs_and_apc_stripped() {
        assert_eq!(sanitize("\x1bP1$txxxxx\x1b\\Data"), "Data");
        assert_eq!(sanitize("\x1b_Gpayload\x07Visible"), "Visible");
    }

    #[test]
    fn bidi_override_controls_stripped() {
        // RLO (Right-to-Left Override) U+202E spoofing filename
        assert_eq!(sanitize("report\u{202e}txt.exe"), "reporttxt.exe");
        // LRI / RLI / FSI isolates
        assert_eq!(sanitize("\u{2066}admin\u{2069}"), "admin");
    }

    #[test]
    fn unfinished_escape_consumes_tail() {
        assert_eq!(sanitize("Safe\x1b[31"), "Safe");
        assert_eq!(sanitize("Safe\x1b]52;c;something"), "Safe");
        assert_eq!(sanitize("Safe\x1bPdevice_command"), "Safe");
    }

    #[test]
    fn single_line_folds_breaks_without_splitting_clusters() {
        assert_eq!(sanitize_line("Fix\nthe  store\tpath"), "Fix the store path");
        // Escapes, controls and bidi never reach the line; text stays.
        assert_eq!(
            sanitize_line("\x1b[31mFix\u{7}\u{9b}31m the\u{202e}store\x1b]0;evil\u{7}"),
            "Fix thestore"
        );
        // Grapheme clusters stay whole: an emoji family and combining marks
        // survive the fold untouched.
        assert_eq!(sanitize_line("a\u{0301} 👩‍👩‍👧‍👦  b\n"), "a\u{0301} 👩‍👩‍👧‍👦 b");
        // Ids and paths come back byte-identical when they were clean.
        assert_eq!(sanitize_line("worker:t1"), "worker:t1");
        assert_eq!(sanitize_line("src/lean/tasks.rs"), "src/lean/tasks.rs");
        // An escape whose payload would have filled the row is gone with it.
        assert_eq!(sanitize_line("\x1b]52;c;AAAA\u{7}done"), "done");
    }
}

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
            self.state = match self.state {
                State::Text => match c {
                    '\u{1b}' => State::Escape,
                    // C1 CSI
                    '\u{9b}' => State::Csi,
                    // C1 DCS, OSC, SOS, PM, APC
                    '\u{90}' | '\u{9d}' | '\u{98}' | '\u{9e}' | '\u{9f}' => State::String,
                    '\n' => {
                        output.push('\n');
                        State::Text
                    }
                    '\t' => {
                        output.push_str("    ");
                        State::Text
                    }
                    // Bidirectional text override and isolate controls:
                    // U+202A..=U+202E (LRE, RLE, PDF, LRO, RLO)
                    // U+2066..=U+2069 (LRI, RLI, FSI, PDI)
                    // U+200E, U+200F (LRM, RLM)
                    // U+061C (ALM)
                    '\u{202a}'..='\u{202e}'
                    | '\u{2066}'..='\u{2069}'
                    | '\u{200e}'
                    | '\u{200f}'
                    | '\u{061c}' => State::Text,
                    // Other control characters (C0: 0x00..=0x1F, 0x7F; C1: 0x80..=0x9F)
                    c if c.is_control() => State::Text,
                    c => {
                        output.push(c);
                        State::Text
                    }
                },
                State::Escape => match c {
                    '[' => State::Csi,
                    // OSC (']'), DCS ('P'), SOS ('X'), PM ('^'), APC ('_')
                    ']' | 'P' | '_' | '^' | 'X' => State::String,
                    _ => State::Text,
                },
                State::Csi => {
                    // CSI parameter and intermediate bytes: 0x20..=0x3F. Final byte: 0x40..=0x7E.
                    if ('\u{40}'..='\u{7e}').contains(&c) {
                        State::Text
                    } else {
                        State::Csi
                    }
                }
                State::String => match c {
                    // String terminator: BEL ('\x07') or C1 ST ('\u{9c}')
                    '\u{7}' | '\u{9c}' => State::Text,
                    // 7-bit ST: ESC '\'
                    '\u{1b}' => State::StringEscape,
                    _ => State::String,
                },
                State::StringEscape => {
                    if c == '\\' {
                        State::Text
                    } else {
                        State::String
                    }
                }
            };
        }
        output
    }
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

//! Provider/tool output is data, never terminal instructions.
/// Strip CSI, OSC, DCS/APC/PM strings, C0/C1 controls and bidirectional overrides.
/// Preserve readable tabs/newlines. An unfinished escape consumes the remainder.
pub fn sanitize(input: &str) -> String {
    #[derive(Clone, Copy)]
    enum State {
        Text,
        Escape,
        Csi,
        String,
        StringEscape,
    }
    let mut state = State::Text;
    let mut output = String::with_capacity(input.len());
    for c in input.chars() {
        state = match state {
            State::Text => match c {
                '\u{1b}' => State::Escape,
                '\u{9b}' => State::Csi,
                '\u{90}' | '\u{9d}' | '\u{98}' | '\u{9e}' | '\u{9f}' => State::String,
                '\n' => {
                    output.push('\n');
                    State::Text
                }
                '\t' => {
                    output.push_str("    ");
                    State::Text
                }
                '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' => State::Text,
                c if c.is_control() => State::Text,
                c => {
                    output.push(c);
                    State::Text
                }
            },
            State::Escape => match c {
                '[' => State::Csi,
                ']' | 'P' | '_' | '^' | 'X' => State::String,
                _ => State::Text,
            },
            State::Csi => {
                if ('\u{40}'..='\u{7e}').contains(&c) {
                    State::Text
                } else {
                    State::Csi
                }
            }
            State::String => match c {
                '\u{7}' | '\u{9c}' => State::Text,
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

use super::*;
#[derive(Clone)]
pub(super) struct Source {
    pub(super) text: String,
    pub(super) lines: Vec<usize>,
    pub(super) path: PathBuf,
    pub(super) fs_path: PathBuf,
}
impl Source {
    pub(super) fn new(path: PathBuf, fs_path: PathBuf, text: String) -> Self {
        let lines = line_starts(&text);
        Self {
            text,
            lines,
            path,
            fs_path,
        }
    }
    pub(super) fn line(&self, pos: usize) -> usize {
        self.lines.partition_point(|p| *p <= pos).max(1)
    }
    pub(super) fn line_count(&self) -> usize {
        physical_lines(&self.text)
    }
    pub(super) fn offset(&self, line: usize) -> usize {
        self.lines
            .get(line.saturating_sub(1))
            .copied()
            .unwrap_or(self.text.len())
    }
    pub(super) fn callable_start(&self, ast_start: usize) -> usize {
        let ln = self.line(ast_start);
        let prefix_kinds = prefix_line_kinds(&self.text);
        let mut first = ln;
        // Attach contiguous comments/outer attributes, retaining intervening blanks.
        let mut cursor = ln.saturating_sub(1);
        while cursor > 0 {
            let prior = cursor;
            let line = self.text.lines().nth(prior - 1).unwrap_or("").trim();
            if line.is_empty() {
                if prior >= 2 && prefix_kinds[prior - 2] {
                    cursor -= 1;
                    continue;
                }
                break;
            }
            if prefix_kinds[prior - 1] {
                first = prior;
                cursor -= 1;
                continue;
            }
            break;
        }
        self.offset(first)
    }
    pub(super) fn count_span(&self, start: usize, end: usize) -> usize {
        let s = self.line(start);
        let e = self.line(end.saturating_sub(1).max(start));
        e.saturating_sub(s) + 1
    }
}
#[derive(Clone, Copy)]
enum LexicalState {
    Normal,
    BlockComment { depth: usize, leading: bool },
    Quoted { quote: u8, escaped: bool },
    RawString { hashes: usize },
}

pub(super) fn prefix_line_kinds(source: &str) -> Vec<bool> {
    let mut state = LexicalState::Normal;
    source
        .lines()
        .map(|raw| scan_prefix_line(raw, &mut state))
        .collect()
}

fn scan_prefix_line(raw: &str, state: &mut LexicalState) -> bool {
    let line = raw.trim();
    let prefix = starts_comment_prefix(line, *state);
    let attribute = matches!(state, LexicalState::Normal) && line.starts_with("#[");
    scan_lexical_state(raw.as_bytes(), state);
    prefix || attribute
}

fn starts_comment_prefix(line: &str, state: LexicalState) -> bool {
    match state {
        LexicalState::BlockComment { leading, .. } => leading,
        LexicalState::Normal => {
            (line.starts_with("//") && !line.starts_with("//!"))
                || (line.starts_with("/*") && !line.starts_with("/*!"))
        }
        LexicalState::Quoted { .. } | LexicalState::RawString { .. } => false,
    }
}

fn scan_lexical_state(bytes: &[u8], state: &mut LexicalState) {
    let mut cursor = 0;
    while cursor < bytes.len() {
        cursor += advance_lexical_state(bytes, cursor, state);
    }
}

fn advance_lexical_state(bytes: &[u8], i: usize, state: &mut LexicalState) -> usize {
    match *state {
        LexicalState::Normal => advance_normal(bytes, i, state),
        LexicalState::BlockComment { .. } => advance_block_comment(bytes, i, state),
        LexicalState::Quoted { .. } => advance_quoted(bytes, i, state),
        LexicalState::RawString { .. } => advance_raw_string(bytes, i, state),
    }
}

fn advance_normal(bytes: &[u8], i: usize, state: &mut LexicalState) -> usize {
    if pair_is(bytes, i, b'/', b'/') {
        return bytes.len() - i;
    }
    if pair_is(bytes, i, b'/', b'*') {
        let leading = bytes[..i].iter().all(u8::is_ascii_whitespace);
        *state = LexicalState::BlockComment { depth: 1, leading };
        return 2;
    }
    if let Some((_, hashes, width)) = raw_string_start(bytes, i) {
        *state = LexicalState::RawString { hashes };
        return width;
    }
    if (matches!(bytes[i], b'b' | b'c') && bytes.get(i + 1) == Some(&b'"')) || bytes[i] == b'"' {
        *state = LexicalState::Quoted {
            quote: b'"',
            escaped: false,
        };
        return if bytes[i] == b'"' { 1 } else { 2 };
    }
    if bytes[i] == b'\'' && looks_like_char_literal(bytes, i) {
        *state = LexicalState::Quoted {
            quote: b'\'',
            escaped: false,
        };
    }
    1
}

fn advance_block_comment(bytes: &[u8], i: usize, state: &mut LexicalState) -> usize {
    let LexicalState::BlockComment { depth, leading } = *state else {
        return 1;
    };
    if pair_is(bytes, i, b'/', b'*') {
        *state = LexicalState::BlockComment {
            depth: depth + 1,
            leading,
        };
        2
    } else if pair_is(bytes, i, b'*', b'/') {
        *state = if depth == 1 {
            LexicalState::Normal
        } else {
            LexicalState::BlockComment {
                depth: depth - 1,
                leading,
            }
        };
        2
    } else {
        1
    }
}

fn advance_quoted(bytes: &[u8], i: usize, state: &mut LexicalState) -> usize {
    let LexicalState::Quoted { quote, escaped } = *state else {
        return 1;
    };
    *state = if escaped {
        LexicalState::Quoted {
            quote,
            escaped: false,
        }
    } else if bytes[i] == b'\\' {
        LexicalState::Quoted {
            quote,
            escaped: true,
        }
    } else if bytes[i] == quote {
        LexicalState::Normal
    } else {
        return 1;
    };
    1
}

fn advance_raw_string(bytes: &[u8], i: usize, state: &mut LexicalState) -> usize {
    let LexicalState::RawString { hashes } = *state else {
        return 1;
    };
    if bytes[i] == b'"' && (0..hashes).all(|n| bytes.get(i + 1 + n) == Some(&b'#')) {
        *state = LexicalState::Normal;
        hashes + 1
    } else {
        1
    }
}

fn pair_is(bytes: &[u8], i: usize, first: u8, second: u8) -> bool {
    bytes.get(i) == Some(&first) && bytes.get(i + 1) == Some(&second)
}

pub(super) fn raw_string_start(bytes: &[u8], start: usize) -> Option<(u8, usize, usize)> {
    let mut i = start;
    if bytes.get(i) == Some(&b'b') {
        i += 1;
    }
    if bytes.get(i) != Some(&b'r') {
        return None;
    }
    i += 1;
    let hashes_start = i;
    while bytes.get(i) == Some(&b'#') {
        i += 1;
    }
    (bytes.get(i) == Some(&b'"')).then_some((b'"', i - hashes_start, i - start + 1))
}

pub(super) fn looks_like_char_literal(bytes: &[u8], start: usize) -> bool {
    let mut i = start + 1;
    if bytes.get(i) == Some(&b'\\') {
        i += 2;
    } else {
        i += 1;
    }
    bytes.get(i) == Some(&b'\'')
}
pub(super) fn line_starts(s: &str) -> Vec<usize> {
    let mut v = vec![0];
    for (i, b) in s.bytes().enumerate() {
        if b == b'\n' && i + 1 < s.len() {
            v.push(i + 1);
        }
    }
    v
}
pub(super) fn physical_lines(s: &str) -> usize {
    if s.is_empty() {
        0
    } else {
        s.bytes().filter(|b| *b == b'\n').count() + usize::from(!s.ends_with('\n'))
    }
}
pub(super) fn offset(span: proc_macro2::Span, src: &Source) -> usize {
    src.offset(span.start().line)
}

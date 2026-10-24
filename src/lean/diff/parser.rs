//! Unified diff parsing, isolated from the public diff data types.

use super::{DiffLine, FileDiff};

fn line(kind: &str, old: Option<u32>, new: Option<u32>, text: &str) -> DiffLine {
    DiffLine {
        kind: kind.into(),
        old,
        new,
        text: text.into(),
    }
}

/// What a diff comment targets: `diff:TASK[:FILE[:LINE]]`, where LINE is a
/// new line number or `oldN` for a removed line. A file may contain `:`.
pub struct Target<'a> {
    pub task: &'a str,
    pub file: Option<&'a str>,
    pub line: Option<&'a str>,
}

pub fn target(key: &str) -> Option<Target<'_>> {
    let rest = key.strip_prefix("diff:")?;
    let (task, file) = match rest.split_once(':') {
        Some((task, file)) => (task, Some(file)),
        None => (rest, None),
    };
    let (file, line) = match file.and_then(|file| file.rsplit_once(':')) {
        Some((file, line))
            if line.trim_start_matches("old").parse::<u32>().is_ok() && !file.is_empty() =>
        {
            (Some(file), Some(line))
        }
        _ => (file, None),
    };
    Some(Target { task, file, line })
}

/// A path from a header line, without its `a/` or `b/` and git's quotes.
fn path(text: &str) -> String {
    let text = text.trim_end_matches('\t');
    let text = text
        .strip_prefix('"')
        .and_then(|text| text.strip_suffix('"'))
        .unwrap_or(text);
    text.strip_prefix("a/")
        .or_else(|| text.strip_prefix("b/"))
        .unwrap_or(text)
        .to_owned()
}

/// `@@ -12,3 +14,5 @@` → (12, 14).
fn hunk_start(header: &str) -> Option<(u32, u32)> {
    let mut parts = header.split_whitespace().skip(1);
    let number = |part: Option<&str>, sign: char| -> Option<u32> {
        part?.strip_prefix(sign)?.split(',').next()?.parse().ok()
    };
    Some((number(parts.next(), '-')?, number(parts.next(), '+')?))
}

/// Parses `git diff` output (unified, with git's extended headers).
pub(super) fn parse(patch: &str) -> Vec<FileDiff> {
    let mut parser = Parser::default();
    for text in patch.lines() {
        parser.consume(text);
    }
    parser.files
}

#[derive(Default)]
struct Parser {
    files: Vec<FileDiff>,
    in_hunk: bool,
    old: u32,
    new: u32,
}

impl Parser {
    fn consume(&mut self, text: &str) {
        if let Some(header) = text.strip_prefix("diff --git ") {
            self.start_file(header);
            return;
        }
        let Some(file) = self.files.last_mut() else {
            return;
        };
        if !self.in_hunk {
            update_metadata(file, text);
        }
        consume_hunk(&mut self.in_hunk, &mut self.old, &mut self.new, file, text);
    }

    fn start_file(&mut self, header: &str) {
        self.in_hunk = false;
        // Exact paths come from the ---/+++ or rename lines; this one
        // only names files without content changes.
        let guess = header.rfind(" b/").map_or(header, |at| &header[at + 1..]);
        self.files.push(FileDiff {
            path: path(guess),
            old_path: None,
            status: "modified".into(),
            added: 0,
            removed: 0,
            binary: false,
            lines: vec![],
        });
    }
}

fn update_metadata(file: &mut FileDiff, text: &str) {
    if text.starts_with("new file mode") {
        file.status = "added".into();
    } else if text.starts_with("deleted file mode") {
        file.status = "deleted".into();
    } else if let Some(from) = text.strip_prefix("rename from ") {
        file.status = "renamed".into();
        file.old_path = Some(path(from));
    } else if let Some(to) = text.strip_prefix("rename to ") {
        file.path = path(to);
    } else if let Some(to) = text.strip_prefix("+++ ") {
        if to != "/dev/null" {
            file.path = path(to);
        }
    } else if let Some(from) = text.strip_prefix("--- ") {
        if from != "/dev/null" && file.status == "deleted" {
            file.path = path(from);
        }
    } else if text.starts_with("Binary files ") {
        file.binary = true;
    }
}

fn consume_hunk(in_hunk: &mut bool, old: &mut u32, new: &mut u32, file: &mut FileDiff, text: &str) {
    if text.starts_with("@@") {
        if let Some((old_start, new_start)) = hunk_start(text) {
            (*old, *new) = (old_start, new_start);
            *in_hunk = true;
            file.lines.push(line("hunk", None, None, text));
        }
    } else if *in_hunk {
        append_hunk_line(file, old, new, text);
    }
}

fn append_hunk_line(file: &mut FileDiff, old: &mut u32, new: &mut u32, text: &str) {
    match text.chars().next() {
        Some('+') => {
            file.added += 1;
            file.lines.push(line("add", None, Some(*new), &text[1..]));
            *new += 1;
        }
        Some('-') => {
            file.removed += 1;
            file.lines.push(line("del", Some(*old), None, &text[1..]));
            *old += 1;
        }
        Some('\\') => {}
        _ => {
            file.lines.push(line(
                "context",
                Some(*old),
                Some(*new),
                text.get(1..).unwrap_or_default(),
            ));
            *old += 1;
            *new += 1;
        }
    }
}

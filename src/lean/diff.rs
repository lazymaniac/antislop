//! A writer's change as files and numbered lines, for the diff view.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: String,
    /// The path before a rename.
    pub old_path: Option<String>,
    /// `added`, `deleted`, `renamed` or `modified`.
    pub status: String,
    pub added: usize,
    pub removed: usize,
    pub binary: bool,
    pub lines: Vec<DiffLine>,
}

/// One line of a hunk. `kind` is `hunk` (its header), `context`, `add` or
/// `del`; `old` and `new` are its line numbers in each version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiffLine {
    pub kind: String,
    pub old: Option<u32>,
    pub new: Option<u32>,
    pub text: String,
}

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
pub fn parse(patch: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = vec![];
    let mut in_hunk = false;
    let (mut old, mut new) = (0, 0);
    for text in patch.lines() {
        if let Some(header) = text.strip_prefix("diff --git ") {
            in_hunk = false;
            // Exact paths come from the ---/+++ or rename lines; this one
            // only names files without content changes.
            let guess = header.rfind(" b/").map_or(header, |at| &header[at + 1..]);
            files.push(FileDiff {
                path: path(guess),
                old_path: None,
                status: "modified".into(),
                added: 0,
                removed: 0,
                binary: false,
                lines: vec![],
            });
            continue;
        }
        let Some(file) = files.last_mut() else {
            continue;
        };
        if !in_hunk {
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
        if text.starts_with("@@") {
            if let Some((o, n)) = hunk_start(text) {
                (old, new) = (o, n);
                in_hunk = true;
                file.lines.push(line("hunk", None, None, text));
            }
            continue;
        }
        if !in_hunk {
            continue;
        }
        match text.chars().next() {
            Some('+') => {
                file.added += 1;
                file.lines.push(line("add", None, Some(new), &text[1..]));
                new += 1;
            }
            Some('-') => {
                file.removed += 1;
                file.lines.push(line("del", Some(old), None, &text[1..]));
                old += 1;
            }
            Some('\\') => {}
            _ => {
                file.lines.push(line(
                    "context",
                    Some(old),
                    Some(new),
                    text.get(1..).unwrap_or_default(),
                ));
                old += 1;
                new += 1;
            }
        }
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATCH: &str = "diff --git a/src/a.rs b/src/a.rs
index 1..2 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -10,3 +10,4 @@ fn main() {
 keep
-old
+new
+more
 tail
diff --git a/new.txt b/new.txt
new file mode 100644
index 0..1
--- /dev/null
+++ b/new.txt
@@ -0,0 +1 @@
+hello
\\ No newline at end of file
diff --git a/gone.txt b/gone.txt
deleted file mode 100644
--- a/gone.txt
+++ /dev/null
@@ -1 +0,0 @@
-bye
diff --git a/old name.rs b/new name.rs
similarity index 100%
rename from old name.rs
rename to new name.rs
diff --git a/logo.png b/logo.png
index 1..2 100644
Binary files a/logo.png and b/logo.png differ
";

    #[test]
    fn diff_targets_keep_colons_in_paths() {
        let parts = |key| {
            let target = target(key).unwrap();
            (target.task, target.file, target.line)
        };
        assert_eq!(parts("diff:t2"), ("t2", None, None));
        assert_eq!(parts("diff:t2:src/a.rs"), ("t2", Some("src/a.rs"), None));
        assert_eq!(
            parts("diff:t2:src/a:b.rs:14"),
            ("t2", Some("src/a:b.rs"), Some("14"))
        );
        assert_eq!(
            parts("diff:t2:a.rs:old3"),
            ("t2", Some("a.rs"), Some("old3"))
        );
        assert!(target("worker:t2").is_none());
    }

    #[test]
    fn files_get_their_status_counts_and_line_numbers() {
        let files = parse(PATCH);
        let summary: Vec<(&str, &str, usize, usize)> = files
            .iter()
            .map(|file| {
                (
                    file.path.as_str(),
                    file.status.as_str(),
                    file.added,
                    file.removed,
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("src/a.rs", "modified", 2, 1),
                ("new.txt", "added", 1, 0),
                ("gone.txt", "deleted", 0, 1),
                ("new name.rs", "renamed", 0, 0),
                ("logo.png", "modified", 0, 0),
            ]
        );
        assert_eq!(files[3].old_path.as_deref(), Some("old name.rs"));
        assert!(files[4].binary);
        let numbers: Vec<(&str, Option<u32>, Option<u32>)> = files[0]
            .lines
            .iter()
            .map(|line| (line.kind.as_str(), line.old, line.new))
            .collect();
        assert_eq!(
            numbers,
            [
                ("hunk", None, None),
                ("context", Some(10), Some(10)),
                ("del", Some(11), None),
                ("add", None, Some(11)),
                ("add", None, Some(12)),
                ("context", Some(12), Some(13)),
            ]
        );
        assert_eq!(files[1].lines.len(), 2, "no line for the newline marker");
    }
}

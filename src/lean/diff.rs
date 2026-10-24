//! A writer's change as files and numbered lines, for the diff view.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewerProfile {
    pub provider: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
}

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

mod parser;
pub use parser::{Target, target};

pub fn parse(patch: &str) -> Vec<FileDiff> {
    parser::parse(patch)
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

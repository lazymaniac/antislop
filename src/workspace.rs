//! Worktrees isolate file ownership, not permissions. External commands remain
//! explicitly authorized local execution, never a substitute for an OS sandbox.
pub mod managed;
pub mod snapshot;
use crate::{Result, hash, require};
use std::{
    collections::BTreeMap,
    path::{Component, Path},
    process::Command,
};

pub fn read_input_bytes(root: &Path, name: &str) -> Result<Vec<u8>> {
    let root = root.canonicalize()?;
    let relative = Path::new(name);
    require(
        !relative.as_os_str().is_empty()
            && relative
                .components()
                .all(|c| matches!(c, Component::Normal(_))),
        "input path must be a normal relative path",
    )?;
    let mut path = root.clone();
    for component in relative.components() {
        path.push(component);
        require(
            !std::fs::symlink_metadata(&path)?.file_type().is_symlink(),
            "symlink inputs are not supported",
        )?;
    }
    require(
        path.canonicalize()?.starts_with(&root),
        "input escaped workspace",
    )?;
    Ok(std::fs::read(path)?)
}
pub fn read_inputs(root: &Path, paths: &[String]) -> Result<BTreeMap<String, String>> {
    paths
        .iter()
        .map(|name| {
            let bytes = read_input_bytes(root, name)?;
            let text = String::from_utf8(bytes)
                .map_err(|_| crate::Error::Invalid("pinned input is not UTF-8".into()))?;
            Ok((name.clone(), text))
        })
        .collect()
}
pub fn worktree(repository: &Path, destination: &Path) -> Result<()> {
    require(!destination.exists(), "worktree destination exists")?;
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(["worktree", "add", "--detach"])
        .arg(destination)
        .arg("HEAD")
        .output()?;
    require(output.status.success(), "git worktree creation failed")
}
/// Compare-and-write refuses a changed input rather than overwriting user work.
pub fn replace(root: &Path, relative: &str, expected_hash: &str, content: &str) -> Result<()> {
    let inputs = read_inputs(root, &[relative.into()])?;
    require(
        hash(&inputs[relative]) == expected_hash,
        "workspace input changed; replan required",
    )?;
    // A single runtime writer owns this worktree. Concurrent external writers are
    // detected before write, but arbitrary hostile filesystem races need sandboxing.
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let target = root.join(relative);
    let permissions = std::fs::metadata(&target)?.permissions();
    let temporary = target.with_file_name(format!(".architect-{}.tmp", crate::id()));
    let result = (|| -> Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(content.as_bytes())?;
        file.set_permissions(permissions)?;
        file.sync_all()?;
        // Rename replaces this directory entry instead of mutating shared hard links.
        std::fs::rename(&temporary, &target)?;
        if let Some(parent) = target.parent() {
            std::fs::File::open(parent)?.sync_all()?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result?;
    Ok(())
}

pub fn check(
    root: &Path,
    argv: &[String],
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<(bool, String)> {
    use std::io::Read;
    use std::os::unix::process::ExitStatusExt;
    use std::process::Stdio;
    use std::sync::atomic::Ordering;
    require(!argv.is_empty(), "empty check command")?;
    let mut command = crate::process::command(&argv[0])?;
    command
        .args(&argv[1..])
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = crate::process::SupervisedChild::spawn(&mut command)?;
    let stdout = child.child.stdout.take().expect("check stdout");
    let stderr = child.child.stderr.take().expect("check stderr");
    fn drain(mut source: impl Read) -> std::io::Result<(Vec<u8>, bool)> {
        let mut output = Vec::new();
        let mut chunk = [0; 8192];
        let mut truncated = false;
        loop {
            let count = source.read(&mut chunk)?;
            if count == 0 {
                break;
            }
            let keep = count.min((4 * 1024 * 1024usize).saturating_sub(output.len()));
            output.extend_from_slice(&chunk[..keep]);
            truncated |= keep < count;
        }
        Ok((output, truncated))
    }
    let out_reader = std::thread::spawn(move || drain(stdout));
    let err_reader = std::thread::spawn(move || drain(stderr));
    let status = loop {
        if cancelled.load(Ordering::SeqCst) {
            child.cancel()?;
            return Err(crate::Error::Invalid(
                "check cancelled; effects require reconciliation".into(),
            ));
        }
        if let Some(status) = child.child.try_wait()? {
            break status;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    };
    let (stdout, out_truncated) = out_reader
        .join()
        .map_err(|_| crate::Error::Invalid("stdout reader failed".into()))??;
    let (stderr, err_truncated) = err_reader
        .join()
        .map_err(|_| crate::Error::Invalid("stderr reader failed".into()))??;
    let output = serde_json::json!({"argv":argv,"exit_code":status.code(),"signal":status.signal(),"stdout":String::from_utf8_lossy(&stdout),
        "stderr":String::from_utf8_lossy(&stderr),"truncated":out_truncated || err_truncated}).to_string();
    Ok((status.success() && !out_truncated && !err_truncated, output))
}

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffFileStatus {
    Modified,
    Added,
    Deleted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffLineKind {
    Context,
    Addition,
    Deletion,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub old_line_number: Option<usize>,
    pub new_line_number: Option<usize>,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffHunk {
    pub old_start: usize,
    pub old_lines: usize,
    pub new_start: usize,
    pub new_lines: usize,
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffFile {
    pub path: String,
    pub status: DiffFileStatus,
    pub additions: usize,
    pub deletions: usize,
    pub hunks: Vec<DiffHunk>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffReport {
    pub files: Vec<DiffFile>,
    pub total_additions: usize,
    pub total_deletions: usize,
}

pub fn diff_texts(path: &str, old_text: Option<&str>, new_text: Option<&str>) -> Option<DiffFile> {
    if old_text == new_text {
        return None;
    }
    match (old_text, new_text) {
        (None, Some(new)) => {
            let lines: Vec<&str> = new.lines().collect();
            let count = lines.len();
            let diff_lines = lines
                .into_iter()
                .enumerate()
                .map(|(i, l)| DiffLine {
                    kind: DiffLineKind::Addition,
                    old_line_number: None,
                    new_line_number: Some(i + 1),
                    content: l.to_string(),
                })
                .collect();
            let hunk = DiffHunk {
                old_start: 0,
                old_lines: 0,
                new_start: if count > 0 { 1 } else { 0 },
                new_lines: count,
                header: format!("@@ -0,0 +1,{count} @@"),
                lines: diff_lines,
            };
            Some(DiffFile {
                path: path.to_string(),
                status: DiffFileStatus::Added,
                additions: count,
                deletions: 0,
                hunks: vec![hunk],
            })
        }
        (Some(old), None) => {
            let lines: Vec<&str> = old.lines().collect();
            let count = lines.len();
            let diff_lines = lines
                .into_iter()
                .enumerate()
                .map(|(i, l)| DiffLine {
                    kind: DiffLineKind::Deletion,
                    old_line_number: Some(i + 1),
                    new_line_number: None,
                    content: l.to_string(),
                })
                .collect();
            let hunk = DiffHunk {
                old_start: if count > 0 { 1 } else { 0 },
                old_lines: count,
                new_start: 0,
                new_lines: 0,
                header: format!("@@ -1,{count} +0,0 @@"),
                lines: diff_lines,
            };
            Some(DiffFile {
                path: path.to_string(),
                status: DiffFileStatus::Deleted,
                additions: 0,
                deletions: count,
                hunks: vec![hunk],
            })
        }
        (Some(old), Some(new)) => {
            let old_raw: Vec<&str> = old.lines().collect();
            let new_raw: Vec<&str> = new.lines().collect();
            let n = old_raw.len();
            let m = new_raw.len();
            let mut dp = vec![vec![0u32; m + 1]; n + 1];
            for i in 0..n {
                for j in 0..m {
                    if old_raw[i] == new_raw[j] {
                        dp[i + 1][j + 1] = dp[i][j] + 1;
                    } else {
                        dp[i + 1][j + 1] = dp[i][j + 1].max(dp[i + 1][j]);
                    }
                }
            }
            let mut i = n;
            let mut j = m;
            let mut all_lines = Vec::new();
            while i > 0 || j > 0 {
                if i > 0 && j > 0 && old_raw[i - 1] == new_raw[j - 1] {
                    all_lines.push(DiffLine {
                        kind: DiffLineKind::Context,
                        old_line_number: Some(i),
                        new_line_number: Some(j),
                        content: old_raw[i - 1].to_string(),
                    });
                    i -= 1;
                    j -= 1;
                } else if j > 0 && (i == 0 || dp[i][j - 1] >= dp[i - 1][j]) {
                    all_lines.push(DiffLine {
                        kind: DiffLineKind::Addition,
                        old_line_number: None,
                        new_line_number: Some(j),
                        content: new_raw[j - 1].to_string(),
                    });
                    j -= 1;
                } else if i > 0 {
                    all_lines.push(DiffLine {
                        kind: DiffLineKind::Deletion,
                        old_line_number: Some(i),
                        new_line_number: None,
                        content: old_raw[i - 1].to_string(),
                    });
                    i -= 1;
                }
            }
            all_lines.reverse();
            let change_indices: Vec<usize> = all_lines
                .iter()
                .enumerate()
                .filter(|(_, l)| l.kind != DiffLineKind::Context)
                .map(|(idx, _)| idx)
                .collect();
            if change_indices.is_empty() {
                return None;
            }
            let mut clusters: Vec<(usize, usize)> = Vec::new();
            let mut cur_start = change_indices[0];
            let mut cur_end = change_indices[0];
            for &idx in &change_indices[1..] {
                if idx <= cur_end + 6 {
                    cur_end = idx;
                } else {
                    clusters.push((cur_start, cur_end));
                    cur_start = idx;
                    cur_end = idx;
                }
            }
            clusters.push((cur_start, cur_end));

            let mut hunks = Vec::new();
            let mut total_adds = 0;
            let mut total_dels = 0;
            for (start_idx, end_idx) in clusters {
                let hunk_start = start_idx.saturating_sub(3);
                let hunk_end = (end_idx + 3 + 1).min(all_lines.len());
                let hunk_slice = &all_lines[hunk_start..hunk_end];

                let old_count = hunk_slice
                    .iter()
                    .filter(|l| l.kind != DiffLineKind::Addition)
                    .count();
                let new_count = hunk_slice
                    .iter()
                    .filter(|l| l.kind != DiffLineKind::Deletion)
                    .count();

                let old_start = hunk_slice
                    .iter()
                    .find_map(|l| l.old_line_number)
                    .unwrap_or(0);
                let new_start = hunk_slice
                    .iter()
                    .find_map(|l| l.new_line_number)
                    .unwrap_or(0);

                let header = format!("@@ -{old_start},{old_count} +{new_start},{new_count} @@");
                hunks.push(DiffHunk {
                    old_start,
                    old_lines: old_count,
                    new_start,
                    new_lines: new_count,
                    header,
                    lines: hunk_slice.to_vec(),
                });
            }
            for line in &all_lines {
                match line.kind {
                    DiffLineKind::Addition => total_adds += 1,
                    DiffLineKind::Deletion => total_dels += 1,
                    DiffLineKind::Context => {}
                }
            }
            Some(DiffFile {
                path: path.to_string(),
                status: DiffFileStatus::Modified,
                additions: total_adds,
                deletions: total_dels,
                hunks,
            })
        }
        (None, None) => None,
    }
}

fn scan_directory(dir: &Path, base: &Path, out: &mut BTreeSet<String>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = entry.file_name();
        let name_str = file_name.to_string_lossy();
        if name_str.starts_with(".git")
            || name_str == "target"
            || name_str == ".antislop"
            || name_str.starts_with(".architect-")
        {
            continue;
        }
        if path.is_dir() {
            scan_directory(&path, base, out);
        } else if path.is_file() {
            let Ok(rel) = path.strip_prefix(base) else {
                continue;
            };
            out.insert(rel.to_string_lossy().to_string());
        }
    }
}

pub fn compute_diff(root: &Path, baseline: &BTreeMap<String, String>) -> Result<DiffReport> {
    let mut paths: BTreeSet<String> = baseline.keys().cloned().collect();
    if root.exists() {
        scan_directory(root, root, &mut paths);
    }
    let mut files = Vec::new();
    let mut total_additions = 0;
    let mut total_deletions = 0;
    for path in paths {
        let old_text = baseline.get(&path).map(String::as_str);
        let target_path = root.join(&path);
        let new_text = if target_path.is_file() {
            std::fs::read_to_string(&target_path).ok()
        } else {
            None
        };
        if let Some(diff_file) = diff_texts(&path, old_text, new_text.as_deref()) {
            total_additions += diff_file.additions;
            total_deletions += diff_file.deletions;
            files.push(diff_file);
        }
    }
    Ok(DiffReport {
        files,
        total_additions,
        total_deletions,
    })
}

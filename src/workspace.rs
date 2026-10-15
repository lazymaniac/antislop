//! Worktrees isolate file ownership, not permissions. External commands remain
//! explicitly authorized local execution, never a substitute for an OS sandbox.
pub mod delivery;
pub mod inputs;
pub mod instructions;
pub mod managed;
pub mod snapshot;
pub mod views;
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
/// Validate the selected scope without loading its source bodies.
pub fn validate_inputs(root: &Path, paths: &[String]) -> Result<()> {
    let root = root.canonicalize()?;
    for name in paths {
        let relative = inputs::relative(&root, name)?;
        let mut path = root.clone();
        for component in Path::new(&relative).components() {
            if let Component::Normal(part) = component {
                path.push(part);
                require(
                    !path.symlink_metadata()?.file_type().is_symlink(),
                    "symlink input scopes are not followed",
                )?;
            }
        }
        let metadata = path.metadata()?;
        require(
            metadata.is_file() || metadata.is_dir(),
            "input must be a file or directory",
        )?;
    }
    Ok(())
}

/// Whether the project still holds the run's pinned inputs. In task-worktree
/// mode team work reads its private base commit instead; the project advances
/// through integrations and user edits without invalidating task inputs.
pub fn inputs_current(run: &crate::domain::Run) -> Result<bool> {
    if run.team.is_some() && crate::runtime::team::views::enabled(run) {
        return Ok(true);
    }
    Ok(read_inputs(Path::new(&run.spec.workspace), &run.spec.input_files)? == run.input_snapshot)
}
pub fn read_inputs(root: &Path, paths: &[String]) -> Result<BTreeMap<String, String>> {
    let root = root.canonicalize()?;
    paths
        .iter()
        .map(|name| {
            let relative = inputs::relative(&root, name)?;
            let mut path = root.clone();
            for component in Path::new(&relative).components() {
                if let Component::Normal(part) = component {
                    path.push(part);
                    require(
                        !std::fs::symlink_metadata(&path)?.file_type().is_symlink(),
                        "symlink input scopes are not followed",
                    )?;
                }
            }
            let text = if path.is_dir() {
                inputs::directory(&path)?
            } else {
                let bytes = read_input_bytes(&root, &relative)?;
                String::from_utf8(bytes).map_err(|_| {
                    crate::Error::Invalid(
                        "pinned file input is not UTF-8; select its containing directory for binary metadata"
                            .into(),
                    )
                })?
            };
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
pub fn apply_edit(root: &Path, edit: &crate::domain::FileEdit, expected_hash: &str) -> Result<()> {
    use crate::domain::FileOperation;
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    if edit.operation == FileOperation::Write {
        return replace(root, &edit.path, expected_hash, &edit.content);
    }
    let relative = inputs::relative(root, &edit.path)?;
    require(
        relative != "."
            && !Path::new(&relative).components().any(|c| {
                c.as_os_str()
                    .to_str()
                    .is_some_and(|p| p.eq_ignore_ascii_case(".git"))
            }),
        "invalid file effect path",
    )?;
    require(
        !edit.content.contains('\0')
            && (edit.operation != FileOperation::Delete || edit.content.is_empty()),
        "invalid text file effect payload",
    )?;
    let mut parent = root.canonicalize()?;
    let parts: Vec<_> = Path::new(&relative).components().collect();
    for part in &parts[..parts.len() - 1] {
        parent.push(part.as_os_str());
        if edit.operation == FileOperation::Create && !parent.exists() {
            std::fs::create_dir(&parent)?;
        }
        let metadata = parent.symlink_metadata()?;
        require(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "file effect ancestor is not an ordinary directory",
        )?;
    }
    let target = parent.join(parts.last().unwrap().as_os_str());
    match edit.operation {
        FileOperation::Create => {
            require(
                expected_hash == inputs::absent_hash(),
                "create requires an observed absent target",
            )?;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o644)
                .open(&target)?;
            file.write_all(edit.content.as_bytes())?;
            file.sync_all()?;
        }
        FileOperation::Delete => {
            require(
                hash(read_input_bytes(root, &relative)?) == expected_hash,
                "delete target changed; replan required",
            )?;
            std::fs::remove_file(&target)?;
        }
        FileOperation::Write => unreachable!(),
    }
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata_change: Option<String>,
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
                metadata_change: None,
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
                metadata_change: None,
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
                metadata_change: None,
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
    if baseline.iter().any(|(scope, text)| {
        scope == "." || root.join(scope).is_dir() || inputs::is_directory_envelope(text)
    }) {
        return directory_diff(root, baseline);
    }

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

fn directory_diff(root: &Path, baseline: &BTreeMap<String, String>) -> Result<DiffReport> {
    use inputs::DiffContent;
    let mut before = BTreeMap::new();
    let mut after = BTreeMap::new();
    for (scope, text) in baseline {
        let relative = inputs::relative(root, scope)?;
        let directory = if inputs::is_directory_envelope(text) || !root.join(scope).is_file() {
            inputs::directory_diff_entries(&relative, text)?
        } else {
            None
        };
        if let Some(entries) = directory {
            before.extend(entries);
            after.extend(inputs::current_directory_diff_entries(root, scope)?);
        } else {
            before.insert(relative.clone(), DiffContent::Text(text.clone()));
            after.extend(inputs::current_file_diff_entry(root, &relative)?);
        }
    }
    let paths: BTreeSet<_> = before.keys().chain(after.keys()).cloned().collect();
    let mut report = DiffReport::default();
    for path in paths {
        let old = before.get(&path);
        let new = after.get(&path);
        if old == new {
            continue;
        }
        let file = match (old, new) {
            (Some(DiffContent::Text(a)), Some(DiffContent::Text(b))) => {
                diff_texts(&path, Some(a), Some(b))
            }
            (Some(DiffContent::Text(a)), None) => diff_texts(&path, Some(a), None),
            (None, Some(DiffContent::Text(b))) => diff_texts(&path, None, Some(b)),
            _ => Some(DiffFile {
                path: path.clone(),
                status: if old.is_none() {
                    DiffFileStatus::Added
                } else if new.is_none() {
                    DiffFileStatus::Deleted
                } else {
                    DiffFileStatus::Modified
                },
                additions: 0,
                deletions: 0,
                hunks: vec![],
                metadata_change: Some(format!(
                    "Source metadata changed: before={}; after={}. Text line statistics are unavailable for this change.",
                    describe_diff_content(old),
                    describe_diff_content(new)
                )),
            }),
        };
        if let Some(file) = file {
            report.total_additions += file.additions;
            report.total_deletions += file.deletions;
            report.files.push(file);
        }
    }
    Ok(report)
}

fn describe_diff_content(content: Option<&inputs::DiffContent>) -> String {
    match content {
        None => "missing".into(),
        Some(inputs::DiffContent::Text(text)) => {
            format!("text ({} bytes, SHA-256 {})", text.len(), hash(text))
        }
        Some(inputs::DiffContent::Metadata(value)) => value.to_string(),
    }
}

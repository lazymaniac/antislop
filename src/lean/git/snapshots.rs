use super::*;
pub fn snapshot(dir: &Path, message: &str) -> Result<String> {
    let subject = commit_subject(message);
    let index = PathBuf::from(ok(dir, &["rev-parse", "--git-path", "index"])?);
    let index = absolute_index(dir, index);
    let scratch = snapshot_index(&index);
    let result = write_snapshot(dir, &index, &scratch, &subject);
    let _ = std::fs::remove_file(scratch);
    result
}

fn absolute_index(dir: &Path, index: PathBuf) -> PathBuf {
    if index.is_absolute() {
        index
    } else {
        dir.join(index)
    }
}

fn snapshot_index(index: &Path) -> PathBuf {
    index.with_file_name(format!(
        "antislop-snapshot-{}-{}.index",
        std::process::id(),
        crate::id()
    ))
}

fn write_snapshot(dir: &Path, index: &Path, scratch: &Path, message: &str) -> Result<String> {
    if index.exists() {
        std::fs::copy(index, scratch)?;
    }
    with_index(dir, scratch, &["add", "--all", "--", "."])?;
    let tree = with_index(dir, scratch, &["write-tree"])?;
    ok(
        dir,
        &[
            "-c",
            "user.name=antislop",
            "-c",
            "user.email=antislop@localhost",
            "commit-tree",
            &tree,
            "-p",
            "HEAD",
            "-m",
            message,
        ],
    )
}

fn with_index(dir: &Path, index: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_INDEX_FILE", index)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned());
    }
    Err(Error::Invalid(format!(
        "git {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr).trim()
    )))
}

/// Commits exactly `files` as they are in the working tree, whatever else
/// the user has staged or changed.
pub fn commit_paths(dir: &Path, files: &[String], message: &str) -> Result<String> {
    let subject = commit_subject(message);
    let message = subject.as_str();
    let mut add = vec!["--literal-pathspecs", "add", "--all", "--"];
    add.extend(files.iter().map(String::as_str));
    ok(dir, &add)?;
    commit_staged_paths(dir, files, message)
}

fn commit_staged_paths(dir: &Path, files: &[String], message: &str) -> Result<String> {
    if staged_paths_clean(dir, files)? {
        return head(dir);
    }
    commit_only_paths(dir, files, message)?;
    head(dir)
}

fn staged_paths_clean(dir: &Path, files: &[String]) -> Result<bool> {
    let mut staged = vec![
        "--literal-pathspecs",
        "diff",
        "--cached",
        "--quiet",
        "HEAD",
        "--",
    ];
    staged.extend(files.iter().map(String::as_str));
    Ok(git(dir, &staged)?.status.success())
}

fn commit_only_paths(dir: &Path, files: &[String], message: &str) -> Result<()> {
    let mut commit = vec![
        "--literal-pathspecs",
        "-c",
        "user.name=antislop",
        "-c",
        "user.email=antislop@localhost",
        "commit",
        "--quiet",
        "--only",
        "-m",
        message,
        "--",
    ];
    commit.extend(files.iter().map(String::as_str));
    ok(dir, &commit)?;
    Ok(())
}

/// Paths with uncommitted changes, untracked ones included.
pub fn uncommitted(dir: &Path) -> Result<Vec<String>> {
    // Not `ok()`: trimming would eat the leading space of " M path".
    let status = git(dir, &["status", "--porcelain=v1", "-z", "--no-renames"])?;
    crate::require(
        status.status.success(),
        &format!(
            "git status: {}",
            String::from_utf8_lossy(&status.stderr).trim()
        ),
    )?;
    Ok(String::from_utf8_lossy(&status.stdout)
        .split('\0')
        .filter(|entry| entry.len() > 3)
        .map(|entry| entry[3..].to_owned())
        .collect())
}

/// Stable identity of the integrated files, including uncommitted content.
/// Git's copied index reuses unchanged objects; no repository text enters SQLite.
pub fn integrated_fingerprint(project: &Path) -> Result<String> {
    let snapshot = snapshot(project, "requirements integrated snapshot")?;
    ok(project, &["rev-parse", &format!("{snapshot}^{{tree}}")])
}

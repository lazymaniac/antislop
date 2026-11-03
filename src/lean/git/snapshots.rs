use super::*;
pub fn snapshot(dir: &Path, message: &str) -> Result<String> {
    snapshot_with_artifacts(dir, &[], message)
}

/// Include explicit artifact outputs in a scratch index, keeping the user's
/// real index untouched even when those documents are ignored.
pub(crate) fn snapshot_with_artifacts(
    dir: &Path,
    outputs: &[String],
    message: &str,
) -> Result<String> {
    let files = super::artifacts::artifact_files(dir, outputs, false)?;
    let subject = commit_subject(message);
    let index = PathBuf::from(ok(dir, &["rev-parse", "--git-path", "index"])?);
    let index = absolute_index(dir, index);
    let scratch = snapshot_index(&index);
    let result = write_snapshot(dir, &index, &scratch, &subject, &files);
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

fn write_snapshot(
    dir: &Path,
    index: &Path,
    scratch: &Path,
    message: &str,
    outputs: &[String],
) -> Result<String> {
    if index.exists() {
        std::fs::copy(index, scratch)?;
    }
    stage_source(dir, Some(scratch))?;
    if !outputs.is_empty() {
        let mut args = vec!["--literal-pathspecs", "add", "--force", "--"];
        args.extend(outputs.iter().map(String::as_str));
        with_index(dir, scratch, &args)?;
    }
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

/// Overlay only proven artifact inputs onto an isolated starting tree. Never
/// touch the primary index/HEAD or pull unrelated changes from a producer.
pub(crate) fn base_with_artifacts(
    dir: &Path,
    base: &str,
    artifacts: &std::collections::BTreeMap<String, String>,
) -> Result<String> {
    if artifacts.is_empty() {
        return Ok(base.into());
    }
    let index = absolute_index(
        dir,
        PathBuf::from(ok(dir, &["rev-parse", "--git-path", "index"])?),
    );
    let scratch = snapshot_index(&index);
    let result = (|| {
        with_index(dir, &scratch, &["read-tree", base])?;
        for (path, commit) in artifacts {
            crate::lean::tasks::validate_artifact_output(path)?;
            let entry = ok(dir, &["--literal-pathspecs", "ls-tree", commit, "--", path])?;
            let (info, _) = entry
                .split_once('\t')
                .ok_or_else(|| Error::Invalid("missing artifact tree entry".into()))?;
            let fields = info.split_whitespace().collect::<Vec<_>>();
            crate::require(
                fields.len() == 3 && matches!(fields[0], "100644" | "100755"),
                "artifact input must be a regular blob",
            )?;
            with_index(
                dir,
                &scratch,
                &[
                    "update-index",
                    "--add",
                    "--cacheinfo",
                    fields[0],
                    fields[2],
                    path,
                ],
            )?;
        }
        let tree = with_index(dir, &scratch, &["write-tree"])?;
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
                base,
                "-m",
                "chore: prepare artifact inputs",
            ],
        )
    })();
    let _ = std::fs::remove_file(scratch);
    result
}

// NUL path lists preserve literal filenames without OS argument limits.
// Explicit artifacts are staged after this runtime exclusion.
pub(super) fn stage_source(dir: &Path, index: Option<&Path>) -> Result<()> {
    use std::io::Write;
    use std::process::Stdio;
    let command = || {
        let mut command = Command::new("git");
        command.arg("-C").arg(dir).env("GIT_TERMINAL_PROMPT", "0");
        if let Some(index) = index {
            command.env("GIT_INDEX_FILE", index);
        }
        command
    };
    let exists = command()
        .args(["rev-parse", "--verify", "HEAD"])
        .output()?
        .status
        .success();
    let reset = if exists {
        vec!["reset", "-q", "HEAD", "--", ".antislop"]
    } else {
        vec![
            "rm",
            "--cached",
            "-r",
            "--ignore-unmatch",
            "--",
            ".antislop",
        ]
    };
    let output = command().args(reset).output()?;
    crate::require(
        output.status.success(),
        &String::from_utf8_lossy(&output.stderr),
    )?;
    let paths = command()
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "--deduplicate",
            "-z",
            "--",
            ".",
            ":(top,exclude).antislop",
        ])
        .output()?;
    crate::require(
        paths.status.success(),
        &String::from_utf8_lossy(&paths.stderr),
    )?;
    if paths.stdout.is_empty() {
        return Ok(());
    }
    let mut child = command()
        .args([
            "--literal-pathspecs",
            "add",
            "--all",
            "--pathspec-from-file=-",
            "--pathspec-file-nul",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let written = child
        .stdin
        .take()
        .expect("piped git stdin")
        .write_all(&paths.stdout);
    let output = child.wait_with_output()?;
    crate::require(
        output.status.success(),
        &crate::lean::agent::clip_tail(&String::from_utf8_lossy(&output.stderr), 8000),
    )?;
    written?;
    Ok(())
}

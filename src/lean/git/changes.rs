use super::*;
pub fn diff_stat(dir: &Path, base: &str, result: &str) -> Result<String> {
    ok(dir, &["diff", "--stat", base, result])
}

/// The change from `base` to `rev`, or to the working tree when `rev` is
/// `None`, as a unified patch with unquoted UTF-8 paths and renames.
pub fn patch(dir: &Path, base: &str, rev: Option<&str>) -> Result<String> {
    let mut args = vec![
        "-c",
        "core.quotePath=false",
        "diff",
        "--no-color",
        "--no-ext-diff",
        "-M",
        base,
    ];
    args.extend(rev);
    ok(dir, &args)
}

pub fn changed_files(dir: &Path, base: &str, result: &str) -> Result<Vec<String>> {
    changed_paths(dir, base, result, None)
}

/// Paths with identical content, type and mode in two snapshots. Compare
/// snapshots rather than the live tree, so a task's starting protection is
/// stable across its turns and resumes. Literal pathspecs keep filenames
/// containing glob characters from widening the comparison.
pub fn matching_paths(
    dir: &Path,
    left: &str,
    right: &str,
    paths: &[String],
) -> Result<Vec<String>> {
    if paths.is_empty() {
        return Ok(vec![]);
    }
    let changed: std::collections::HashSet<String> = changed_paths(dir, left, right, Some(paths))?
        .into_iter()
        .collect();
    Ok(paths
        .iter()
        .filter(|path| !changed.contains(*path))
        .cloned()
        .collect())
}

fn changed_paths(
    dir: &Path,
    left: &str,
    right: &str,
    paths: Option<&[String]>,
) -> Result<Vec<String>> {
    let mut args = vec![
        "--literal-pathspecs",
        "diff",
        "--no-renames",
        "--name-only",
        "-z",
        left,
        right,
        "--",
    ];
    if let Some(paths) = paths {
        args.extend(paths.iter().map(String::as_str));
    }
    let output = git(dir, &args)?;
    parse_changed_paths(output)
}

fn parse_changed_paths(output: std::process::Output) -> Result<Vec<String>> {
    crate::require(
        output.status.success(),
        &format!(
            "git diff: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    )?;
    Ok(std::str::from_utf8(&output.stdout)
        .map_err(|error| Error::Invalid(format!("git paths: {error}")))?
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
        .collect())
}

/// One commit holding everything from `base` to `result`, titled `message`.
pub fn squash(worktree: &Path, base: &str, message: &str) -> Result<String> {
    let subject = commit_subject(message);
    let message = subject.as_str();
    let tree = ok(worktree, &["rev-parse", "HEAD^{tree}"])?;
    ok(
        worktree,
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
            message,
        ],
    )
}

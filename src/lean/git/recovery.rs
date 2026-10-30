use super::*;
use std::path::Component;

/// Only changed source bytes, not the entire repository. A dirty file can
/// change again while `git status` continues to show the same `M` entry.
pub fn recovery_fingerprint(project: &Path, scope: &[String]) -> Result<String> {
    let repository = is_repository(project)?;
    let (paths, mut observed) = scope_observations(project, scope, repository)?;
    if repository && !paths.is_empty() {
        append_repository_state(project, &paths, &mut observed)?;
    }
    Ok(crate::hash(observed.join("\0").as_bytes()))
}

fn is_repository(project: &Path) -> Result<bool> {
    let probe = git(project, &["rev-parse", "--is-inside-work-tree"])?;
    if probe.status.success() {
        return Ok(true);
    }
    let error = String::from_utf8_lossy(&probe.stderr);
    if !error.contains("not a git repository") {
        return Err(Error::Invalid(format!(
            "git recovery observation: {}",
            error.trim()
        )));
    }
    Ok(false)
}

fn scope_observations(
    project: &Path,
    scope: &[String],
    repository: bool,
) -> Result<(Vec<String>, Vec<String>)> {
    let roots = if scope.is_empty() {
        vec![".".to_owned()]
    } else {
        scope.to_vec()
    };
    let mut paths = Vec::new();
    let mut observed = Vec::new();
    for root in roots {
        let path = Path::new(&root);
        if repository
            && !path.is_absolute()
            && !path.components().any(|part| part == Component::ParentDir)
        {
            paths.push(root.to_owned());
        } else {
            observed.push(observe_non_repository_path(project, path)?);
        }
    }
    Ok((paths, observed))
}

fn observe_non_repository_path(project: &Path, path: &Path) -> Result<String> {
    let path = project.join(path);
    let bytes = if path.is_file() {
        std::fs::read(&path)?
    } else {
        format!("unversioned directory or missing path: {}", path.display()).into_bytes()
    };
    Ok(crate::hash(&bytes))
}

fn append_repository_state(
    project: &Path,
    paths: &[String],
    observed: &mut Vec<String>,
) -> Result<()> {
    append_head_and_diffs(project, paths, observed)?;
    append_untracked(project, paths, observed)
}

fn append_head_and_diffs(
    project: &Path,
    paths: &[String],
    observed: &mut Vec<String>,
) -> Result<()> {
    let head = git(project, &["rev-parse", "--verify", "--quiet", "HEAD"])?;
    if !head.status.success() && !(head.status.code() == Some(1) && head.stderr.is_empty()) {
        return Err(Error::Invalid(format!(
            "git recovery HEAD: {}",
            String::from_utf8_lossy(&head.stderr).trim()
        )));
    }
    observed.push(format!("head:{}", String::from_utf8_lossy(&head.stdout)));
    for staged in [false, true] {
        let mut args = vec!["diff", "--no-ext-diff", "--no-textconv", "--binary"];
        if staged {
            args.push("--cached");
        }
        args.push("--");
        args.extend(paths.iter().map(String::as_str));
        observed.push(crate::hash(&raw_git(project, &args)?));
    }
    Ok(())
}

fn append_untracked(project: &Path, paths: &[String], observed: &mut Vec<String>) -> Result<()> {
    let mut args = vec!["ls-files", "--others", "--exclude-standard", "-z", "--"];
    args.extend(paths.iter().map(String::as_str));
    let untracked = raw_git(project, &args)?;
    for path in untracked
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
    {
        let path = std::str::from_utf8(path)
            .map_err(|error| Error::Invalid(format!("git recovery path: {error}")))?;
        observed.push(path.to_owned());
        observed.push(crate::hash(&std::fs::read(project.join(path))?));
    }
    Ok(())
}

fn raw_git(project: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = git(project, args)?;
    if !output.status.success() {
        return Err(Error::Invalid(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirty_content_changes_are_evidence_even_when_status_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        ok(dir.path(), &["init", "-b", "main"]).unwrap();
        recovery_fingerprint(dir.path(), &[".".into()]).unwrap();
        std::fs::write(dir.path().join("source"), "baseline\n").unwrap();
        ok(dir.path(), &["add", "source"]).unwrap();
        ok(
            dir.path(),
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-m",
                "baseline",
            ],
        )
        .unwrap();
        std::fs::write(dir.path().join("source"), "first\n").unwrap();
        let first = recovery_fingerprint(dir.path(), &[".".into()]).unwrap();
        let status = ok(dir.path(), &["status", "--porcelain"]).unwrap();
        std::fs::write(dir.path().join("source"), "second\n").unwrap();
        assert_eq!(status, ok(dir.path(), &["status", "--porcelain"]).unwrap());
        assert_ne!(
            first,
            recovery_fingerprint(dir.path(), &[".".into()]).unwrap()
        );
        std::fs::write(dir.path().join("untracked"), "new input").unwrap();
        let first = recovery_fingerprint(dir.path(), &[".".into()]).unwrap();
        std::fs::write(dir.path().join("untracked"), "corrected input").unwrap();
        assert_ne!(
            first,
            recovery_fingerprint(dir.path(), &[".".into()]).unwrap()
        );
    }
}

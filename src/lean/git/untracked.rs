use super::*;
use std::io::Write;

/// Check the primary checkout before launching a writer, including ignored,
/// untracked, and tracked-but-deleted files. Delivery repeats the check.
pub(crate) fn require_new_outputs(project: &Path, outputs: &[String]) -> Result<()> {
    for output in outputs {
        crate::lean::tasks::validate_artifact_output(output)?;
        crate::require(
            ok(project, &["--literal-pathspecs", "ls-files", "--", output])?.is_empty(),
            &format!("untracked output already tracked: {output}"),
        )?;
        let mut path = project.to_owned();
        for component in Path::new(output).components() {
            path.push(component);
            match path.symlink_metadata() {
                Ok(metadata) => crate::require(
                    path != project.join(output)
                        && metadata.is_dir()
                        && !metadata.file_type().is_symlink(),
                    &format!("untracked output already exists or has an unsafe parent: {output}"),
                )?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(())
}

/// Reject edits/deletions of existing files before verification, even inside
/// the isolated writer checkout. Check against the original task base.
pub(crate) fn require_added_files(
    project: &Path,
    base: &str,
    change: &str,
    expected: &std::collections::BTreeMap<String, String>,
) -> Result<()> {
    for path in changed_files(project, base, change)? {
        crate::require(
            ok(
                project,
                &["--literal-pathspecs", "ls-tree", base, "--", &path],
            )?
            .is_empty()
                || expected.get(&path).is_some_and(|commit| {
                matches!((super::artifact_update::artifact_bytes(project,commit,&path),super::artifact_update::artifact_bytes(project,base,&path)),(Ok(expected),Ok(actual)) if expected==actual)
            }),
            &format!("untracked delivery cannot modify an existing file: {path}"),
        )?;
    }
    Ok(())
}

/// Deliver only newly added regular files. Never use the user's index, merge
/// machinery, or an overwriting filesystem operation for explicit untracked output.
pub fn deliver_untracked(project: &Path, base: &str, change: &str) -> Result<Integration> {
    let paths = changed_files(project, base, change)?;
    let mut blobs = Vec::new();
    let mut conflicts = Vec::new();
    for path in &paths {
        let relative = Path::new(path);
        crate::require(
            relative
                .components()
                .all(|p| matches!(p, std::path::Component::Normal(_))),
            "invalid artifact path",
        )?;
        let entry = ok(project, &["ls-tree", change, "--", path])?;
        let added = ok(project, &["ls-tree", base, "--", path])?.is_empty();
        let tracked = ok(project, &["ls-files", "--", path])?;
        let target = project.join(path);
        if !added
            || !tracked.is_empty()
            || target.symlink_metadata().is_ok()
            || !(entry.starts_with("100644 ") || entry.starts_with("100755 "))
        {
            conflicts.push(path.clone());
            continue;
        }
        let output = git(project, &["show", &format!("{change}:{path}")])?;
        crate::require(
            output.status.success(),
            &String::from_utf8_lossy(&output.stderr),
        )?;
        blobs.push((target, output.stdout));
    }
    if !conflicts.is_empty() {
        return Ok(Integration::UserEdits(conflicts));
    }
    let mut created = Vec::new();
    let result = (|| -> Result<()> {
        for (target, bytes) in blobs {
            ensure_parents(project, target.parent().unwrap())?;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)?;
            created.push(target);
            file.write_all(&bytes)?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        for path in created {
            let _ = std::fs::remove_file(path);
        }
        return Err(error);
    }
    Ok(Integration::Applied)
}

fn ensure_parents(project: &Path, parent: &Path) -> Result<()> {
    let mut directory = project.to_owned();
    for component in parent
        .strip_prefix(project)
        .map_err(|e| Error::Invalid(e.to_string()))?
        .components()
    {
        directory.push(component);
        match directory.symlink_metadata() {
            Ok(metadata) => crate::require(
                metadata.is_dir() && !metadata.file_type().is_symlink(),
                "artifact parent is not a regular directory",
            )?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&directory)?
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn new_outputs_reject_existing_ignored_tracked_deleted_and_symlink_paths() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        ok(root, &["init", "-q"]).unwrap();
        std::fs::write(root.join(".gitignore"), ".antislop/\n").unwrap();
        std::fs::write(root.join("tracked.md"), "existing").unwrap();
        commit_all(root, "base").unwrap();
        std::fs::remove_file(root.join("tracked.md")).unwrap();
        std::fs::create_dir(root.join(".antislop")).unwrap();
        std::fs::write(root.join(".antislop/existing.md"), "user content").unwrap();
        std::fs::write(root.join("untracked.md"), "user content").unwrap();
        std::os::unix::fs::symlink("missing.md", root.join("link.md")).unwrap();
        std::os::unix::fs::symlink(".antislop", root.join("alias")).unwrap();
        let index = std::fs::read(root.join(".git/index")).unwrap();
        for output in [
            "tracked.md",
            ".antislop/existing.md",
            "untracked.md",
            "link.md",
            "alias/new.md",
        ] {
            assert!(
                require_new_outputs(root, &[output.into()]).is_err(),
                "{output}"
            );
        }
        require_new_outputs(root, &[".antislop/new.md".into()]).unwrap();
        require_new_outputs(root, &["new/directory/report.md".into()]).unwrap();
        assert!(!root.join("new").exists());
        assert_eq!(std::fs::read(root.join(".git/index")).unwrap(), index);
        assert_eq!(
            std::fs::read_to_string(root.join("untracked.md")).unwrap(),
            "user content"
        );
    }

    #[test]
    fn untracked_changes_reject_edits_and_deletions_against_original_base() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        ok(root, &["init", "-q"]).unwrap();
        std::fs::write(root.join("existing.md"), "original").unwrap();
        let base = commit_all(root, "base").unwrap();
        std::fs::write(root.join("new.md"), "new report").unwrap();
        let checkpoint = commit_all(root, "checkpoint").unwrap();
        std::fs::write(root.join("new.md"), "corrected report").unwrap();
        let corrected = commit_all(root, "corrected").unwrap();
        require_added_files(root, &base, &checkpoint, &Default::default()).unwrap();
        require_added_files(root, &base, &corrected, &Default::default()).unwrap();
        for content in [Some("changed existing report"), None] {
            if let Some(content) = content {
                std::fs::write(root.join("existing.md"), content).unwrap();
            } else {
                std::fs::remove_file(root.join("existing.md")).unwrap();
            }
            let change = commit_all(root, "invalid").unwrap();
            let error = require_added_files(root, &base, &change, &Default::default())
                .unwrap_err()
                .to_string();
            assert!(error.contains("existing.md"), "{error}");
        }
    }
    #[test]
    fn delivery_preserves_head_index_and_existing_user_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        ok(root, &["init", "-q"]).unwrap();
        std::fs::write(root.join("source"), "original").unwrap();
        let base = commit_all(root, "base").unwrap();
        std::fs::write(root.join("report.md"), "all 16 findings").unwrap();
        let change = snapshot(root, "artifact").unwrap();
        std::fs::remove_file(root.join("report.md")).unwrap();
        std::fs::write(root.join("source"), "staged user edit").unwrap();
        ok(root, &["add", "source"]).unwrap();
        let index = std::fs::read(root.join(".git/index")).unwrap();
        assert_eq!(
            deliver_untracked(root, &base, &change).unwrap(),
            Integration::Applied
        );
        assert_eq!(head(root).unwrap(), base);
        assert_eq!(std::fs::read(root.join(".git/index")).unwrap(), index);
        assert_eq!(ok(root, &["ls-files", "--", "report.md"]).unwrap(), "");
        std::fs::write(root.join("report.md"), "user report").unwrap();
        assert_eq!(
            deliver_untracked(root, &base, &change).unwrap(),
            Integration::UserEdits(vec!["report.md".into()])
        );
        assert_eq!(
            std::fs::read_to_string(root.join("report.md")).unwrap(),
            "user report"
        );
    }
}

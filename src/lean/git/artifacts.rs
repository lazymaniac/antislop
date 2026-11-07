use super::*;

/// Only named regular documents bypass ignore rules in an isolated worktree.
/// Other ignored runtime files must never enter its checkpoint or reviewer view.
pub(crate) fn commit_artifacts(
    worktree: &Path,
    outputs: &[String],
    purpose: CommitPurpose,
    require_all: bool,
) -> Result<String> {
    commit_artifact_files(worktree, outputs, commit_subject(purpose), require_all)
}

pub(crate) fn commit_artifacts_with_message(
    worktree: &Path,
    outputs: &[String],
    message: &str,
    require_all: bool,
) -> Result<String> {
    commit_artifact_files(worktree, outputs, message, require_all)
}

fn commit_artifact_files(
    worktree: &Path,
    outputs: &[String],
    message: &str,
    require_all: bool,
) -> Result<String> {
    let files = artifact_files(worktree, outputs, require_all)?;
    super::snapshots::stage_source(worktree, None)?;
    if !files.is_empty() {
        let mut args = vec!["--literal-pathspecs", "add", "--force", "--"];
        args.extend(files.iter().map(String::as_str));
        ok(worktree, &args)?;
    }
    super::worktree::commit_staged(worktree, message)
}

pub(super) fn artifact_files(
    directory: &Path,
    outputs: &[String],
    require_all: bool,
) -> Result<Vec<String>> {
    let mut files = Vec::new();
    for output in outputs {
        crate::lean::tasks::validate_artifact_output(output)?;
        let path = directory.join(output);
        match path.symlink_metadata() {
            Ok(metadata) => crate::require(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                &format!("artifact output must be a regular file: {output}"),
            )?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                crate::require(!require_all, &format!("artifact output missing: {output}"))?;
                continue;
            }
            Err(error) => return Err(error.into()),
        }
        let mut parent = directory.to_path_buf();
        for component in Path::new(output).parent().unwrap().components() {
            parent.push(component);
            let metadata = parent.symlink_metadata()?;
            crate::require(
                metadata.is_dir() && !metadata.file_type().is_symlink(),
                &format!("artifact parent must be a regular directory: {output}"),
            )?;
        }
        files.push(output.clone());
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignored_artifact_capture_refuses_symlinks_and_directories_before_staging() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        std::fs::create_dir(&project).unwrap();
        ok(&project, &["init", "-q"]).unwrap();
        std::fs::write(project.join(".gitignore"), ".antislop/\n").unwrap();
        let base = commit_all(&project, CommitPurpose::TestFixture).unwrap();
        let index = std::fs::read(project.join(".git/index")).unwrap();
        std::fs::create_dir(project.join(".antislop")).unwrap();
        let outside = directory.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("report.md"), "external document").unwrap();
        std::os::unix::fs::symlink(outside.join("report.md"), project.join(".antislop/link.md"))
            .unwrap();
        std::os::unix::fs::symlink(&outside, project.join(".antislop/alias")).unwrap();
        std::fs::create_dir(project.join(".antislop/directory.md")).unwrap();
        for output in [
            ".antislop/link.md",
            ".antislop/alias/report.md",
            ".antislop/directory.md",
        ] {
            let error =
                commit_artifacts(&project, &[output.into()], CommitPurpose::TestFixture, true)
                    .unwrap_err()
                    .to_string();
            assert!(error.contains("regular"), "{output}: {error}");
            assert_eq!(head(&project).unwrap(), base);
            assert_eq!(std::fs::read(project.join(".git/index")).unwrap(), index);
        }
    }
}

//! Exclusive publication: an update never truncates or renames over a user file.
//! The displaced inode is retained in private runtime state, including writes
//! through an editor's already-open descriptor. A raced replacement is a conflict.
use super::*;
use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
mod index;
mod parent;
use parent::Parent;

pub(crate) fn artifact_bytes(project: &Path, commit: &str, path: &str) -> Result<Vec<u8>> {
    crate::lean::tasks::validate_artifact_output(path)?;
    let entry = ok(
        project,
        &["--literal-pathspecs", "ls-tree", commit, "--", path],
    )?;
    crate::require(
        entry.starts_with("100644 ") || entry.starts_with("100755 "),
        &format!("artifact is not a regular recorded file: {path}"),
    )?;
    let output = git(project, &["show", &format!("{commit}:{path}")])?;
    crate::require(
        output.status.success(),
        &String::from_utf8_lossy(&output.stderr),
    )?;
    Ok(output.stdout)
}

pub(crate) fn require_owned_outputs(
    project: &Path,
    outputs: &[String],
    expected: &BTreeMap<String, String>,
) -> Result<()> {
    for path in outputs {
        if let Some(commit) = expected.get(path) {
            require_untracked(project, path)?;
            let (parent, name) = Parent::at(project, path, false)?;
            crate::require(
                parent.matches(&name, &artifact_bytes(project, commit, path)?)?,
                &format!("delivered artifact was changed or removed by the user: {path}"),
            )?;
        } else {
            super::untracked::require_new_outputs(project, std::slice::from_ref(path))?;
        }
    }
    Ok(())
}

pub(crate) fn deliver_owned_artifacts(
    project: &Path,
    base: &str,
    change: &str,
    original: &BTreeMap<String, String>,
    expected: &BTreeMap<String, String>,
    backups: &Path,
) -> Result<Integration> {
    let _index = index::Lease::acquire(project)?;
    let paths = changed_files(project, base, change)?;
    super::untracked::require_added_files(project, base, change, original)?;
    if require_owned_outputs(project, &paths, expected).is_err() {
        return Ok(Integration::UserEdits(paths));
    }
    // Read and validate all blobs before mutating a destination.
    let blobs = paths
        .iter()
        .map(|path| Ok((path, artifact_bytes(project, change, path)?)))
        .collect::<Result<Vec<_>>>()?;
    for (path, bytes) in blobs {
        let old = expected
            .get(path)
            .map(|commit| artifact_bytes(project, commit, path))
            .transpose()?;
        publish(project, path, &bytes, old.as_deref(), backups, |_| {})?;
    }
    Ok(Integration::Applied)
}

fn require_untracked(project: &Path, path: &str) -> Result<()> {
    crate::require(
        ok(project, &["--literal-pathspecs", "ls-files", "--", path])?.is_empty(),
        &format!("artifact is now tracked by the user: {path}"),
    )
}

fn publish(
    project: &Path,
    path: &str,
    bytes: &[u8],
    expected: Option<&[u8]>,
    backups: &Path,
    mut observe: impl FnMut(bool),
) -> Result<()> {
    let (parent, name) = Parent::at(project, path, true)?;
    if expected.is_some_and(|old| old == bytes) && parent.matches(&name, bytes)? {
        return Ok(());
    }
    std::fs::create_dir_all(backups)?;
    let backup_metadata = backups.symlink_metadata()?;
    crate::require(
        backup_metadata.is_dir() && !backup_metadata.file_type().is_symlink(),
        "artifact backup directory must not be a symlink",
    )?;
    let temporary = tempfile::Builder::new()
        .prefix("artifact-")
        .tempdir_in(backups)?;
    let private = Parent::open(temporary.path())?;
    let mut replacement = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .open(temporary.path().join("replacement"))?;
    replacement.write_all(bytes)?;
    replacement.sync_all()?;
    observe(false);
    let saved = if let Some(old) = expected {
        // Move first, then check the actual displaced file; a check followed
        // by overwriting rename would lose an intervening user save.
        parent.rename(&name, &private, "previous")?;
        let saved = temporary.keep();
        let checked = (|| -> Result<()> {
            crate::require(
                private.matches("previous", old)?,
                "artifact contents or type changed",
            )?;
            let permissions = private.read("previous")?.metadata()?.permissions();
            replacement.set_permissions(permissions)?;
            Ok(())
        })();
        if let Err(error) = checked {
            let recovery = recover(&private, &parent, &name, &saved);
            return Err(Error::Invalid(format!(
                "artifact changed during update: {path}: {error}; {recovery}"
            )));
        }
        Some(saved)
    } else {
        None
    };
    observe(true);
    // linkat has no replace mode: a user-created target always wins.
    if let Err(error) = private.link("replacement", &parent, &name) {
        if let Some(saved) = &saved {
            let recovery = recover(&private, &parent, &name, saved);
            return Err(Error::Invalid(format!(
                "could not publish artifact {path}: {error}; {recovery}"
            )));
        }
        return Err(error);
    }
    if saved.is_some() {
        let _ = private.unlink("replacement");
    }
    Ok(())
}

fn recover(private: &Parent, parent: &Parent, name: &str, saved: &Path) -> String {
    if private.restore("previous", parent, name).is_ok() {
        "original restored without replacing a concurrent file".into()
    } else {
        format!(
            "prior file retained at {}",
            saved.join("previous").display()
        )
    }
}

#[cfg(test)]
mod tests;

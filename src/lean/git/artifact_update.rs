//! Exclusive publication: an update never truncates or renames over a user file.
//! The displaced inode is retained in private runtime state, including writes
//! through an editor's already-open descriptor. A raced replacement is a conflict.
use super::*;
use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::OpenOptionsExt;
mod index;
pub(crate) mod parent;
use parent::Parent;

/// Spool a recorded Git blob directly to a file; memory is independent of blob size.
pub(crate) fn artifact_file(project: &Path, commit: &str, path: &str) -> Result<std::fs::File> {
    crate::lean::tasks::validate_artifact_output(path)?;
    let entry = ok(
        project,
        &["--literal-pathspecs", "ls-tree", commit, "--", path],
    )?;
    crate::require(
        entry.starts_with("100644 ") || entry.starts_with("100755 "),
        &format!("artifact is not a regular recorded file: {path}"),
    )?;
    let mut file = tempfile::tempfile()?;
    let mut errors = tempfile::tempfile()?;
    let status = std::process::Command::new("git")
        .args(["show", &format!("{commit}:{path}")])
        .current_dir(project)
        .stdin(std::process::Stdio::null())
        .stdout(file.try_clone()?)
        .stderr(errors.try_clone()?)
        .status()?;
    errors.seek(SeekFrom::Start(
        errors.metadata()?.len().saturating_sub(8192),
    ))?;
    let mut diagnostic = String::new();
    errors.read_to_string(&mut diagnostic)?;
    crate::require(status.success(), &diagnostic)?;
    file.seek(SeekFrom::Start(0))?;
    Ok(file)
}

pub(super) fn blobs_match(left: &std::fs::File, right: &std::fs::File) -> Result<bool> {
    if left.metadata()?.len() != right.metadata()?.len() {
        return Ok(false);
    }
    let mut left = std::io::BufReader::new(left.try_clone()?);
    let mut right = std::io::BufReader::new(right.try_clone()?);
    left.seek(SeekFrom::Start(0))?;
    right.seek(SeekFrom::Start(0))?;
    let mut a = [0; 64 * 1024];
    let mut b = [0; 64 * 1024];
    loop {
        let n = left.read(&mut a)?;
        if n == 0 {
            return Ok(right.read(&mut b[..1])? == 0);
        }
        if right.read_exact(&mut b[..n]).is_err() || a[..n] != b[..n] {
            return Ok(false);
        }
    }
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
                parent.matches(&name, &artifact_file(project, commit, path)?)?,
                &format!("delivered artifact was changed or removed by the user: {path}"),
            )?;
        } else {
            super::untracked::require_new_outputs(project, std::slice::from_ref(path))?;
        }
    }
    Ok(())
}

type IndexStamp = (u64, u64, u64, i64, i64);

pub(crate) struct PreparedArtifacts {
    paths: Vec<String>,
    blobs: Vec<(String, Publication, Option<std::fs::File>)>,
    index: PathBuf,
    index_stamp: Option<IndexStamp>,
    conflict: bool,
}

fn index_stamp(lock: &Path) -> Result<Option<IndexStamp>> {
    use std::os::unix::fs::MetadataExt;
    let index = lock.with_file_name("index");
    match std::fs::metadata(index) {
        Ok(m) => Ok(Some((m.dev(), m.ino(), m.len(), m.mtime(), m.mtime_nsec()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub(crate) fn prepare_owned_artifacts(
    project: &Path,
    base: &str,
    change: &str,
    original: &BTreeMap<String, String>,
    expected: &BTreeMap<String, String>,
    backups: &Path,
) -> Result<PreparedArtifacts> {
    let index = index::lock_path(project)?;
    let before = index_stamp(&index)?;
    let paths = changed_files(project, base, change)?;
    super::untracked::require_added_files(project, base, change, original)?;
    let conflict = require_owned_outputs(project, &paths, expected).is_err();
    let blobs = if conflict {
        Vec::new()
    } else {
        paths
            .iter()
            .map(|path| {
                let current = artifact_file(project, change, path)?;
                let old = expected
                    .get(path)
                    .map(|commit| artifact_file(project, commit, path))
                    .transpose()?;
                let staged = prepare_publication(&current, backups)?;
                Ok((path.clone(), staged, old))
            })
            .collect::<Result<Vec<_>>>()?
    };
    let conflict = conflict || before != index_stamp(&index)?;
    Ok(PreparedArtifacts {
        paths,
        blobs,
        index,
        index_stamp: before,
        conflict,
    })
}

impl PreparedArtifacts {
    /// Publication performs only fd-relative filesystem operations. Git and
    /// full blob preparation have completed before any shared scope lock.
    pub(crate) fn publish(self, project: &Path) -> Result<Integration> {
        let _index = index::Lease::acquire_at(self.index.clone())?;
        if self.conflict || index_stamp(&self.index)? != self.index_stamp {
            return Ok(Integration::UserEdits(self.paths));
        }
        for (path, _, old) in &self.blobs {
            if let Some(old) = old {
                let (parent, name) = Parent::at(project, path, false)?;
                if !parent.matches(&name, old)? {
                    return Ok(Integration::UserEdits(self.paths));
                }
            } else if project.join(path).symlink_metadata().is_ok() {
                return Ok(Integration::UserEdits(self.paths));
            }
        }
        for (path, staged, old) in self.blobs {
            publish_prepared(project, &path, old.as_ref(), staged, |_| {})?;
        }
        Ok(Integration::Applied)
    }
}

#[cfg(test)]
fn deliver_owned_artifacts(
    project: &Path,
    base: &str,
    change: &str,
    original: &BTreeMap<String, String>,
    expected: &BTreeMap<String, String>,
    backups: &Path,
) -> Result<Integration> {
    prepare_owned_artifacts(project, base, change, original, expected, backups)?.publish(project)
}

fn require_untracked(project: &Path, path: &str) -> Result<()> {
    crate::require(
        ok(project, &["--literal-pathspecs", "ls-files", "--", path])?.is_empty(),
        &format!("artifact is now tracked by the user: {path}"),
    )
}

struct Publication {
    temporary: tempfile::TempDir,
    replacement: std::fs::File,
}

fn prepare_publication(bytes: &std::fs::File, backups: &Path) -> Result<Publication> {
    std::fs::create_dir_all(backups)?;
    let backup_metadata = backups.symlink_metadata()?;
    crate::require(
        backup_metadata.is_dir() && !backup_metadata.file_type().is_symlink(),
        "artifact backup directory must not be a symlink",
    )?;
    let temporary = tempfile::Builder::new()
        .prefix("artifact-")
        .tempdir_in(backups)?;
    let mut replacement = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o644)
        .open(temporary.path().join("replacement"))?;
    let mut source = bytes.try_clone()?;
    source.seek(SeekFrom::Start(0))?;
    std::io::copy(&mut source, &mut replacement)?;
    replacement.sync_all()?;
    Ok(Publication {
        temporary,
        replacement,
    })
}

fn publish_prepared(
    project: &Path,
    path: &str,
    expected: Option<&std::fs::File>,
    staged: Publication,
    mut observe: impl FnMut(bool),
) -> Result<()> {
    let Publication {
        temporary,
        replacement,
    } = staged;
    let (parent, name) = Parent::at(project, path, true)?;
    if expected
        .map(|old| blobs_match(old, &replacement))
        .transpose()?
        .unwrap_or(false)
        && parent.matches(&name, &replacement)?
    {
        return Ok(());
    }
    let private = Parent::open(temporary.path())?;
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
fn publish(
    project: &Path,
    path: &str,
    bytes: &[u8],
    expected: Option<&[u8]>,
    backups: &Path,
    observe: impl FnMut(bool),
) -> Result<()> {
    use std::io::Write;
    fn file(bytes: &[u8]) -> Result<std::fs::File> {
        let mut file = tempfile::tempfile()?;
        file.write_all(bytes)?;
        Ok(file)
    }
    let bytes = file(bytes)?;
    let expected = expected.map(file).transpose()?;
    let staged = prepare_publication(&bytes, backups)?;
    publish_prepared(project, path, expected.as_ref(), staged, observe)
}

#[cfg(test)]
mod tests;

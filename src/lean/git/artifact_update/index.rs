//! Serialize the untracked check/publication with normal Git index writers.
//! Only an exclusive lock is created; index bytes and HEAD are never changed.
use super::super::*;
use std::fs::{File, OpenOptions};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

pub(super) struct Lease {
    file: File,
    path: PathBuf,
}
impl Lease {
    pub(super) fn acquire(project: &Path) -> Result<Self> {
        let path = PathBuf::from(ok(project, &["rev-parse", "--git-path", "index"])?);
        let path = if path.is_absolute() {
            path
        } else {
            project.join(path)
        };
        let path = path.with_file_name("index.lock");
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| {
                Error::Invalid(format!(
                    "cannot lock index for artifact delivery at {}: {e}",
                    path.display()
                ))
            })?;
        Ok(Self { file, path })
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        if let (Ok(owned), Ok(current)) = (self.file.metadata(), self.path.symlink_metadata())
            && owned.dev() == current.dev()
            && owned.ino() == current.ino()
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

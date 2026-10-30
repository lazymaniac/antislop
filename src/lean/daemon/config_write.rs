use super::*;

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ConfigUpdateStatus {
    Applied,
    AppliedWithWarnings,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ConfigUpdateStage {
    EventCommit,
    DirectorySync,
}

#[derive(Serialize)]
pub(super) struct ConfigUpdateWarning {
    pub(super) stage: ConfigUpdateStage,
    pub(super) message: String,
}

#[derive(Serialize)]
pub(super) struct ConfigUpdateReply {
    pub(super) toml: String,
    pub(super) path: PathBuf,
    pub(super) status: ConfigUpdateStatus,
    pub(super) warnings: Vec<ConfigUpdateWarning>,
}

/// Prepare and flush file contents before acquiring the event store lock.
pub(super) struct PreparedConfigWrite {
    target: PathBuf,
    pub(super) temporary: PathBuf,
    directory: std::fs::File,
    // Keep the advisory lock until publication or cleanup has completed.
    pub(super) lease: std::fs::File,
}

/// Serialize creation/locking with orphan scanning, including the small gap
/// between create_new and locking the new descriptor.
pub(super) struct ConfigDirectoryLease(std::fs::File);

impl ConfigDirectoryLease {
    fn acquire(directory: &std::fs::File) -> Result<Self> {
        use std::os::fd::AsRawFd;
        let lease = Self(directory.try_clone()?);
        // SAFETY: this owned directory descriptor remains open until Drop.
        if unsafe { libc::flock(lease.0.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(lease)
    }
}

impl Drop for ConfigDirectoryLease {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        // SAFETY: the owned descriptor is still open while the lock releases.
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

pub(super) fn config_temporary_prefix(target: &Path) -> String {
    format!(
        ".antislop-config-{}-",
        &crate::hash(target.as_os_str().as_encoded_bytes())[..16]
    )
}

/// Reap only this target's unlocked, user-owned temporary files. Active
/// writers, other configurations and symbolic links are never removed.
pub(super) fn cleanup_config_temporaries(path: &Path) -> Result<usize> {
    let target = path.canonicalize()?;
    let directory = config_directory(&target)?;
    let directory_handle = std::fs::File::open(&directory)?;
    let _lease = ConfigDirectoryLease::acquire(&directory_handle)?;
    let prefix = config_temporary_prefix(&target);
    let mut removed = 0;
    for entry in std::fs::read_dir(&directory)? {
        removed += usize::from(cleanup_temporary(&entry?, &prefix)?);
    }
    sync_directory_if_changed(&directory, removed)?;
    Ok(removed)
}

fn config_directory(target: &Path) -> Result<PathBuf> {
    target
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| Error::Invalid("configuration has no parent directory".into()))
}

fn cleanup_temporary(entry: &std::fs::DirEntry, prefix: &str) -> Result<bool> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = entry.path();
    if !temporary_name_matches(entry, prefix) {
        return Ok(false);
    }
    let Ok(file) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
    else {
        return Ok(false);
    };
    let metadata = file.metadata()?;
    if !owned_unlocked_file(&file, &metadata)? {
        return Ok(false);
    }
    remove_if_same_file(&path, &metadata)
}

fn temporary_name_matches(entry: &std::fs::DirEntry, prefix: &str) -> bool {
    let name = entry.file_name();
    let name = name.to_string_lossy();
    name.starts_with(prefix) && name.ends_with(".tmp")
}

fn owned_unlocked_file(file: &std::fs::File, metadata: &std::fs::Metadata) -> Result<bool> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::MetadataExt;
    // SAFETY: geteuid has no arguments and flock receives an owned fd.
    Ok(metadata.is_file()
        && metadata.uid() == unsafe { libc::geteuid() }
        && unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0)
}

fn remove_if_same_file(path: &Path, metadata: &std::fs::Metadata) -> Result<bool> {
    use std::os::unix::fs::MetadataExt;
    let current = std::fs::symlink_metadata(path)?;
    if current.dev() != metadata.dev() || current.ino() != metadata.ino() {
        return Ok(false);
    }
    std::fs::remove_file(path)?;
    Ok(true)
}

fn sync_directory_if_changed(directory: &Path, removed: usize) -> Result<()> {
    if removed > 0 {
        std::fs::File::open(directory)?.sync_all()?;
    }
    Ok(())
}

fn temporary_path(target: &Path) -> PathBuf {
    target.with_file_name(format!(
        "{}{}.tmp",
        config_temporary_prefix(target),
        uuid::Uuid::new_v4()
    ))
}

impl PreparedConfigWrite {
    pub(super) fn prepare(path: &Path, text: &str) -> Result<Self> {
        let target = path.canonicalize()?;
        let temporary = temporary_path(&target);
        let directory = std::fs::File::open(config_directory(&target)?)?;
        let directory_lease = ConfigDirectoryLease::acquire(&directory)?;
        let mut prepared = Self::create_locked(target, temporary, directory, directory_lease)?;
        prepared.write_text(text)?;
        Ok(prepared)
    }

    fn create_locked(
        target: PathBuf,
        temporary: PathBuf,
        directory: std::fs::File,
        directory_lease: ConfigDirectoryLease,
    ) -> Result<Self> {
        use std::os::unix::fs::OpenOptionsExt;
        let lease = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        let prepared = Self {
            target,
            temporary,
            directory,
            lease,
        };
        prepared.lock_lease()?;
        drop(directory_lease);
        Ok(prepared)
    }

    fn lock_lease(&self) -> Result<()> {
        use std::os::fd::AsRawFd;
        // SAFETY: the owned file descriptor remains open for the prepared write.
        require(
            unsafe { libc::flock(self.lease.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
            "could not lock prepared configuration write",
        )
    }

    fn write_text(&mut self, text: &str) -> Result<()> {
        self.lease
            .set_permissions(std::fs::metadata(&self.target)?.permissions())?;
        self.lease.write_all(text.as_bytes())?;
        self.lease.sync_all()?;
        Ok(())
    }

    pub(super) fn publish(&self) -> Result<()> {
        std::fs::rename(&self.temporary, &self.target)?;
        Ok(())
    }

    pub(super) fn sync_directory(&self) -> Result<()> {
        self.directory.sync_all()?;
        Ok(())
    }
}

impl Drop for PreparedConfigWrite {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.temporary);
    }
}

#[cfg(test)]
pub(super) fn atomic_config_write(path: &Path, text: &str) -> Result<()> {
    let prepared = PreparedConfigWrite::prepare(path, text)?;
    prepared.publish()?;
    prepared.sync_directory()
}

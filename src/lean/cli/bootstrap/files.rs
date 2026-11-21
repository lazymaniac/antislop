use super::*;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::os::fd::AsRawFd;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct Identity {
    device: u64,
    inode: u64,
    bytes: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    mode: u32,
    pub uid: u32,
    pub gid: u32,
    sha256: String,
}

impl Identity {
    pub fn same_contents_and_metadata(&self, other: &Self) -> bool {
        self.bytes == other.bytes
            && self.modified_seconds == other.modified_seconds
            && self.modified_nanoseconds == other.modified_nanoseconds
            && self.mode == other.mode
            && self.uid == other.uid
            && self.gid == other.gid
            && self.sha256 == other.sha256
    }

    pub fn check_executable(&self) -> Result<()> {
        require(
            self.mode & 0o111 != 0 && self.mode & 0o6000 == 0,
            "bootstrap requires an executable without setuid/setgid",
        )
    }
}

pub(super) fn identity(path: &Path) -> Result<Identity> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let before = file.metadata()?;
    require(before.is_file(), "bootstrap requires a regular file")?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    let after = file.metadata()?;
    require(
        before.len() == after.len()
            && before.mtime() == after.mtime()
            && before.mtime_nsec() == after.mtime_nsec()
            && before.ctime() == after.ctime()
            && before.ctime_nsec() == after.ctime_nsec(),
        "file changed while hashing",
    )?;
    Ok(Identity {
        device: after.dev(),
        inode: after.ino(),
        bytes: after.len(),
        modified_seconds: after.mtime(),
        modified_nanoseconds: after.mtime_nsec(),
        mode: after.mode(),
        uid: after.uid(),
        gid: after.gid(),
        sha256: hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    })
}

pub(super) fn copy_synced(source: &Path, target: &Path, expected: &Identity) -> Result<()> {
    #[cfg(test)]
    super::tests::fail("copy", target)?;
    let mut input = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(source)?;
    let metadata = input.metadata()?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(target)?;
    std::io::copy(&mut input, &mut output)?;
    output.set_permissions(metadata.permissions())?;
    output.set_times(fs::FileTimes::new().set_modified(metadata.modified()?))?;
    #[cfg(test)]
    super::tests::fail("copy_before_sync", target)?;
    sync_file(target)?;
    let actual = identity(target)?;
    require(
        actual.same_contents_and_metadata(expected),
        "copy hash or metadata mismatch; partial files retained",
    )
}

pub(super) fn sync_file(path: &Path) -> Result<()> {
    #[cfg(test)]
    super::tests::fail("file_sync", path)?;
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?
        .sync_all()
        .map_err(Into::into)
}

pub(super) fn directory_sync(directory: &Path) -> Result<()> {
    #[cfg(test)]
    super::tests::fail("sync", directory)?;
    File::open(directory)?.sync_all().map_err(Into::into)
}

pub(super) fn lock(parent: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(parent.join(".antislop-bootstrap.lock"))?;
    // SAFETY: owned fd; the stable lock file is never renamed or removed.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(file)
}

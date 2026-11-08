//! Directory-fd operations keep publication away from raced symlinks.
use super::super::*;
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::OpenOptionsExt;

pub(crate) struct Parent {
    directory: File,
}
impl Parent {
    pub(super) fn open(path: &Path) -> Result<Self> {
        Ok(Self {
            directory: OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
                .open(path)?,
        })
    }
    pub(super) fn at(root: &Path, path: &str, create: bool) -> Result<(Self, String)> {
        crate::lean::tasks::validate_artifact_output(path)?;
        let mut parent = Self::open(root)?;
        let relative = Path::new(path);
        for component in relative.parent().unwrap().components() {
            let name = component.as_os_str().to_string_lossy();
            let c = cstring(&name)?;
            // SAFETY: live directory fd and NUL-terminated component; returned fd is owned below.
            let mut fd = unsafe {
                libc::openat(
                    parent.directory.as_raw_fd(),
                    c.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if fd < 0
                && create
                && std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound
            {
                // SAFETY: same live parent/component; no symlink traversal.
                let made =
                    unsafe { libc::mkdirat(parent.directory.as_raw_fd(), c.as_ptr(), 0o755) };
                if made < 0
                    && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists
                {
                    return Err(std::io::Error::last_os_error().into());
                }
                // SAFETY: as above; a raced symlink is rejected by O_NOFOLLOW.
                fd = unsafe {
                    libc::openat(
                        parent.directory.as_raw_fd(),
                        c.as_ptr(),
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                    )
                };
            }
            if fd < 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            // SAFETY: openat returned a new, valid, owned fd.
            parent = Self {
                directory: unsafe { File::from_raw_fd(fd) },
            };
        }
        Ok((
            parent,
            relative.file_name().unwrap().to_string_lossy().into_owned(),
        ))
    }
    /// Open a recovery path's parent without resolving any parent through a
    /// symlink. Recovery accepts Git's full set of literal filenames, so it
    /// uses the caller's component validation rather than artifact policy.
    pub(crate) fn recovery_at(root: &Path, path: &str, create: bool) -> Result<(Self, String)> {
        let relative = Path::new(path);
        let mut parent = Self::open(root)?;
        for component in relative.parent().unwrap().components() {
            let name = component.as_os_str().to_string_lossy();
            let c = cstring(&name)?;
            // SAFETY: live directory fd and NUL-terminated component; O_NOFOLLOW
            // pins each traversed directory and rejects symlink substitution.
            let mut fd = unsafe {
                libc::openat(
                    parent.directory.as_raw_fd(),
                    c.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if fd < 0
                && create
                && std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound
            {
                // SAFETY: same live parent/component; mkdirat cannot follow a link.
                let made =
                    unsafe { libc::mkdirat(parent.directory.as_raw_fd(), c.as_ptr(), 0o755) };
                if made < 0
                    && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists
                {
                    return Err(std::io::Error::last_os_error().into());
                }
                // SAFETY: O_NOFOLLOW rejects a symlink raced into the new component.
                fd = unsafe {
                    libc::openat(
                        parent.directory.as_raw_fd(),
                        c.as_ptr(),
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                    )
                };
            }
            if fd < 0 {
                let error = std::io::Error::last_os_error();
                if matches!(
                    error.raw_os_error(),
                    Some(libc::ELOOP) | Some(libc::ENOTDIR)
                ) {
                    return Err(Error::Invalid(
                        "manual recovery snapshot path crosses a non-directory or symlink".into(),
                    ));
                }
                return Err(error.into());
            }
            // SAFETY: openat returned a new, valid, owned fd.
            parent = Self {
                directory: unsafe { File::from_raw_fd(fd) },
            };
        }
        let name = relative
            .file_name()
            .ok_or_else(|| Error::Invalid("recovery path has no filename".into()))?
            .to_str()
            .ok_or_else(|| Error::Invalid("recovery path is not UTF-8".into()))?
            .to_owned();
        Ok((parent, name))
    }
    pub(super) fn read(&self, name: &str) -> Result<File> {
        let name = cstring(name)?;
        // SAFETY: live directory fd and NUL-terminated filename. No links are followed.
        let fd = unsafe {
            libc::openat(
                self.directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        // SAFETY: newly opened descriptor is uniquely owned.
        let file = unsafe { File::from_raw_fd(fd) };
        crate::require(
            file.metadata()?.is_file(),
            "artifact must remain a regular file",
        )?;
        Ok(file)
    }
    pub(super) fn matches(&self, name: &str, expected: &[u8]) -> Result<bool> {
        let file = match self.read(name) {
            Ok(f) => f,
            Err(_) => return Ok(false),
        };
        if file.metadata()?.len() != expected.len() as u64 {
            return Ok(false);
        }
        let mut bytes = Vec::new();
        file.take(expected.len() as u64 + 1)
            .read_to_end(&mut bytes)?;
        Ok(bytes == expected)
    }
    pub(crate) fn rename(&self, from: &str, to: &Self, name: &str) -> Result<()> {
        let (from, name) = (cstring(from)?, cstring(name)?);
        // SAFETY: both live directory fds and valid component strings.
        let result = unsafe {
            libc::renameat(
                self.directory.as_raw_fd(),
                from.as_ptr(),
                to.directory.as_raw_fd(),
                name.as_ptr(),
            )
        };
        syscall(result)
    }
    pub(super) fn restore(&self, from: &str, to: &Self, name: &str) -> Result<()> {
        let (from, name) = (cstring(from)?, cstring(name)?);
        #[cfg(target_os = "macos")]
        // SAFETY: valid directory fds and names; RENAME_EXCL refuses any concurrent destination.
        let result = unsafe {
            libc::renameatx_np(
                self.directory.as_raw_fd(),
                from.as_ptr(),
                to.directory.as_raw_fd(),
                name.as_ptr(),
                libc::RENAME_EXCL,
            )
        };
        #[cfg(target_os = "linux")]
        // SAFETY: valid directory fds and names; RENAME_NOREPLACE refuses any concurrent destination.
        let result = unsafe {
            libc::renameat2(
                self.directory.as_raw_fd(),
                from.as_ptr(),
                to.directory.as_raw_fd(),
                name.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        // SAFETY: valid fds/names; linkat does not replace an existing destination.
        let result = unsafe {
            libc::linkat(
                self.directory.as_raw_fd(),
                from.as_ptr(),
                to.directory.as_raw_fd(),
                name.as_ptr(),
                0,
            )
        };
        syscall(result)
    }
    pub(crate) fn unlink(&self, name: &str) -> Result<()> {
        let name = cstring(name)?;
        // SAFETY: live private directory fd, valid filename; unlink removes only this link.
        syscall(unsafe { libc::unlinkat(self.directory.as_raw_fd(), name.as_ptr(), 0) })
    }
    pub(super) fn link(&self, from: &str, to: &Self, name: &str) -> Result<()> {
        let (from, name) = (cstring(from)?, cstring(name)?);
        // SAFETY: both live directory fds and valid components; flags=0 never replaces a target.
        let result = unsafe {
            libc::linkat(
                self.directory.as_raw_fd(),
                from.as_ptr(),
                to.directory.as_raw_fd(),
                name.as_ptr(),
                0,
            )
        };
        syscall(result)
    }
}
fn cstring(name: &str) -> Result<std::ffi::CString> {
    std::ffi::CString::new(name).map_err(|_| Error::Invalid("artifact path contains NUL".into()))
}
fn syscall(result: i32) -> Result<()> {
    if result < 0 {
        Err(std::io::Error::last_os_error().into())
    } else {
        Ok(())
    }
}

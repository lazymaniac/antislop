//! File delivery preserves displaced entries and never overwrites a newly
//! appearing destination. Project effects require runtime authorization.
use crate::{Result, hash, require};
use serde::{Deserialize, Serialize};
use std::{
    ffi::CString,
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Component, Path},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub device: u64,
    pub inode: u64,
}
impl Identity {
    pub fn read(path: &Path) -> Result<Self> {
        let m = path.metadata()?;
        Ok(Self {
            device: m.dev(),
            inode: m.ino(),
        })
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    File,
    Link,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stamp {
    pub kind: Kind,
    pub sha256: String,
    pub bytes: u64,
    pub permissions: u32,
    pub owner_uid: u32,
    pub owner_gid: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub stamp: Stamp,
    pub data: Vec<u8>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Change {
    pub path: String,
    pub before: Option<Stamp>,
    pub after: Option<Entry>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectState {
    Intent,
    BackedUp,
    Applied,
    Conflict,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Effect {
    pub path: String,
    pub backup: String,
    pub state: EffectState,
}

fn name(name: &str) -> Result<CString> {
    CString::new(name).map_err(|_| crate::Error::Invalid("NUL in filesystem name".into()))
}
fn result(value: libc::c_int) -> Result<libc::c_int> {
    if value < 0 {
        Err(std::io::Error::last_os_error().into())
    } else {
        Ok(value)
    }
}
struct Directory(File);
impl Directory {
    fn root(path: &Path) -> Result<Self> {
        let c = CString::new(path.as_os_str().as_bytes())
            .map_err(|_| crate::Error::Invalid("invalid source root".into()))?;
        // SAFETY: a valid C string is passed; the returned descriptor is owned.
        let fd = result(unsafe {
            libc::open(
                c.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        })?;
        Ok(Self(unsafe { File::from_raw_fd(fd) }))
    }
    fn child(&self, part: &str, create: bool) -> Result<Self> {
        let c = name(part)?;
        if create {
            // SAFETY: owned directory descriptor, terminated name, fixed mode.
            let value = unsafe { libc::mkdirat(self.0.as_raw_fd(), c.as_ptr(), 0o700) };
            if value < 0
                && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists
            {
                result(value)?;
            }
        }
        // SAFETY: O_NOFOLLOW refuses a symlink directory at this component.
        let fd = result(unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                c.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        })?;
        Ok(Self(unsafe { File::from_raw_fd(fd) }))
    }
    fn parent(&self, path: &str, create: bool) -> Result<(Self, String)> {
        valid_path(path)?;
        let mut parts: Vec<_> = Path::new(path)
            .components()
            .map(|p| p.as_os_str().to_str().unwrap())
            .collect();
        let leaf = parts.pop().unwrap().to_owned();
        let mut parent = Self(self.0.try_clone()?);
        for part in parts {
            parent = parent.child(part, create)?;
        }
        Ok((parent, leaf))
    }
    fn read(&self, leaf: &str) -> Result<Option<Entry>> {
        let c = name(leaf)?;
        let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
        // SAFETY: stat points to allocated storage; the name and descriptor live.
        let value = unsafe {
            libc::fstatat(
                self.0.as_raw_fd(),
                c.as_ptr(),
                metadata.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if value < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
            return Ok(None);
        }
        result(value)?;
        let metadata = unsafe { metadata.assume_init() };
        let mode = metadata.st_mode as u32;
        let (kind, data) = if mode & libc::S_IFMT as u32 == libc::S_IFLNK as u32 {
            let mut data = vec![0; 128];
            loop {
                // SAFETY: buffer has the supplied length and the path is terminated.
                let size = unsafe {
                    libc::readlinkat(
                        self.0.as_raw_fd(),
                        c.as_ptr(),
                        data.as_mut_ptr().cast(),
                        data.len(),
                    )
                };
                if size < 0 {
                    return Err(std::io::Error::last_os_error().into());
                }
                if (size as usize) < data.len() {
                    data.truncate(size as usize);
                    break;
                }
                data.resize(
                    data.len()
                        .checked_mul(2)
                        .ok_or_else(|| crate::Error::Invalid("link size overflow".into()))?,
                    0,
                );
            }
            (Kind::Link, data)
        } else {
            require(
                mode & libc::S_IFMT as u32 == libc::S_IFREG as u32,
                "delivery refuses special files and directories",
            )?;
            let fd = result(unsafe {
                libc::openat(
                    self.0.as_raw_fd(),
                    c.as_ptr(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
                )
            })?;
            let mut file = unsafe { File::from_raw_fd(fd) };
            let opened = file.metadata()?;
            require(
                opened.is_file()
                    && opened.dev() == metadata.st_dev as u64
                    && opened.ino() == metadata.st_ino,
                "entry changed during delivery observation",
            )?;
            let mut data = vec![];
            file.read_to_end(&mut data)?;
            (Kind::File, data)
        };
        Ok(Some(Entry {
            stamp: Stamp {
                kind,
                sha256: hash(&data),
                bytes: data.len() as u64,
                permissions: mode & 0o7777,
                owner_uid: metadata.st_uid,
                owner_gid: metadata.st_gid,
            },
            data,
        }))
    }
    fn link(&self, leaf: &str, destination: &Self, target: &str) -> Result<()> {
        let from = name(leaf)?;
        let to = name(target)?;
        // SAFETY: directory descriptors anchor both names. Flags zero links the
        // symlink itself, and EEXIST refuses any newly appearing destination.
        result(unsafe {
            libc::linkat(
                self.0.as_raw_fd(),
                from.as_ptr(),
                destination.0.as_raw_fd(),
                to.as_ptr(),
                0,
            )
        })?;
        destination.0.sync_all()?;
        Ok(())
    }
}
fn valid_path(path: &str) -> Result<()> {
    require(
        !path.is_empty()
            && !path.contains('\0')
            && Path::new(path).components().all(
                |c| matches!(c,Component::Normal(p) if !p.as_bytes().eq_ignore_ascii_case(b".git")),
            ),
        "delivery path escapes project or Git administration",
    )
}
pub fn read(root: &Path, path: &str) -> Result<Option<Entry>> {
    let root = Directory::root(root)?;
    match root.parent(path, false) {
        Ok((parent, leaf)) => parent.read(&leaf),
        Err(crate::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}
pub fn stamp(root: &Path, path: &str) -> Result<Option<Stamp>> {
    Ok(read(root, path)?.map(|e| e.stamp))
}

pub fn restore_missing(
    root: &Path,
    identity: &Identity,
    path: &str,
    backup: &str,
    expected: &Stamp,
) -> Result<()> {
    require(
        Identity::read(root)? == *identity,
        "source root identity changed before backup restoration",
    )?;
    let directory = Directory::root(root)?;
    let opened = directory.0.metadata()?;
    require(
        opened.dev() == identity.device && opened.ino() == identity.inode,
        "source directory changed while opening restoration scope",
    )?;
    let (parent, leaf) = directory.parent(path, true)?;
    require(
        parent.read(&leaf)?.is_none(),
        "backup restoration never overwrites an existing user entry",
    )?;
    let (backups, name) = directory.parent(backup, false)?;
    require(
        backups.read(&name)?.map(|e| e.stamp) == Some(expected.clone()),
        "backup version changed before restoration",
    )?;
    backups.link(&name, &parent, &leaf)
}

/// Persist each supplied effect checkpoint before allowing the next operation.
/// Backups are retained, including after success; they are never auto-deleted.
pub fn apply(
    root: &Path,
    identity: &Identity,
    operation: &str,
    changes: &[Change],
    checkpoint: &mut dyn FnMut(&Effect) -> Result<()>,
) -> Result<Vec<Effect>> {
    require(
        uuid::Uuid::parse_str(operation).is_ok(),
        "invalid delivery operation identity",
    )?;
    require(
        Identity::read(root)? == *identity,
        "delivery source root identity changed",
    )?;
    let directory = Directory::root(root)?;
    let opened = directory.0.metadata()?;
    require(
        opened.dev() == identity.device && opened.ino() == identity.inode,
        "source directory changed while opening delivery scope",
    )?;
    let mut seen = std::collections::BTreeSet::new();
    for change in changes {
        valid_path(&change.path)?;
        require(
            seen.insert(&change.path) && (change.before.is_some() || change.after.is_some()),
            "duplicate or empty delivery change",
        )?;
        require(
            stamp(root, &change.path)? == change.before,
            "source target changed since delivery preview",
        )?;
        if let Some(entry) = &change.after {
            require(
                entry.stamp.owner_uid == unsafe { libc::geteuid() },
                "delivery requires current-user file ownership",
            )?;
            require(
                entry.stamp.sha256 == hash(&entry.data)
                    && entry.stamp.bytes == entry.data.len() as u64,
                "delivery payload hash mismatch",
            )?;
            require(
                entry.stamp.permissions & 0o7000 == 0,
                "privileged permission bits require separate admission",
            )?;
            if entry.stamp.kind == Kind::Link {
                let target = Path::new(
                    std::str::from_utf8(&entry.data)
                        .map_err(|_| crate::Error::Invalid("link target is not UTF-8".into()))?,
                );
                let mut depth = Path::new(&change.path)
                    .parent()
                    .map_or(0, |p| p.components().count());
                for c in target.components() {
                    match c {
                        Component::Normal(_) => depth += 1,
                        Component::CurDir => (),
                        Component::ParentDir if depth > 0 => depth -= 1,
                        _ => {
                            return Err(crate::Error::Invalid(
                                "delivery link escapes project".into(),
                            ));
                        }
                    }
                }
            }
        }
    }
    let backup_name = format!(".antislop-delivery-{operation}");
    let mut effects = vec![];
    for (index, change) in changes.iter().enumerate() {
        let effect = Effect {
            path: change.path.clone(),
            backup: format!("{backup_name}/{index}"),
            state: EffectState::Intent,
        };
        checkpoint(&effect)?; // No project mutation before durable intent.
        require(
            Identity::read(root)? == *identity,
            "delivery root moved during transfer",
        )?;
        let (parent, leaf) = directory.parent(&change.path, true)?;
        require(
            parent.read(&leaf)?.map(|e| e.stamp) == change.before,
            "source target changed before capture",
        )?;
        let backups = directory.child(&backup_name, true)?;
        let private = backups.0.metadata()?;
        require(
            private.uid() == unsafe { libc::geteuid() } && private.mode() & 0o077 == 0,
            "delivery backup directory is not private and owned",
        )?;
        let old = index.to_string();
        require(
            backups.read(&old)?.is_none(),
            "delivery backup exists; inspect prior effects without replay",
        )?;
        let mut effect = effect;
        if change.before.is_some() {
            let from = name(&leaf)?;
            let to = name(&old)?;
            result(unsafe {
                libc::renameat(
                    parent.0.as_raw_fd(),
                    from.as_ptr(),
                    backups.0.as_raw_fd(),
                    to.as_ptr(),
                )
            })?;
            parent.0.sync_all()?;
            backups.0.sync_all()?;
            effect.state = EffectState::BackedUp;
            checkpoint(&effect)?;
            if backups.read(&old)?.map(|e| e.stamp) != change.before {
                let _ = backups.link(&old, &parent, &leaf); // Restore only if still absent.
                effect.state = EffectState::Conflict;
                checkpoint(&effect)?;
                return Err(crate::Error::Invalid(
                    "captured source changed; its displaced version is retained in the backup"
                        .into(),
                ));
            }
        }
        if let Some(entry) = &change.after {
            let staged = format!("new-{index}");
            let c = name(&staged)?;
            if entry.stamp.kind == Kind::Link {
                let target = CString::new(entry.data.clone())
                    .map_err(|_| crate::Error::Invalid("NUL in link target".into()))?;
                result(unsafe {
                    libc::symlinkat(target.as_ptr(), backups.0.as_raw_fd(), c.as_ptr())
                })?;
                result(unsafe {
                    libc::fchownat(
                        backups.0.as_raw_fd(),
                        c.as_ptr(),
                        entry.stamp.owner_uid,
                        entry.stamp.owner_gid,
                        libc::AT_SYMLINK_NOFOLLOW,
                    )
                })?;
            } else {
                let fd = result(unsafe {
                    libc::openat(
                        backups.0.as_raw_fd(),
                        c.as_ptr(),
                        libc::O_WRONLY
                            | libc::O_CREAT
                            | libc::O_EXCL
                            | libc::O_NOFOLLOW
                            | libc::O_CLOEXEC,
                        0o600,
                    )
                })?;
                let mut file = unsafe { File::from_raw_fd(fd) };
                file.write_all(&entry.data)?;
                result(unsafe {
                    libc::fchown(
                        file.as_raw_fd(),
                        entry.stamp.owner_uid,
                        entry.stamp.owner_gid,
                    )
                })?;
                result(unsafe {
                    libc::fchmod(file.as_raw_fd(), entry.stamp.permissions as libc::mode_t)
                })?;
                file.sync_all()?;
            }
            backups.0.sync_all()?;
            if let Err(error) = backups.link(&staged, &parent, &leaf) {
                if change.before.is_some() {
                    let _ = backups.link(&old, &parent, &leaf);
                }
                effect.state = EffectState::Conflict;
                checkpoint(&effect)?;
                return Err(error);
            }
        }
        require(
            parent.read(&leaf)?.map(|e| e.stamp) == change.after.as_ref().map(|e| e.stamp.clone()),
            "delivered target changed; inspect retained versions",
        )?;
        effect.state = EffectState::Applied;
        checkpoint(&effect)?;
        effects.push(effect);
    }
    Ok(effects)
}

//! Read-only source observations for managed workspace admission. No checkout,
//! index refresh, stash, reset, clean, commit or network operation is performed.
use crate::{Result, hash, require};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    os::unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Component, Path},
    process::Stdio,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitEntry {
    pub mode: String,
    pub object_id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Repository {
    pub root: String,
    pub project_prefix: String,
    pub common_directory: String,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub head_entries: BTreeMap<String, GitEntry>,
    pub index_entries: BTreeMap<String, BTreeMap<u8, GitEntry>>,
    pub untracked: BTreeSet<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Regular,
    Symlink,
    Directory,
    Gitlink,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileObservation {
    pub kind: FileKind,
    pub permissions: u32,
    pub owner_uid: u32,
    pub owner_gid: u32,
    pub content_hash: Option<String>,
    pub git_entry: Option<GitEntry>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub root: String,
    pub source_device: u64,
    pub source_inode: u64,
    pub repository: Option<Repository>,
    /// Includes missing tracked files as None; symlinks hash their target bytes.
    pub files: BTreeMap<String, Option<FileObservation>>,
    /// Observable staged, unstaged and nonignored untracked changes; gitlink
    /// working content is explicitly unresolved, never treated as clean.
    pub local_changes: BTreeSet<String>,
    /// Explicitly requested paths may also include ignored files.
    pub additional_paths: BTreeSet<String>,
    #[serde(default)]
    pub excluded_paths: BTreeSet<String>,
    /// Gitlinks are observed but need separate recursive admission before copying.
    pub gitlinks: BTreeSet<String>,
    /// Directory entries (such as untracked nested repositories) whose contents
    /// Git did not enumerate. They cannot authorize copying an unknown subtree.
    pub unobserved_directories: BTreeSet<String>,
    pub content_hash: String,
}
fn check_cancelled(cancelled: &AtomicBool) -> Result<()> {
    require(
        !cancelled.load(Ordering::SeqCst),
        "workspace observation cancelled",
    )
}
fn digest_stream(
    mut reader: impl Read,
    mut target: impl Write,
    cancelled: Option<&AtomicBool>,
) -> Result<String> {
    let mut digest = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        if let Some(cancelled) = cancelled {
            check_cancelled(cancelled)?;
        }
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
        target.write_all(&buffer[..count])?;
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}
fn git(
    root: &Path,
    args: &[&str],
    input: Option<Box<dyn Read + Send>>,
    cancelled: &AtomicBool,
) -> Result<(Option<i32>, Vec<u8>, Option<String>)> {
    check_cancelled(cancelled)?;
    let mut command = crate::process::command("git")?;
    command.env_clear();
    for name in ["PATH", "HOME", "TMPDIR"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command
        .env("LANG", "C")
        .env("LC_ALL", "C")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "submodule.recurse=false",
            "-C",
        ])
        .arg(root)
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = crate::process::SupervisedChild::spawn(&mut command)?;
    let mut stdout = child.child.stdout.take().unwrap();
    let mut stderr = child.child.stderr.take().unwrap();
    let output = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let error = std::thread::spawn(move || std::io::copy(&mut stderr, &mut std::io::sink()));
    let writer = input.map(|reader| {
        let stdin = child.child.stdin.take().unwrap();
        std::thread::spawn(move || digest_stream(reader, stdin, None))
    });
    let status = loop {
        if cancelled.load(Ordering::SeqCst) {
            child.cancel()?;
            break None;
        }
        if let Some(status) = child.child.try_wait()? {
            break Some(status);
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let bytes = output
        .join()
        .map_err(|_| crate::Error::Invalid("Git output reader failed".into()))??;
    error
        .join()
        .map_err(|_| crate::Error::Invalid("Git error reader failed".into()))??;
    let input_hash = writer
        .map(|writer| {
            writer
                .join()
                .map_err(|_| crate::Error::Invalid("Git input writer failed".into()))
        })
        .transpose()?;
    check_cancelled(cancelled)?;
    let input_hash = input_hash.transpose()?;
    Ok((status.and_then(|s| s.code()), bytes, input_hash))
}
pub(crate) fn required_git(
    root: &Path,
    args: &[&str],
    input: Option<Box<dyn Read + Send>>,
    cancelled: &AtomicBool,
) -> Result<Vec<u8>> {
    let (status, bytes, _) = git(root, args, input, cancelled)?;
    require(
        status == Some(0),
        &format!("read-only Git observation failed: {}", args[0]),
    )?;
    Ok(bytes)
}
fn text(bytes: &[u8]) -> Result<String> {
    String::from_utf8(bytes.to_vec()).map_err(|_| {
        crate::Error::Invalid(
            "workspace metadata path is not UTF-8; cannot silently omit it".into(),
        )
    })
}
fn line(bytes: Vec<u8>) -> Result<String> {
    require(
        bytes.ends_with(b"\n"),
        "Git observation lacks line terminator",
    )?;
    text(&bytes[..bytes.len() - 1])
}
fn path_valid(path: &str) -> Result<()> {
    require(
        !path.is_empty()
            && Path::new(path)
                .components()
                .all(|c| matches!(c,Component::Normal(n) if n != ".git")),
        "workspace path must be normal, relative and outside Git metadata",
    )
}
fn object_id(id: &str) -> Result<()> {
    require(
        matches!(id.len(), 40 | 64) && id.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid Git object identity",
    )
}
fn entry(header: &str) -> Result<GitEntry> {
    let fields: Vec<_> = header.split(' ').collect();
    require(fields.len() == 3, "malformed Git entry")?;
    require(
        matches!(fields[0], "100644" | "100755" | "120000" | "160000"),
        "unsupported Git entry mode",
    )?;
    object_id(fields[1])?;
    Ok(GitEntry {
        mode: fields[0].into(),
        object_id: fields[1].into(),
    })
}
pub(crate) fn repository(root: &Path, cancelled: &AtomicBool) -> Result<Option<Repository>> {
    if !root
        .ancestors()
        .any(|p| p.join(".git").symlink_metadata().is_ok())
    {
        return Ok(None);
    }
    let top = line(required_git(
        root,
        &["rev-parse", "--show-toplevel"],
        None,
        cancelled,
    )?)?;
    let top_path = Path::new(&top).canonicalize()?;
    let prefix = root
        .strip_prefix(&top_path)
        .map_err(|_| crate::Error::Invalid("Git root does not contain workspace".into()))?;
    let project_prefix = if prefix.as_os_str().is_empty() {
        String::new()
    } else {
        format!(
            "{}/",
            prefix
                .to_str()
                .ok_or_else(|| crate::Error::Invalid("non-UTF-8 project prefix".into()))?
        )
    };
    let common = line(required_git(
        root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        None,
        cancelled,
    )?)?;
    let (branch_status, branch_output, _) =
        git(root, &["symbolic-ref", "--quiet", "HEAD"], None, cancelled)?;
    let branch = match branch_status {
        Some(0) => Some(line(branch_output)?),
        Some(1) => None,
        _ => {
            return Err(crate::Error::Invalid(
                "cannot observe current Git branch".into(),
            ));
        }
    };
    let refs = text(&required_git(
        root,
        &["for-each-ref", "--format=%(refname)"],
        None,
        cancelled,
    )?)?;
    let unborn = branch
        .as_ref()
        .is_some_and(|b| !refs.lines().any(|r| r == b));
    let head = if unborn {
        None
    } else {
        Some(line(required_git(
            root,
            &["rev-parse", "--verify", "HEAD"],
            None,
            cancelled,
        )?)?)
    };
    if let Some(head) = &head {
        object_id(head)?;
    }
    let mut heads = BTreeMap::new();
    if let Some(head) = &head {
        let raw = required_git(
            root,
            &["ls-tree", "-r", "--full-tree", "-z", head],
            None,
            cancelled,
        )?;
        for row in raw.split(|b| *b == 0).filter(|r| !r.is_empty()) {
            let (header, path) = row.split_at(
                row.iter()
                    .position(|b| *b == b'\t')
                    .ok_or_else(|| crate::Error::Invalid("malformed Git tree".into()))?,
            );
            let path = text(&path[1..])?;
            let Some(path) = path.strip_prefix(&project_prefix) else {
                continue;
            };
            let path = path.to_string();
            path_valid(&path)?;
            let header = text(header)?;
            let fields: Vec<_> = header.split(' ').collect();
            require(
                fields.len() == 3 && matches!(fields[1], "blob" | "commit"),
                "malformed Git tree type",
            )?;
            let e = entry(&format!("{} {} 0", fields[0], fields[2]))?;
            require(heads.insert(path, e).is_none(), "duplicate Git tree path")?;
        }
    }
    let raw = required_git(
        root,
        &["ls-files", "--stage", "--full-name", "-z"],
        None,
        cancelled,
    )?;
    let mut index: BTreeMap<String, BTreeMap<u8, GitEntry>> = BTreeMap::new();
    for row in raw.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let (header, path) = row.split_at(
            row.iter()
                .position(|b| *b == b'\t')
                .ok_or_else(|| crate::Error::Invalid("malformed Git index".into()))?,
        );
        let path = text(&path[1..])?;
        let Some(path) = path.strip_prefix(&project_prefix) else {
            continue;
        };
        let path = path.to_string();
        path_valid(&path)?;
        let header = text(header)?;
        let e = entry(&header)?;
        let stage = header
            .rsplit(' ')
            .next()
            .unwrap()
            .parse::<u8>()
            .map_err(|_| crate::Error::Invalid("invalid index stage".into()))?;
        require(
            stage <= 3 && index.entry(path).or_default().insert(stage, e).is_none(),
            "invalid or duplicate index stage",
        )?;
    }
    let raw = required_git(
        root,
        &[
            "ls-files",
            "--others",
            "--exclude-standard",
            "--full-name",
            "-z",
        ],
        None,
        cancelled,
    )?;
    let mut untracked = BTreeSet::new();
    for path in raw.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let path = text(path)?;
        let Some(path) = path.strip_prefix(&project_prefix) else {
            continue;
        };
        let path = path.to_string();
        path_valid(&path)?;
        untracked.insert(path);
    }
    Ok(Some(Repository {
        root: top,
        project_prefix,
        common_directory: common,
        head,
        branch,
        head_entries: heads,
        index_entries: index,
        untracked,
    }))
}
fn nongit_paths(
    root: &Path,
    relative: &Path,
    paths: &mut BTreeSet<String>,
    excluded: &BTreeSet<String>,
    cancelled: &AtomicBool,
) -> Result<()> {
    for entry in std::fs::read_dir(root.join(relative))? {
        check_cancelled(cancelled)?;
        let entry = entry?;
        let path = relative.join(entry.file_name());
        let name = path
            .to_str()
            .ok_or_else(|| crate::Error::Invalid("non-UTF-8 workspace path".into()))?;
        if excluded
            .iter()
            .any(|prefix| name == prefix || name.starts_with(&format!("{prefix}/")))
        {
            continue;
        }
        path_valid(name)?;
        if entry.file_type()?.is_dir() {
            nongit_paths(root, &path, paths, excluded, cancelled)?;
        } else {
            paths.insert(name.into());
        }
    }
    Ok(())
}
fn file(
    root: &Path,
    path: &str,
    repo: bool,
    cancelled: &AtomicBool,
) -> Result<Option<FileObservation>> {
    path_valid(path)?;
    let mut absolute = root.to_path_buf();
    let parts: Vec<_> = Path::new(path).components().collect();
    for (n, part) in parts.iter().enumerate() {
        absolute.push(part);
        match absolute.symlink_metadata() {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
            Ok(m) if n + 1 < parts.len() => require(
                m.is_dir() && !m.file_type().is_symlink(),
                "workspace path traverses a symlink or non-directory",
            )?,
            _ => (),
        }
    }
    let m = absolute.symlink_metadata()?;
    let (kind, reader, mode): (FileKind, Option<Box<dyn Read + Send>>, &str) =
        if m.file_type().is_symlink() {
            (
                FileKind::Symlink,
                Some(Box::new(std::io::Cursor::new(
                    std::fs::read_link(&absolute)?
                        .as_os_str()
                        .as_bytes()
                        .to_vec(),
                ))),
                "120000",
            )
        } else if m.is_file() {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&absolute)?;
            let opened = file.metadata()?;
            require(
                opened.is_file() && (opened.dev(), opened.ino()) == (m.dev(), m.ino()),
                "workspace file changed before observation",
            )?;
            (
                FileKind::Regular,
                Some(Box::new(file)),
                if m.permissions().mode() & 0o100 != 0 {
                    "100755"
                } else {
                    "100644"
                },
            )
        } else if m.is_dir() {
            (FileKind::Directory, None, "040000")
        } else {
            return Err(crate::Error::Invalid(
                "special workspace file needs separate admission".into(),
            ));
        };
    check_cancelled(cancelled)?;
    let (content_hash, git_entry) = if let Some(reader) = reader {
        if repo {
            let (status, output, input_hash) = git(
                root,
                &["hash-object", "--no-filters", "--stdin"],
                Some(reader),
                cancelled,
            )?;
            require(status == Some(0), "read-only Git blob observation failed")?;
            let oid = line(output)?;
            object_id(&oid)?;
            (
                input_hash,
                Some(GitEntry {
                    mode: mode.into(),
                    object_id: oid,
                }),
            )
        } else {
            (
                Some(digest_stream(reader, std::io::sink(), Some(cancelled))?),
                None,
            )
        }
    } else {
        (None, None)
    };
    Ok(Some(FileObservation {
        kind,
        permissions: m.permissions().mode() & 0o7777,
        owner_uid: m.uid(),
        owner_gid: m.gid(),
        content_hash,
        git_entry,
    }))
}
fn once(
    root: &Path,
    additional: &BTreeSet<String>,
    excluded: &BTreeSet<String>,
    cancelled: &AtomicBool,
) -> Result<Snapshot> {
    let repo = repository(root, cancelled)?;
    let mut paths = additional.clone();
    if let Some(repo) = &repo {
        paths.extend(repo.head_entries.keys().cloned());
        paths.extend(repo.index_entries.keys().cloned());
        paths.extend(repo.untracked.iter().cloned());
    } else {
        nongit_paths(root, Path::new(""), &mut paths, excluded, cancelled)?;
    }
    paths.retain(|path| {
        !excluded
            .iter()
            .any(|prefix| path == prefix || path.starts_with(&format!("{prefix}/")))
    });
    let mut files: BTreeMap<_, _> = paths
        .iter()
        .map(|p| Ok((p.clone(), file(root, p, repo.is_some(), cancelled)?)))
        .collect::<Result<_>>()?;
    let mut changes = BTreeSet::new();
    let mut gitlinks = BTreeSet::new();
    for (path, observed) in &mut files {
        let gitlink = repo.as_ref().is_some_and(|r| {
            r.head_entries.get(path).is_some_and(|e| e.mode == "160000")
                || r.index_entries
                    .get(path)
                    .is_some_and(|s| s.values().any(|e| e.mode == "160000"))
        });
        if gitlink
            && let Some(file) = observed
            && file.kind == FileKind::Directory
        {
            file.kind = FileKind::Gitlink;
        }
        if gitlink {
            gitlinks.insert(path.clone());
        }
        if let Some(repo) = &repo {
            let stages = repo.index_entries.get(path);
            let index = stages.and_then(|s| s.get(&0));
            let current = observed.as_ref().and_then(|f| f.git_entry.as_ref());
            if stages.is_some_and(|s| s.keys().any(|stage| *stage != 0))
                || index != repo.head_entries.get(path)
                || (!observed
                    .as_ref()
                    .is_some_and(|f| f.kind == FileKind::Gitlink)
                    && current != index)
                || repo.untracked.contains(path)
            {
                changes.insert(path.clone());
            }
        } else {
            changes.insert(path.clone());
        }
    }
    let metadata = root.metadata()?;
    let unobserved_directories = files
        .iter()
        .filter_map(|(path, file)| {
            file.as_ref()
                .is_some_and(|f| f.kind == FileKind::Directory)
                .then_some(path.clone())
        })
        .collect();
    let mut snapshot = Snapshot {
        root: root
            .to_str()
            .ok_or_else(|| crate::Error::Invalid("non-UTF-8 workspace root".into()))?
            .into(),
        source_device: metadata.dev(),
        source_inode: metadata.ino(),
        repository: repo,
        files,
        local_changes: changes,
        additional_paths: additional.clone(),
        excluded_paths: excluded.clone(),
        gitlinks,
        unobserved_directories,
        content_hash: String::new(),
    };
    snapshot.content_hash = hash(serde_json::to_vec(&snapshot)?);
    Ok(snapshot)
}
/// Two equal observations are required; filesystem leases cannot stop arbitrary
/// external writers. Revalidation immediately before copying is still required.
pub fn observe(
    root: &Path,
    additional: &BTreeSet<String>,
    cancelled: &AtomicBool,
) -> Result<Snapshot> {
    observe_excluding(root, additional, &BTreeSet::new(), cancelled)
}
pub(crate) fn observe_excluding(
    root: &Path,
    additional: &BTreeSet<String>,
    excluded: &BTreeSet<String>,
    cancelled: &AtomicBool,
) -> Result<Snapshot> {
    check_cancelled(cancelled)?;
    let root = root.canonicalize()?;
    require(root.is_dir(), "workspace source is not a directory")?;
    for path in additional {
        path_valid(path)?;
    }
    for path in excluded {
        path_valid(path)?;
    }
    let first = once(&root, additional, excluded, cancelled)?;
    require(
        first == once(&root, additional, excluded, cancelled)?,
        "workspace changed during source observation; retry the read",
    )?;
    Ok(first)
}
impl Snapshot {
    pub fn validate_current(&self, cancelled: &AtomicBool) -> Result<()> {
        require(
            *self
                == observe_excluding(
                    Path::new(&self.root),
                    &self.additional_paths,
                    &self.excluded_paths,
                    cancelled,
                )?,
            "workspace source snapshot changed",
        )
    }
    /// Bind selection of these observed working copies/deletions. This is a
    /// scope digest, not an approval or permission to create a worktree.
    pub fn inclusion_scope(&self, paths: &BTreeSet<String>) -> Result<String> {
        let mut content = self.clone();
        content.content_hash.clear();
        require(
            self.content_hash == hash(serde_json::to_vec(&content)?),
            "workspace snapshot hash changed",
        )?;
        require(
            self.gitlinks.is_empty(),
            "submodule/gitlink requires recursive workspace admission",
        )?;
        require(
            self.unobserved_directories.is_empty(),
            "unobserved directory contents require explicit source admission",
        )?;
        require(
            paths.is_subset(&self.local_changes),
            "included local path is not an observed change",
        )?;
        require(
            self.repository.as_ref().is_none_or(|r| {
                r.index_entries
                    .values()
                    .all(|s| s.keys().all(|stage| *stage == 0))
            }),
            "unresolved Git index conflicts require integration before preparation",
        )?;
        Ok(hash(serde_json::to_vec(&(
            "workspace_source_inclusion_v1",
            self,
            paths,
        ))?))
    }
}

//! Directory inputs preserve the user's scope as one versioned source.
//! A directory observation includes text contents and explicit binary/link metadata.
use super::snapshot;
use crate::{Result, hash, require};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path},
    sync::atomic::AtomicBool,
};

pub fn relative(root: &Path, selector: &str) -> Result<String> {
    require(!selector.is_empty(), "input scope is empty")?;
    let root = root.canonicalize()?;
    let path = Path::new(selector);
    let path = if path.is_absolute() {
        let alias = if path.starts_with(&root) {
            Some(root.as_path())
        } else {
            // macOS /var and /private/var refer to the same workspace.
            // Keep all components below its outermost alias for link checks.
            path.ancestors()
                .filter(|ancestor| ancestor.canonicalize().is_ok_and(|p| p == root))
                .last()
        }
        .ok_or_else(|| crate::Error::Invalid("input scope is outside the workspace".into()))?;
        path.strip_prefix(alias)
            .map_err(|_| crate::Error::Invalid("input scope is outside the workspace".into()))?
    } else {
        path
    };
    require(
        path.components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir)),
        "input scope must stay within the workspace",
    )?;
    let mut normalized = std::path::PathBuf::new();
    for part in path.components() {
        if let Component::Normal(part) = part {
            normalized.push(part);
        }
    }
    Ok(if normalized.as_os_str().is_empty() {
        ".".into()
    } else {
        normalized
            .to_str()
            .ok_or_else(|| crate::Error::Invalid("input path is not UTF-8".into()))?
            .into()
    })
}

#[derive(Serialize, Deserialize)]
struct DirectoryInput {
    #[serde(default)]
    format: String,
    kind: String,
    selection: String,
    limitations: String,
    excluded_runtime_state: BTreeSet<String>,
    files: BTreeMap<String, Entry>,
}
#[derive(Serialize, Deserialize)]
struct Entry {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    tracked: bool,
    kind: String,
    sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_hex: Option<String>,
}

pub(super) fn directory(root: &Path) -> Result<String> {
    let excluded = runtime_exclusions(root);
    let first = capture(root, &excluded)?;
    require(
        first == capture(root, &excluded)?,
        "directory input changed during capture; retry the observation",
    )?;
    Ok(first)
}

fn runtime_exclusions(root: &Path) -> BTreeSet<String> {
    let mut excluded = BTreeSet::new();
    // These private directories belong to the local runtime, not the project.
    for name in [".antislop", ".architect"] {
        let path = root.join(name);
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let private = path.symlink_metadata().is_ok_and(|m| {
            m.is_dir()
                && !m.file_type().is_symlink()
                && m.uid() == unsafe { libc::geteuid() }
                && m.permissions().mode() & 0o077 == 0
        });
        if private && path.join("daemon.lock").is_file() && path.join("state.db").is_file() {
            excluded.insert(name.to_owned());
        }
    }
    excluded
}

fn capture(root: &Path, excluded: &BTreeSet<String>) -> Result<String> {
    let (repository, paths, tracked) =
        snapshot::source_members(root, excluded, &AtomicBool::new(false))?;
    let mut files = BTreeMap::new();
    for path in paths {
        let absolute = root.join(&path);
        let metadata = match absolute.symlink_metadata() {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                files.insert(
                    path.clone(),
                    Entry {
                        tracked: tracked.contains(&path),
                        kind: "missing".into(),
                        sha256: None,
                        text: None,
                        bytes: None,
                        target_hex: None,
                    },
                );
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        let entry = if metadata.file_type().is_symlink() {
            use std::os::unix::ffi::OsStrExt;
            let target = std::fs::read_link(&absolute)?;
            let data = target.as_os_str().as_bytes();
            Entry {
                tracked: tracked.contains(&path),
                kind: "symlink".into(),
                sha256: Some(hash(data)),
                text: None,
                bytes: Some(data.len() as u64),
                target_hex: Some(data.iter().map(|b| format!("{b:02x}")).collect()),
            }
        } else if metadata.is_dir() {
            let text = capture(&absolute, &BTreeSet::new())?;
            Entry {
                tracked: tracked.contains(&path),
                kind: "directory".into(),
                sha256: Some(hash(&text)),
                text: Some(text),
                bytes: None,
                target_hex: None,
            }
        } else if metadata.is_file() {
            let data = super::read_input_bytes(root, &path)?;
            let sha256 = Some(hash(&data));
            let bytes = Some(data.len() as u64);
            match String::from_utf8(data) {
                Ok(text) if !text.contains('\0') => Entry {
                    tracked: tracked.contains(&path),
                    kind: "text".into(),
                    sha256,
                    text: Some(text),
                    bytes,
                    target_hex: None,
                },
                _ => Entry {
                    tracked: tracked.contains(&path),
                    kind: "binary".into(),
                    sha256,
                    text: None,
                    bytes,
                    target_hex: None,
                },
            }
        } else {
            return Err(crate::Error::Invalid(
                "special files cannot be project inputs".into(),
            ));
        };
        files.insert(path, entry);
    }
    Ok(serde_json::to_string(&DirectoryInput {
        format:"antislop-directory-v1".into(),
        kind: "directory".into(),
        selection: (if repository {
            "Git tracked and nonignored untracked working files; deleted tracked files remain explicit"
        } else {
            "Current source files recursively; Git administration and runtime state excluded"
        }).into(),
        limitations: "Text contents are supplied. Binary files expose byte count and SHA-256, not decoded contents. Symlink targets are recorded without following them; do not infer unavailable content.".into(),
        excluded_runtime_state: excluded.clone(),
        files,
    })?)
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum DiffContent {
    Text(String),
    Metadata(serde_json::Value),
}

/// Recognize only a runtime directory envelope, not an arbitrary JSON text file.
pub(crate) fn directory_diff_entries(
    scope: &str,
    text: &str,
) -> Result<Option<BTreeMap<String, DiffContent>>> {
    let Ok(directory) = serde_json::from_str::<DirectoryInput>(text) else {
        return Ok(None);
    };
    if directory.kind != "directory" {
        return Ok(None);
    }
    let mut files = BTreeMap::new();
    flatten(Path::new(scope), directory, &mut files)?;
    Ok(Some(files))
}
fn flatten(
    prefix: &Path,
    directory: DirectoryInput,
    files: &mut BTreeMap<String, DiffContent>,
) -> Result<()> {
    for (name, entry) in directory.files {
        let path = Path::new(&name);
        require(
            path.components().all(|c| matches!(c, Component::Normal(_))),
            "invalid captured directory member",
        )?;
        let path = prefix.join(path);
        let path = if let Ok(relative) = path.strip_prefix(".") {
            relative.to_path_buf()
        } else {
            path
        };
        let name = path
            .to_str()
            .ok_or_else(|| crate::Error::Invalid("non-UTF-8 directory member".into()))?
            .to_owned();
        if entry.kind == "missing" {
            continue;
        }
        if entry.kind == "directory" {
            let nested: DirectoryInput =
                serde_json::from_str(entry.text.as_deref().ok_or_else(|| {
                    crate::Error::Invalid("nested directory body missing".into())
                })?)?;
            flatten(&path, nested, files)?;
        } else if entry.kind == "text" {
            files.insert(
                name,
                DiffContent::Text(
                    entry.text.ok_or_else(|| {
                        crate::Error::Invalid("directory text body missing".into())
                    })?,
                ),
            );
        } else {
            files.insert(name, DiffContent::Metadata(serde_json::to_value(entry)?));
        }
    }
    Ok(())
}
/// Size-bounded description of a captured scope: paths, kinds, sizes and hashes,
/// without contents. Reviewers bind to the full capture through its receipt hash.
pub fn manifest(text: &str) -> serde_json::Value {
    match serde_json::from_str::<DirectoryInput>(text) {
        Ok(directory) if directory.format == "antislop-directory-v1" => {
            let files: BTreeMap<_, _> = directory
                .files
                .iter()
                .map(|(path, entry)| {
                    let value = if entry.kind == "directory" {
                        manifest(entry.text.as_deref().unwrap_or_default())
                    } else {
                        serde_json::json!({
                            "kind": entry.kind,
                            "sha256": entry.sha256,
                            "bytes": entry.bytes.or(entry.text.as_ref().map(|t| t.len() as u64)),
                        })
                    };
                    (path.clone(), value)
                })
                .collect();
            serde_json::json!({"kind": "directory", "selection": directory.selection, "files": files})
        }
        _ => serde_json::json!({"kind": "text", "sha256": hash(text), "bytes": text.len()}),
    }
}
pub(crate) fn is_directory_envelope(text: &str) -> bool {
    serde_json::from_str::<DirectoryInput>(text)
        .is_ok_and(|d| d.kind == "directory" && d.format == "antislop-directory-v1")
}

/// A view of an already captured directory. The caller must establish that the
/// source is a directory; an ordinary JSON file must never invent file scope.
pub(crate) struct ContextMember {
    pub metadata: serde_json::Value,
    pub text: Option<String>,
}
pub(crate) fn context_members(text: &str) -> Result<BTreeMap<String, ContextMember>> {
    fn visit(
        prefix: &Path,
        directory: DirectoryInput,
        output: &mut BTreeMap<String, ContextMember>,
    ) -> Result<()> {
        require(
            directory.format == "antislop-directory-v1" && directory.kind == "directory",
            "invalid context directory capture",
        )?;
        for (name, entry) in directory.files {
            require(
                !name.is_empty()
                    && Path::new(&name)
                        .components()
                        .all(|c| matches!(c, Component::Normal(_))),
                "invalid context member path",
            )?;
            let path = prefix.join(&name);
            let name = path
                .to_str()
                .ok_or_else(|| crate::Error::Invalid("non-UTF-8 context member".into()))?
                .to_owned();
            let mut metadata = serde_json::to_value(&entry)?;
            metadata.as_object_mut().unwrap().remove("text");
            let body = if matches!(entry.kind.as_str(), "text" | "directory") {
                let text = entry
                    .text
                    .as_ref()
                    .ok_or_else(|| crate::Error::Invalid("missing captured context body".into()))?;
                require(
                    entry.sha256.as_ref() == Some(&hash(text))
                        && entry.bytes.is_none_or(|n| n == text.len() as u64),
                    "captured context member hash/length changed",
                )?;
                if entry.kind == "directory" {
                    None
                } else {
                    Some(text.clone())
                }
            } else {
                require(
                    matches!(entry.kind.as_str(), "binary" | "symlink" | "missing")
                        && entry.text.is_none(),
                    "unsupported captured context member",
                )?;
                None
            };
            require(
                output
                    .insert(
                        name,
                        ContextMember {
                            metadata,
                            text: body,
                        },
                    )
                    .is_none(),
                "overlapping captured context members",
            )?;
            if entry.kind == "directory" {
                visit(
                    &path,
                    crate::transport::decode(entry.text.unwrap().as_bytes())?,
                    output,
                )?;
            }
        }
        Ok(())
    }
    let mut output = BTreeMap::new();
    visit(
        Path::new(""),
        crate::transport::decode(text.as_bytes())?,
        &mut output,
    )?;
    Ok(output)
}

/// Source equivalence across workspace relocation excludes capture mechanism
/// metadata, while retaining every observed member's bytes/type/link identity.
pub(crate) fn equivalent_sources(
    baseline: &BTreeMap<String, String>,
    candidate: &BTreeMap<String, String>,
) -> Result<bool> {
    if baseline.keys().ne(candidate.keys()) {
        return Ok(false);
    }
    for (scope, before) in baseline {
        let after = &candidate[scope];
        if before == after {
            continue;
        }
        match (
            directory_diff_entries(scope, before)?,
            directory_diff_entries(scope, after)?,
        ) {
            (Some(a), Some(b)) if a == b => (),
            _ => return Ok(false),
        }
    }
    Ok(true)
}
pub(crate) fn current_directory_diff_entries(
    root: &Path,
    scope: &str,
) -> Result<BTreeMap<String, DiffContent>> {
    let root = root.canonicalize()?;
    let relative = relative(&root, scope)?;
    let path = root.join(&relative);
    let metadata = match path.symlink_metadata() {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return current_file_diff_entry(&root, &relative);
    }
    super::validate_inputs(&root, std::slice::from_ref(&relative))?;
    let text = capture(&path, &runtime_exclusions(&path))?;
    directory_diff_entries(&relative, &text)?
        .ok_or_else(|| crate::Error::Invalid("directory observation malformed".into()))
}

pub(crate) fn current_file_diff_entry(
    root: &Path,
    relative: &str,
) -> Result<BTreeMap<String, DiffContent>> {
    let mut path = root.to_path_buf();
    let components: Vec<_> = Path::new(relative).components().collect();
    require(
        components.iter().all(|c| matches!(c, Component::Normal(_))),
        "invalid diff file path",
    )?;
    for (index, part) in components.iter().enumerate() {
        path.push(part.as_os_str());
        let metadata = match path.symlink_metadata() {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(BTreeMap::new());
            }
            Err(error) => return Err(error.into()),
        };
        require(
            index + 1 == components.len() || !metadata.file_type().is_symlink(),
            "diff file ancestor became a symlink",
        )?;
    }
    let metadata = path.symlink_metadata()?;
    let content = if metadata.file_type().is_symlink() {
        use std::os::unix::ffi::OsStrExt;
        let target = std::fs::read_link(&path)?;
        let data = target.as_os_str().as_bytes();
        DiffContent::Metadata(serde_json::json!({
            "kind":"symlink", "sha256":hash(data), "bytes":data.len(),
            "target_hex":data.iter().map(|b|format!("{b:02x}")).collect::<String>()
        }))
    } else {
        require(
            metadata.is_file(),
            "diff file changed to an unsupported source type",
        )?;
        let data = super::read_input_bytes(root, relative)?;
        match std::str::from_utf8(&data) {
            Ok(text) if !text.contains('\0') => DiffContent::Text(text.into()),
            _ => DiffContent::Metadata(serde_json::json!({
                "kind":"binary", "sha256":hash(&data), "bytes":data.len()
            })),
        }
    };
    Ok(BTreeMap::from([(relative.into(), content)]))
}

/// Test scope membership using components, including the project scope `.`.
pub fn scope_contains(root: &Path, scope: &str, path: &str) -> Result<bool> {
    let scope = relative(root, scope)?;
    let path = relative(root, path)?;
    Ok(scope == "." || Path::new(&path).starts_with(scope))
}
fn directory_body(text: &str) -> Option<DirectoryInput> {
    serde_json::from_str::<DirectoryInput>(text)
        .ok()
        .filter(|d| d.kind == "directory" && d.format == "antislop-directory-v1")
}
/// Resolve a text leaf or an observed absence without expanding declared inputs.
pub fn captured_text(
    root: &Path,
    snapshot: &BTreeMap<String, String>,
    path: &str,
) -> Result<Option<String>> {
    captured_text_observed(root, snapshot, None, path)
}
pub fn captured_text_observed(
    root: &Path,
    snapshot: &BTreeMap<String, String>,
    observation: Option<&super::observation::Observation>,
    path: &str,
) -> Result<Option<String>> {
    if let Some(observation) = observation {
        observation.validate(snapshot)?;
    }
    let edited = relative(root, path)?;
    require(
        edited != "."
            && !Path::new(&edited).components().any(|p| {
                p.as_os_str()
                    .to_str()
                    .is_some_and(|p| p.eq_ignore_ascii_case(".git"))
            }),
        "file effect cannot target project root or Git administration",
    )?;
    let mut version = None;
    let mut covered = false;
    for (scope, text) in snapshot {
        let directory_kind =
            observation.map(|o| o.entries[scope].kind == super::observation::Kind::Directory);
        let scope = relative(root, scope)?;
        let observed = if scope == edited {
            require(
                directory_kind.map_or_else(
                    || directory_body(text).is_none(),
                    |is_directory| !is_directory,
                ),
                "file effect cannot replace a directory input",
            )?;
            Some(Some(text.clone()))
        } else if scope == "." || edited.starts_with(&format!("{scope}/")) {
            if directory_kind == Some(false) {
                continue;
            }
            let Some(directory) = directory_body(text) else {
                continue;
            };
            let member = if scope == "." {
                edited.as_str()
            } else {
                &edited[scope.len() + 1..]
            };
            require(
                !directory
                    .excluded_runtime_state
                    .iter()
                    .any(|p| member == p || member.starts_with(&format!("{p}/"))),
                "file effect targets excluded runtime state",
            )?;
            let entries = directory_diff_entries(".", text)?.expect("directory envelope");
            Some(match entries.get(member) {
                Some(DiffContent::Text(text)) => Some(text.clone()),
                Some(DiffContent::Metadata(_)) => {
                    return Err(crate::Error::Invalid(
                        "file effect requires a text leaf, not binary data or a link".into(),
                    ));
                }
                None => None,
            })
        } else {
            None
        };
        if let Some(observed) = observed {
            require(
                !covered || version == observed,
                "overlapping input scopes disagree about file version",
            )?;
            covered = true;
            version = observed;
        }
    }
    require(covered, "file effect is outside captured input scopes")?;
    Ok(version)
}
pub fn absent_hash() -> String {
    hash("antislop-observed-absent-file-v1")
}
pub fn version_hash(version: Option<&str>) -> String {
    version.map_or_else(absent_hash, hash)
}
pub fn after_text(edit: &crate::domain::FileEdit) -> Option<&str> {
    (edit.operation != crate::domain::FileOperation::Delete).then_some(edit.content.as_str())
}
pub fn validate_creation(
    root: &Path,
    snapshot: &BTreeMap<String, String>,
    path: &str,
) -> Result<()> {
    let relative = relative(root, path)?;
    let mut target = root.canonicalize()?;
    let parts: Vec<_> = Path::new(&relative).components().collect();
    for (index, part) in parts.iter().enumerate() {
        target.push(part.as_os_str());
        match target.symlink_metadata() {
            Ok(metadata) => require(
                index + 1 < parts.len() && metadata.is_dir() && !metadata.file_type().is_symlink(),
                "create target exists or its ancestor is not an ordinary directory",
            )?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
    }
    if snapshot
        .values()
        .any(|text| directory_body(text).is_some_and(|d| d.selection.starts_with("Git ")))
    {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["check-ignore", "--quiet", "--no-index", "--", &relative])
            .output()?;
        require(
            output.status.code() == Some(1),
            "create target is ignored or Git membership cannot be verified",
        )?;
    }
    Ok(())
}
/// Advance only one authenticated effect; never reread unrelated filesystem changes.
pub fn record_change(
    root: &Path,
    snapshot: &mut BTreeMap<String, String>,
    edit: &crate::domain::FileEdit,
    expected_hash: &str,
) -> Result<()> {
    record_change_observed(root, snapshot, None, edit, expected_hash)
}
pub fn record_change_observed(
    root: &Path,
    snapshot: &mut BTreeMap<String, String>,
    observation: Option<&super::observation::Observation>,
    edit: &crate::domain::FileEdit,
    expected_hash: &str,
) -> Result<()> {
    use crate::domain::FileOperation;
    require(
        !edit.content.contains('\0'),
        "text file effects cannot carry binary data",
    )?;
    let before = captured_text_observed(root, snapshot, observation, &edit.path)?;
    require(
        version_hash(before.as_deref()) == expected_hash
            && match edit.operation {
                FileOperation::Create => before.is_none(),
                FileOperation::Write | FileOperation::Delete => before.is_some(),
            },
        "managed effect does not match its pinned before version",
    )?;
    require(
        edit.operation != FileOperation::Delete || edit.content.is_empty(),
        "delete must not carry replacement content",
    )?;
    let edited = relative(root, &edit.path)?;
    let mut updated = snapshot.clone();
    let mut covered = false;
    for (scope, text) in &mut updated {
        let directory_kind =
            observation.map(|o| o.entries[scope].kind == super::observation::Kind::Directory);
        let scope = relative(root, scope)?;
        if scope == edited {
            require(
                edit.operation == FileOperation::Write,
                "create/delete requires a captured containing directory",
            )?;
            *text = edit.content.clone();
            covered = true;
            continue;
        }
        if directory_kind == Some(false) {
            continue;
        }
        let Some(mut directory) = directory_body(text) else {
            continue;
        };
        let member = if scope == "." {
            Some(edited.as_str())
        } else {
            edited.strip_prefix(&format!("{scope}/"))
        };
        if let Some(member) = member {
            change_member(&mut directory, member, edit)?;
            *text = serde_json::to_string(&directory)?;
            covered = true;
        }
    }
    require(covered, "managed effect has no captured source")?;
    *snapshot = updated;
    Ok(())
}
pub fn record_edit(
    root: &Path,
    snapshot: &mut BTreeMap<String, String>,
    path: &str,
    expected_hash: &str,
    content: &str,
) -> Result<()> {
    record_change(
        root,
        snapshot,
        &crate::domain::FileEdit {
            path: path.into(),
            content: content.into(),
            operation: crate::domain::FileOperation::Write,
        },
        expected_hash,
    )
}
fn change_member(
    directory: &mut DirectoryInput,
    member: &str,
    edit: &crate::domain::FileEdit,
) -> Result<()> {
    for (name, entry) in &mut directory.files {
        if entry.kind == "directory"
            && let Some(member) = member.strip_prefix(&format!("{name}/"))
        {
            let mut nested: DirectoryInput = serde_json::from_str(
                entry
                    .text
                    .as_deref()
                    .ok_or_else(|| crate::Error::Invalid("nested source missing".into()))?,
            )?;
            change_member(&mut nested, member, edit)?;
            let text = serde_json::to_string(&nested)?;
            entry.sha256 = Some(hash(&text));
            entry.text = Some(text);
            return Ok(());
        }
    }
    let tracked = directory.files.get(member).is_some_and(|e| e.tracked);
    if let Some(content) = after_text(edit) {
        directory.files.insert(
            member.into(),
            Entry {
                tracked,
                kind: if content.contains('\0') {
                    "binary"
                } else {
                    "text"
                }
                .into(),
                sha256: Some(hash(content)),
                text: (!content.contains('\0')).then(|| content.into()),
                bytes: Some(content.len() as u64),
                target_hex: None,
            },
        );
    } else if tracked {
        directory.files.insert(
            member.into(),
            Entry {
                tracked,
                kind: "missing".into(),
                sha256: None,
                text: None,
                bytes: None,
                target_hex: None,
            },
        );
    } else {
        directory.files.remove(member);
    }
    Ok(())
}

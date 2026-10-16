//! Runtime-observed source types, bound to exact captured bodies. Payload JSON
//! never determines whether its enclosing input was a directory or a file.
use crate::{Result, hash, require};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    os::unix::fs::MetadataExt,
    path::{Component, Path},
};

pub const FORMAT: &str = "input-observation-v1";
pub(crate) fn append_prompt(prompt: &mut String, observation: Option<&Observation>) -> Result<()> {
    if let Some(observation) = observation {
        prompt.push_str("\nPinned runtime input types: ");
        prompt.push_str(&serde_json::to_string(observation)?);
        prompt.push_str("\nThese are the captured enclosing filesystem types, bound to original source bytes. A regular file containing directory-shaped JSON remains a file; do not invent children or instruction scopes from its payload. This observation does not expand input selectors, edit scopes or runtime permissions.");
    }
    Ok(())
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    File,
    Directory,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub kind: Kind,
    pub source_hash: String,
    pub utf8_bytes: usize,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub format: String,
    pub snapshot_hash: String,
    pub entries: BTreeMap<String, Entry>,
}
pub struct Capture {
    pub snapshot: BTreeMap<String, String>,
    pub observation: Observation,
}
/// Advance content and its type binding together after a mediated text effect.
/// Both records remain unchanged if either update is refused.
pub fn record_change(
    root: &Path,
    snapshot: &mut BTreeMap<String, String>,
    observation: &mut Option<Observation>,
    edit: &crate::domain::FileEdit,
    expected: &str,
) -> Result<()> {
    if let Some(observation) = observation.as_ref() {
        observation.validate(snapshot)?;
    }
    let mut next_observation = match observation {
        Some(observation) => Some(observation.clone()),
        None => {
            super::legacy_input_types_known(root, snapshot)?;
            let entries = snapshot
                .iter()
                .map(|(scope, body)| {
                    Ok((
                        scope.clone(),
                        Entry {
                            kind: if super::inputs::relative(root, scope)? == "." {
                                Kind::Directory
                            } else {
                                Kind::File
                            },
                            source_hash: hash(body),
                            utf8_bytes: body.len(),
                        },
                    ))
                })
                .collect::<Result<_>>()?;
            Some(Observation {
                format: FORMAT.into(),
                snapshot_hash: hash(serde_json::to_vec(snapshot)?),
                entries,
            })
        }
    };
    let mut next = snapshot.clone();
    super::inputs::record_change_observed(
        root,
        &mut next,
        next_observation.as_ref(),
        edit,
        expected,
    )?;
    if let Some(observation) = &mut next_observation {
        observation.advance(&next)?;
    }
    *snapshot = next;
    *observation = next_observation;
    Ok(())
}
impl Observation {
    pub(crate) fn baseline(
        &self,
        root: &Path,
        snapshot: &BTreeMap<String, String>,
    ) -> Result<Self> {
        let mut version = self.clone();
        for (scope, body) in snapshot {
            if !version.entries.contains_key(scope) {
                require(
                    super::inputs::relative(root, scope)? == ".",
                    "baseline source type missing",
                )?;
                version.entries.insert(
                    scope.clone(),
                    Entry {
                        kind: Kind::Directory,
                        source_hash: hash(body),
                        utf8_bytes: body.len(),
                    },
                );
            }
        }
        version
            .entries
            .retain(|scope, _| snapshot.contains_key(scope));
        version.advance(snapshot)?;
        Ok(version)
    }
    /// A mediated version retains its selected roots' types. This derives a
    /// binding for those new bytes; it never claims a fresh filesystem read.
    pub(crate) fn rebound(&self, snapshot: &BTreeMap<String, String>) -> Result<Self> {
        let mut version = self.clone();
        version.advance(snapshot)?;
        Ok(version)
    }
    pub fn manifest(&self, selector: &str, body: &str) -> Result<serde_json::Value> {
        let entry = self
            .entries
            .get(selector)
            .ok_or_else(|| crate::Error::Invalid("input manifest type missing".into()))?;
        require(
            entry.source_hash == hash(body) && entry.utf8_bytes == body.len(),
            "input manifest binding changed",
        )?;
        Ok(if entry.kind == Kind::Directory {
            super::inputs::context_members(body)?;
            super::inputs::manifest(body)
        } else {
            serde_json::json!({"kind":if body.contains('\0') {"binary"} else {"text"},"sha256":entry.source_hash,"bytes":entry.utf8_bytes})
        })
    }
    pub fn validate(&self, snapshot: &BTreeMap<String, String>) -> Result<()> {
        require(
            self.format == FORMAT
                && self.snapshot_hash == hash(serde_json::to_vec(snapshot)?)
                && self.entries.len() == snapshot.len(),
            "input type observation differs from captured sources",
        )?;
        for (selector, body) in snapshot {
            let entry = self.entries.get(selector).ok_or_else(|| {
                crate::Error::Invalid("input type observation missing source".into())
            })?;
            require(
                entry.source_hash == hash(body) && entry.utf8_bytes == body.len(),
                "input type source binding changed",
            )?;
            if entry.kind == Kind::Directory {
                super::inputs::context_members(body)?;
            }
        }
        Ok(())
    }
    /// Only mediated text effects preserve the selected root's file/directory
    /// type. Uncertain external effects require a new filesystem observation.
    pub(crate) fn advance(&mut self, snapshot: &BTreeMap<String, String>) -> Result<()> {
        require(
            self.entries.keys().eq(snapshot.keys()),
            "mediated effect changed input selectors",
        )?;
        for (selector, body) in snapshot {
            let entry = self.entries.get_mut(selector).unwrap();
            entry.source_hash = hash(body);
            entry.utf8_bytes = body.len();
        }
        self.snapshot_hash = hash(serde_json::to_vec(snapshot)?);
        self.validate(snapshot)
    }
}

fn path(root: &Path, selector: &str) -> Result<std::path::PathBuf> {
    let relative = super::inputs::relative(root, selector)?;
    let mut path = root.to_path_buf();
    for part in Path::new(&relative).components() {
        if let Component::Normal(part) = part {
            path.push(part);
            require(
                !path.symlink_metadata()?.file_type().is_symlink(),
                "symlink input scopes are not followed",
            )?;
        }
    }
    Ok(path)
}
pub fn capture(root: &Path, selectors: &[String]) -> Result<Capture> {
    let root = root.canonicalize()?;
    require(root.is_dir(), "workspace root must be a directory")?;
    let mut snapshot = BTreeMap::new();
    let mut entries = BTreeMap::new();
    for selector in selectors {
        let selected = path(&root, selector)?;
        let before = selected.symlink_metadata()?;
        require(
            before.is_dir() || before.is_file(),
            "input must be a regular file or directory",
        )?;
        let kind = if before.is_dir() {
            Kind::Directory
        } else {
            Kind::File
        };
        let body = if kind == Kind::Directory {
            super::inputs::directory(&selected)?
        } else {
            use std::{io::Read, os::unix::fs::OpenOptionsExt};
            let mut file = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(&selected)?;
            let opened = file.metadata()?;
            require(
                opened.is_file() && (opened.dev(), opened.ino()) == (before.dev(), before.ino()),
                "input replaced during capture",
            )?;
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)?;
            String::from_utf8(bytes).map_err(|_| crate::Error::Invalid("pinned file input is not UTF-8; select its containing directory for binary metadata".into()))?
        };
        let after = path(&root, selector)?.symlink_metadata()?;
        require(
            (before.dev(), before.ino(), before.is_dir())
                == (after.dev(), after.ino(), after.is_dir())
                && (kind == Kind::Directory
                    || (before.len(), before.mtime(), before.mtime_nsec())
                        == (after.len(), after.mtime(), after.mtime_nsec())),
            "input changed during type/content capture",
        )?;
        require(
            snapshot.insert(selector.clone(), body.clone()).is_none(),
            "duplicate input selector",
        )?;
        entries.insert(
            selector.clone(),
            Entry {
                kind,
                source_hash: hash(&body),
                utf8_bytes: body.len(),
            },
        );
    }
    let observation = Observation {
        format: FORMAT.into(),
        snapshot_hash: hash(serde_json::to_vec(&snapshot)?),
        entries,
    };
    observation.validate(&snapshot)?;
    Ok(Capture {
        snapshot,
        observation,
    })
}

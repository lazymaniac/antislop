//! Write-ahead edit receipts. Recovery observes effects; it never replays writes.
use crate::{Result, domain::*, hash, require};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, BTreeMap};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EditState { Pending, Applied, Unapplied, Conflict }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditReceipt {
    pub path: String,
    pub before_hash: String,
    pub after_hash: String,
    pub state: EditState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag="state", rename_all="snake_case")]
pub enum InputObservation {
    Readable { hash: String },
    Missing,
    Unreadable { hash: Option<String>, reason: String },
}
pub struct Observation {
    pub inputs: BTreeMap<String, InputObservation>,
    pub readable: Option<BTreeMap<String, String>>,
    pub hash: String,
}
pub fn observe(run: &Run) -> Result<Observation> {
    let root=std::path::Path::new(&run.spec.workspace);
    let mut inputs=BTreeMap::new(); let mut readable=BTreeMap::new(); let mut complete=true;
    for path in &run.spec.input_files {
        let state=match crate::workspace::read_input_bytes(root, path) {
            Ok(bytes) => {
                let digest=hash(&bytes);
                match String::from_utf8(bytes) {
                    Ok(text) => { readable.insert(path.clone(),text); InputObservation::Readable{hash:digest} },
                    Err(_) => { complete=false; InputObservation::Unreadable{hash:Some(digest),reason:"non-UTF8 input".into()} },
                }
            }
            Err(crate::Error::Io(error)) if error.kind()==std::io::ErrorKind::NotFound => { complete=false; InputObservation::Missing },
            Err(error) => { complete=false; InputObservation::Unreadable{hash:None,reason:error.to_string()} },
        };
        inputs.insert(path.clone(),state);
    }
    let digest=hash(serde_json::to_vec(&inputs)?);
    Ok(Observation{inputs,readable:complete.then_some(readable),hash:digest})
}

pub fn prepare(run: &Run, edits: &[FileEdit]) -> Result<Vec<EditReceipt>> {
    let current = crate::workspace::read_inputs(std::path::Path::new(&run.spec.workspace), &run.spec.input_files)?;
    require(current == run.input_snapshot, "pinned inputs changed; explicit revision required")?;
    let mut seen = BTreeSet::new();
    edits.iter().map(|edit| {
        require(run.spec.editable_files.contains(&edit.path) && seen.insert(&edit.path), "unauthorized or duplicate file edit")?;
        let before = current.get(&edit.path).ok_or_else(|| crate::Error::Invalid("edit is not a pinned input".into()))?;
        Ok(EditReceipt { path:edit.path.clone(), before_hash:hash(before), after_hash:hash(&edit.content), state:EditState::Pending })
    }).collect()
}

pub fn inspect(run: &mut Run) -> Result<()> {
    let current = observe(run)?;
    for receipt in &mut run.pending_edits {
        let observed = match current.inputs.get(&receipt.path) {
            Some(InputObservation::Readable{hash}) => Some(hash),
            Some(InputObservation::Unreadable{hash:Some(hash),..}) => Some(hash),
            _ => None,
        };
        receipt.state = if observed == Some(&receipt.after_hash) {
            EditState::Applied
        } else if observed == Some(&receipt.before_hash) {
            EditState::Unapplied
        } else { EditState::Conflict };
    }
    Ok(())

}

/// Shared ancestor locks allow siblings to run independently while serializing
/// overlapping (including nested) workspaces across daemons and state directories.
pub struct WorkspaceLease { files: Vec<std::fs::File> }
impl WorkspaceLease {
    pub fn acquire(workspace: &str) -> Result<Self> {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::{OpenOptionsExt, MetadataExt, PermissionsExt};
        let root = std::fs::canonicalize(workspace)?;
        // SAFETY: geteuid has no arguments or failure modes.
        let uid = unsafe { libc::geteuid() };
        let directory = std::env::temp_dir().join(format!("architect-leases-{uid}"));
        crate::ipc::private_directory(&directory)?;
        let mut files = Vec::new();
        let ancestors: Vec<_> = root.ancestors().collect();
        for ancestor in ancestors.into_iter().rev() {
            let path = directory.join(hash(ancestor.as_os_str().as_encoded_bytes()));
            let file = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false)
                .mode(0o600).custom_flags(libc::O_NOFOLLOW).open(path)?;
            let metadata = file.metadata()?;
            require(metadata.is_file() && metadata.uid() == uid && metadata.permissions().mode() & 0o077 == 0,
                "unsafe workspace lease file")?;
            let kind = if ancestor == root { libc::LOCK_EX } else { libc::LOCK_SH };
            // SAFETY: flock uses a live owned fd and is nonblocking.
            require(unsafe { libc::flock(file.as_raw_fd(), kind | libc::LOCK_NB) } == 0,
                "workspace has active work; wait for process cleanup before retrying")?;
            files.push(file);
        }
        Ok(Self { files })
    }
    pub fn fds(&self) -> Vec<std::os::fd::RawFd> {
        use std::os::fd::AsRawFd;
        self.files.iter().map(|f| f.as_raw_fd()).collect()
    }
}

pub fn overlaps(left: &str, right: &str) -> bool {
    let left = std::path::Path::new(left);
    let right = std::path::Path::new(right);
    left.starts_with(right) || right.starts_with(left)
}

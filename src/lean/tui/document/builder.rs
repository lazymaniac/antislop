use crate::Result;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};

#[derive(Debug)]
pub(super) struct Files(pub(super) PathBuf);
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Debug, Clone)]
pub struct Document {
    // Shared with the sibling layout worker, which owns index scheduling and reads.
    pub(super) files: Arc<Files>,
    /// Byte ranges of the blocks in the text file, in order.
    pub(super) ranges: Arc<Vec<(u64, u64)>>,
    pub(super) owner: Option<Arc<()>>,
    pub(super) alive: Weak<()>,
    pub(super) layouts: Arc<Mutex<Vec<(LayoutKey, LayoutState)>>>,
}
impl PartialEq for Document {
    fn eq(&self, other: &Self) -> bool {
        self.files.0 == other.files.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LayoutKey {
    pub(super) width: usize,
    pub(super) query: String,
}

#[derive(Debug, Clone)]
pub(super) enum LayoutState {
    Building,
    Ready(Arc<LayoutIndex>),
    Failed(String),
}
#[derive(Debug)]
pub(super) struct LayoutIndex {
    pub(super) path: PathBuf,
    pub(super) lines: usize,
    pub(super) text: PathBuf,
}
impl Drop for LayoutIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub struct Builder {
    pub(super) files: Arc<Files>,
    text: File,
    ranges: Vec<(u64, u64)>,
    offset: u64,
}

pub(super) fn private_file(path: &Path) -> Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    Ok(OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?)
}

impl Builder {
    pub fn new(state: &Path) -> Result<Self> {
        let directory = state.join("inspection-cache").join(crate::id());
        crate::lean::daemon::private_directory(&directory)?;
        let files = Arc::new(Files(directory));
        let text = private_file(&files.0.join("text"))?;
        Ok(Self {
            files,
            text,
            ranges: Vec::new(),
            offset: 0,
        })
    }

    /// Adds one block: `header`, then the chunks `source` delivers (successive
    /// UTF-8 pieces, never a whole history).
    pub fn append(
        &mut self,
        header: &str,
        source: impl FnOnce(&mut dyn FnMut(&str) -> Result<()>) -> Result<()>,
    ) -> Result<()> {
        let start = self.offset;
        self.text.write_all(header.as_bytes())?;
        self.offset += header.len() as u64;
        let mut sanitizer = crate::lean::tui::sanitize::Sanitizer::default();
        source(&mut |chunk| {
            let clean = sanitizer.push(chunk);
            self.text.write_all(clean.as_bytes())?;
            self.offset += clean.len() as u64;
            Ok(())
        })?;
        let end = self.offset;
        self.text.write_all(b"\n")?;
        self.offset += 1;
        self.ranges.push((start, end));
        Ok(())
    }

    pub fn finish(mut self) -> Result<Document> {
        self.text.flush()?;
        let owner = Arc::new(());
        Ok(Document {
            alive: Arc::downgrade(&owner),
            owner: Some(owner),
            files: self.files,
            ranges: Arc::new(self.ranges),
            layouts: Arc::new(Mutex::new(Vec::new())),
        })
    }
}

/// A viewport page, with each row's dimmed-search-match flag.
pub struct Viewport {
    pub lines: Vec<(bool, String)>,
    pub total: usize,
    pub scroll: usize,
}

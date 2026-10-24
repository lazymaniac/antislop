//! Disk-backed inspection documents (a task's result, its messages). Source is
//! transferred once in bounded chunks. Wrapping runs off the UI thread and its
//! index lives on disk; drawing reads only the viewport, independently of the
//! size of the text.
use crate::Result;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

const CHUNK: usize = 64 * 1024;
const INDEX_BYTES: u64 = 17;

#[derive(Debug)]
struct Files(PathBuf);
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Debug, Clone)]
pub struct Document {
    files: Arc<Files>,
    /// Byte ranges of the blocks in the text file, in order.
    ranges: Arc<Vec<(u64, u64)>>,
    owner: Option<Arc<()>>,
    alive: Weak<()>,
    layouts: Arc<Mutex<Vec<(LayoutKey, LayoutState)>>>,
}
impl PartialEq for Document {
    fn eq(&self, other: &Self) -> bool {
        self.files.0 == other.files.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LayoutKey {
    width: usize,
    query: String,
}

#[derive(Debug, Clone)]
enum LayoutState {
    Building,
    Ready(Arc<LayoutIndex>),
    Failed(String),
}
#[derive(Debug)]
struct LayoutIndex {
    path: PathBuf,
    lines: usize,
    text: PathBuf,
}
impl Drop for LayoutIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub struct Builder {
    files: Arc<Files>,
    text: File,
    ranges: Vec<(u64, u64)>,
    offset: u64,
}

fn private_file(path: &Path) -> Result<File> {
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
        let mut sanitizer = super::sanitize::Sanitizer::default();
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

pub struct Viewport {
    pub lines: Vec<(bool, String)>,
    pub total: usize,
    pub scroll: usize,
}
impl Document {
    /// None means the background index is still being built. At most four
    /// viewport layouts are cached; evicting a layout never removes source.
    pub fn viewport(
        &self,
        width: usize,
        height: usize,
        scroll: usize,
        follow: bool,
    ) -> Result<Option<Viewport>> {
        self.viewport_filtered(width, height, scroll, follow, "")
    }

    /// Like [`viewport`](Self::viewport), showing only the blocks that
    /// contain `query` (case-insensitive).
    pub fn viewport_filtered(
        &self,
        width: usize,
        height: usize,
        scroll: usize,
        follow: bool,
        query: &str,
    ) -> Result<Option<Viewport>> {
        let key = LayoutKey {
            width: width.max(1),
            query: query.trim().to_lowercase(),
        };
        let mut layouts = self.layouts.lock().expect("document layouts");
        let state = layouts
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, state)| state.clone());
        let index = match state {
            Some(LayoutState::Ready(index)) => index,
            Some(LayoutState::Failed(error)) => return Err(crate::Error::Invalid(error)),
            Some(LayoutState::Building) => return Ok(None),
            None => {
                if layouts.len() >= 4 {
                    // Never evict an in-progress layout: resize storms must
                    // not start an unbounded number of background threads.
                    if let Some(index) = layouts
                        .iter()
                        .position(|(_, state)| !matches!(state, LayoutState::Building))
                    {
                        layouts.remove(index);
                    } else {
                        return Ok(None);
                    }
                }
                layouts.push((key.clone(), LayoutState::Building));
                let mut document = self.clone();
                // Background work must not keep a closed/replaced view alive.
                document.owner = None;
                std::thread::spawn(move || {
                    let result = document.build_index(&key);
                    let mut layouts = document.layouts.lock().expect("document layouts");
                    if let Some((_, state)) = layouts.iter_mut().find(|(k, _)| *k == key) {
                        *state = match result {
                            Ok(index) => LayoutState::Ready(Arc::new(index)),
                            Err(error) => LayoutState::Failed(error.to_string()),
                        };
                    }
                });
                return Ok(None);
            }
        };
        drop(layouts);
        let bottom = index.lines.saturating_sub(height);
        let scroll = if follow { bottom } else { scroll.min(bottom) };
        let mut positions = File::open(&index.path)?;
        positions.seek(SeekFrom::Start(scroll as u64 * INDEX_BYTES))?;
        let mut text = File::open(&index.text)?;
        let mut lines = Vec::with_capacity(height.min(index.lines));
        for _ in scroll..scroll.saturating_add(height).min(index.lines) {
            let mut row = [0u8; INDEX_BYTES as usize];
            positions.read_exact(&mut row)?;
            let start = u64::from_le_bytes(row[..8].try_into().expect("offset"));
            let end = u64::from_le_bytes(row[8..16].try_into().expect("offset"));
            text.seek(SeekFrom::Start(start))?;
            let mut bytes = vec![0; (end - start) as usize];
            text.read_exact(&mut bytes)?;
            lines.push((
                row[16] != 0,
                String::from_utf8(bytes).map_err(|e| crate::Error::Invalid(e.to_string()))?,
            ));
        }
        Ok(Some(Viewport {
            lines,
            total: index.lines,
            scroll,
        }))
    }

    fn build_index(&self, key: &LayoutKey) -> Result<LayoutIndex> {
        crate::require(self.alive.strong_count() > 0, "inspection view was closed")?;
        let LayoutKey { width, query } = key;
        let text = self.files.0.join("text");
        let path = self.files.0.join(format!("layout-{}", crate::id()));
        let mut output = private_file(&path)?;
        let mut lines = 0;
        for &(start, end) in self.ranges.iter() {
            if !range_matches(&text, start, end, query, &self.alive)? {
                continue;
            }
            lines += wrap_range(&text, &mut output, start, end, *width, false, &self.alive)?;
        }
        output.flush()?;
        Ok(LayoutIndex { path, lines, text })
    }
}

/// Streaming, case-insensitive substring search. Only the query and one
/// transfer chunk are resident, including matches across chunk boundaries.
fn range_matches(path: &Path, start: u64, end: u64, query: &str, alive: &Weak<()>) -> Result<bool> {
    let pattern = query.chars().collect::<Vec<_>>();
    if pattern.is_empty() {
        return Ok(true);
    }
    let mut prefixes = vec![0; pattern.len()];
    for i in 1..pattern.len() {
        let mut j = prefixes[i - 1];
        while j > 0 && pattern[i] != pattern[j] {
            j = prefixes[j - 1];
        }
        if pattern[i] == pattern[j] {
            j += 1;
        }
        prefixes[i] = j;
    }
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(start))?;
    let mut remaining = end - start;
    let mut matched = 0;
    while remaining > 0 {
        crate::require(alive.strong_count() > 0, "inspection view was closed")?;
        let mut bytes = vec![0; remaining.min(CHUNK as u64) as usize];
        file.read_exact(&mut bytes)?;
        remaining -= bytes.len() as u64;
        while std::str::from_utf8(&bytes).is_err() && remaining > 0 {
            let mut byte = [0];
            file.read_exact(&mut byte)?;
            bytes.push(byte[0]);
            remaining -= 1;
        }
        let chunk =
            std::str::from_utf8(&bytes).map_err(|e| crate::Error::Invalid(e.to_string()))?;
        for c in chunk.chars().flat_map(char::to_lowercase) {
            while matched > 0 && c != pattern[matched] {
                matched = prefixes[matched - 1];
            }
            if c == pattern[matched] {
                matched += 1;
            }
            if matched == pattern.len() {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// A line index records byte ranges only. Last grapheme is carried between
/// chunks, including ZWJ sequences, combining marks and regional indicators.
fn wrap_range(
    path: &Path,
    output: &mut File,
    start: u64,
    end: u64,
    width: usize,
    dim: bool,
    alive: &Weak<()>,
) -> Result<usize> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(start))?;
    let mut remaining = end - start;
    let mut pending = String::new();
    let mut base = start;
    let mut line = start;
    let mut columns = 0;
    let mut count = 0;
    let mut emit = |a: u64, b: u64| -> Result<()> {
        output.write_all(&a.to_le_bytes())?;
        output.write_all(&b.to_le_bytes())?;
        output.write_all(&[u8::from(dim)])?;
        count += 1;
        Ok(())
    };
    loop {
        crate::require(alive.strong_count() > 0, "inspection view was closed")?;
        if remaining > 0 {
            let mut bytes = vec![0; remaining.min(CHUNK as u64) as usize];
            file.read_exact(&mut bytes)?;
            remaining -= bytes.len() as u64;
            // Keep UTF-8 and grapheme boundaries independently: the former
            // may require up to three extra bytes from the next chunk.
            while std::str::from_utf8(&bytes).is_err() && remaining > 0 {
                let mut byte = [0];
                file.read_exact(&mut byte)?;
                bytes.push(byte[0]);
                remaining -= 1;
            }
            pending.push_str(
                std::str::from_utf8(&bytes).map_err(|e| crate::Error::Invalid(e.to_string()))?,
            );
        }
        let keep = if remaining == 0 {
            pending.len()
        } else {
            pending
                .grapheme_indices(true)
                .next_back()
                .map_or(0, |(i, _)| i)
        };
        for (offset, grapheme) in pending[..keep].grapheme_indices(true) {
            let position = base + offset as u64;
            if grapheme == "\n" || grapheme == "\r\n" {
                emit(line, position)?;
                line = position + grapheme.len() as u64;
                columns = 0;
            } else {
                let cells = UnicodeWidthStr::width(grapheme);
                if columns > 0 && columns + cells > width {
                    emit(line, position)?;
                    line = position;
                    columns = 0;
                }
                columns += cells;
            }
        }
        pending.drain(..keep);
        base += keep as u64;
        if remaining == 0 {
            break;
        }
    }
    if line < end {
        emit(line, end)?;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn page(document: &Document, width: usize, height: usize, scroll: usize) -> Viewport {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            if let Some(page) = document.viewport(width, height, scroll, false).unwrap() {
                return page;
            }
            assert!(std::time::Instant::now() < deadline, "layout timed out");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn large_text_renders_only_the_viewport_and_retains_every_byte_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let mut builder = Builder::new(dir.path()).unwrap();
        let chunk = "👩‍💻e\u{301} request\n".repeat(2000);
        builder
            .append("Header\n", |sink| {
                for _ in 0..64 {
                    sink(&chunk)?;
                }
                Ok(())
            })
            .unwrap();
        let document = builder.finish().unwrap();
        let cache = document.files.0.clone();
        let text = std::fs::metadata(cache.join("text")).unwrap().len();
        assert!(text > 1_000_000);
        let first = page(&document, 40, 5, 0);
        assert_eq!(first.lines.len(), 5);
        assert_eq!(first.lines[0].1, "Header");
        assert!(first.lines[1].1.contains("👩‍💻e\u{301} request"));
        let last = page(&document, 40, 5, usize::MAX);
        assert_eq!(last.lines.len(), 5);
        assert!(last.scroll > 100_000);
    }

    #[test]
    fn search_reads_disk_backed_blocks_across_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let mut builder = Builder::new(dir.path()).unwrap();
        builder
            .append("", |sink| {
                sink(&"x".repeat(CHUNK - 1))?;
                sink("Ą👩‍💻needle\n")
            })
            .unwrap();
        builder
            .append("", |sink| sink("unrelated record"))
            .unwrap();
        let document = builder.finish().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        let page = loop {
            if let Some(page) = document
                .viewport_filtered(80, 4, usize::MAX, false, "Ą👩‍💻needle")
                .unwrap()
            {
                break page;
            }
            assert!(std::time::Instant::now() < deadline, "layout timed out");
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        let text = page
            .lines
            .into_iter()
            .map(|(_, s)| s)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Ą👩‍💻needle"));
        assert!(!text.contains("unrelated"));
    }

    #[test]
    fn replacing_a_view_cancels_background_layouts_and_releases_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let mut builder = Builder::new(dir.path()).unwrap();
        let chunk = "long line of text\n".repeat(3000);
        builder
            .append("Header\n", |sink| {
                for _ in 0..64 {
                    sink(&chunk)?;
                }
                Ok(())
            })
            .unwrap();
        let document = builder.finish().unwrap();
        let path = document.files.0.clone();
        let alive = document.alive.clone();
        for width in 1..20 {
            let _ = document.viewport(width, 10, 0, false).unwrap();
        }
        assert!(document.layouts.lock().unwrap().len() <= 4);
        drop(document);
        assert_eq!(alive.strong_count(), 0);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while path.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!path.exists());
    }
}

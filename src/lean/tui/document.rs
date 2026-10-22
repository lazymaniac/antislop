//! Disk-backed inspection documents. Source is transferred once in bounded
//! chunks. Wrapping runs off the UI thread and its index lives on disk; drawing
//! reads only the viewport, independently of the size of the transcript.
use crate::Result;
use rusqlite::{Connection, OptionalExtension, params};
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
    answer_only: bool,
    owner: Option<Arc<()>>,
    alive: Weak<()>,
    layouts: Arc<Mutex<Vec<(LayoutKey, LayoutState)>>>,
}
impl PartialEq for Document {
    fn eq(&self, other: &Self) -> bool {
        self.files.0 == other.files.0 && self.answer_only == other.answer_only
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LayoutKey {
    width: usize,
    expanded: bool,
    answer_only: bool,
    query: String,
    raw: bool,
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
    db: Connection,
    text: File,
    raw: File,
    raw_offset: u64,
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
        let index = files.0.join("blocks.db");
        drop(private_file(&index)?);
        let db = Connection::open(index)?;
        db.execute_batch("PRAGMA cache_size=-256; PRAGMA temp_store=FILE;
            CREATE TABLE blocks(id INTEGER PRIMARY KEY,kind TEXT NOT NULL,header INTEGER NOT NULL,start INTEGER NOT NULL,end INTEGER NOT NULL,duplicate INTEGER NOT NULL DEFAULT 0,raw_start INTEGER NOT NULL,raw_end INTEGER NOT NULL);")?;
        let text = private_file(&files.0.join("text"))?;
        let raw = private_file(&files.0.join("raw"))?;
        let mut builder = Self {
            files,
            db,
            text,
            raw,
            raw_offset: 0,
            offset: 0,
        };
        builder.append("collapsed", "", |sink| sink("Session trace [+]\n"))?;
        builder.append("expanded", "", |sink| sink("Session trace [-]\n"))?;
        builder.append("legacy", "", |sink| sink("Legacy history: final answer was not recorded separately; earlier text may have been truncated.\n"))?;
        Ok(builder)
    }

    /// `source` must deliver successive UTF-8 chunks, never a whole history.
    pub fn append(
        &mut self,
        kind: &str,
        header: &str,
        source: impl FnOnce(&mut dyn FnMut(&str) -> Result<()>) -> Result<()>,
    ) -> Result<()> {
        let raw_start = self.raw_offset;
        let raw_header = format!("{{\"kind\":{},\"text\":\"", serde_json::to_string(kind)?);
        self.raw.write_all(raw_header.as_bytes())?;
        self.raw_offset += raw_header.len() as u64;
        let header_offset = self.offset;
        self.text.write_all(header.as_bytes())?;
        self.offset += header.len() as u64;
        let start = self.offset;
        let mut sanitizer = super::sanitize::Sanitizer::default();
        source(&mut |chunk| {
            let clean = sanitizer.push(chunk);
            self.text.write_all(clean.as_bytes())?;
            let encoded = serde_json::to_string(&clean)?;
            let escaped = &encoded.as_bytes()[1..encoded.len() - 1];
            self.raw.write_all(escaped)?;
            self.raw_offset += escaped.len() as u64;
            self.offset += clean.len() as u64;
            Ok(())
        })?;
        let end = self.offset;
        self.raw.write_all(b"\"}\n")?;
        self.raw_offset += 3;
        let raw_end = self.raw_offset - 1;
        self.text.write_all(b"\n")?;
        self.offset += 1;
        self.db.execute(
            "INSERT INTO blocks(kind,header,start,end,raw_start,raw_end) VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                kind,
                header_offset as i64,
                start as i64,
                end as i64,
                raw_start as i64,
                raw_end as i64
            ],
        )?;
        if kind == "final" {
            self.deduplicate(self.db.last_insert_rowid(), start, end)?;
        }
        Ok(())
    }

    fn deduplicate(&mut self, id: i64, start: u64, end: u64) -> Result<()> {
        if start == end {
            return Ok(());
        }
        self.text.flush()?;
        let mut rows = self
            .db
            .prepare("SELECT id,kind,start,end FROM blocks WHERE id<?1 ORDER BY id DESC")?;
        let mut remaining = end - start;
        let mut first = None;
        for row in rows.query_map([id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)? as u64,
                row.get::<_, i64>(3)? as u64,
            ))
        })? {
            let (candidate, kind, a, b) = row?;
            if kind != "text" || b - a > remaining {
                break;
            }
            if !equal_ranges(
                &self.files.0.join("text"),
                a,
                start + remaining - (b - a),
                b - a,
            )? {
                break;
            }
            remaining -= b - a;
            first = Some(candidate);
            if remaining == 0 {
                break;
            }
        }
        drop(rows);
        if remaining == 0
            && let Some(first) = first
        {
            self.db.execute(
                "UPDATE blocks SET duplicate=1 WHERE id>=?1 AND id<?2 AND kind='text'",
                params![first, id],
            )?;
        }
        Ok(())
    }

    pub fn finish(mut self) -> Result<Document> {
        self.text.flush()?;
        let latest:Option<(i64,i64)>=self.db.query_row("SELECT start,end FROM blocks WHERE kind IN ('prompt','user','text','tool','mail','failure') AND duplicate=0 ORDER BY id DESC LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        if let Some((start, end)) = latest {
            let mut file = File::open(self.files.0.join("text"))?;
            file.seek(SeekFrom::Start(start as u64))?;
            let mut bytes = vec![0; ((end - start) as usize).min(1024)];
            file.read_exact(&mut bytes)?;
            let valid = std::str::from_utf8(&bytes)
                .map(str::len)
                .unwrap_or_else(|e| e.valid_up_to());
            let prefix = std::str::from_utf8(&bytes[..valid]).expect("UTF-8 prefix");
            // A short preview never displays a cluster cut by the read bound.
            let prefix = if (end - start) as usize > valid {
                let keep = prefix
                    .grapheme_indices(true)
                    .next_back()
                    .map_or(0, |(i, _)| i);
                &prefix[..keep]
            } else {
                prefix
            };
            let summary = super::view::truncate(prefix.lines().next().unwrap_or_default(), 100);
            self.db.execute(
                "UPDATE blocks SET kind='retired' WHERE kind='collapsed'",
                [],
            )?;
            self.append("collapsed", "Session trace [+] · ", |sink| sink(&summary))?;
        }
        self.text.flush()?;
        self.raw.flush()?;
        let owner = Arc::new(());
        Ok(Document {
            alive: Arc::downgrade(&owner),
            owner: Some(owner),
            files: self.files,
            answer_only: false,
            layouts: Arc::new(Mutex::new(Vec::new())),
        })
    }
}

fn equal_ranges(path: &Path, a: u64, b: u64, mut length: u64) -> Result<bool> {
    let mut left = File::open(path)?;
    let mut right = File::open(path)?;
    left.seek(SeekFrom::Start(a))?;
    right.seek(SeekFrom::Start(b))?;
    let mut x = [0u8; CHUNK];
    let mut y = [0u8; CHUNK];
    while length > 0 {
        let n = length.min(CHUNK as u64) as usize;
        left.read_exact(&mut x[..n])?;
        right.read_exact(&mut y[..n])?;
        if x[..n] != y[..n] {
            return Ok(false);
        }
        length -= n as u64;
    }
    Ok(true)
}

pub struct Viewport {
    pub lines: Vec<(bool, String)>,
    pub total: usize,
    pub scroll: usize,
}
impl Document {
    pub fn answer(&self) -> Self {
        let mut answer = self.clone();
        answer.answer_only = true;
        answer
    }
    /// None means the background index is still being built. At most four
    /// viewport layouts are cached; evicting a layout never removes source.
    pub fn viewport(
        &self,
        width: usize,
        height: usize,
        scroll: usize,
        follow: bool,
        expanded: bool,
    ) -> Result<Option<Viewport>> {
        self.viewport_options(width, height, scroll, follow, expanded, "", false)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn viewport_options(
        &self,
        width: usize,
        height: usize,
        scroll: usize,
        follow: bool,
        expanded: bool,
        query: &str,
        raw: bool,
    ) -> Result<Option<Viewport>> {
        let key = LayoutKey {
            width: width.max(1),
            expanded,
            answer_only: self.answer_only,
            query: query.trim().to_lowercase(),
            raw,
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
        let LayoutKey {
            width,
            expanded,
            answer_only,
            query,
            raw,
        } = key;
        let (width, expanded, answer_only, raw) = (*width, *expanded, *answer_only, *raw);
        let text = self.files.0.join(if raw { "raw" } else { "text" });
        let columns = if raw {
            "raw_start,raw_end"
        } else {
            "header,end"
        };
        let path = self.files.0.join(format!("layout-{}", crate::id()));
        let mut output = private_file(&path)?;
        let db = Connection::open(self.files.0.join("blocks.db"))?;
        db.execute_batch("PRAGMA cache_size=-256; PRAGMA temp_store=FILE;")?;
        let plain: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM blocks WHERE kind='plain')",
            [],
            |r| r.get(0),
        )?;
        if plain {
            let mut lines = 0;
            let mut statement = db.prepare(&format!(
                "SELECT {columns} FROM blocks WHERE kind='plain' ORDER BY id"
            ))?;
            for row in
                statement.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?
            {
                let (a, b) = row?;
                if !range_matches(&text, a as u64, b as u64, query, &self.alive)? {
                    continue;
                }
                lines += wrap_range(
                    &text,
                    &mut output,
                    a as u64,
                    b as u64,
                    width,
                    false,
                    &self.alive,
                )?;
            }
            output.flush()?;
            return Ok(LayoutIndex { path, lines, text });
        }
        let first: Option<i64> =
            db.query_row("SELECT min(id) FROM blocks WHERE kind='prompt'", [], |r| {
                r.get(0)
            })?;
        let last: Option<i64> =
            db.query_row("SELECT max(id) FROM blocks WHERE kind='final'", [], |r| {
                r.get(0)
            })?;
        let mut lines = 0;
        let mut emit = |start, end, dim| -> Result<()> {
            if !range_matches(&text, start, end, query, &self.alive)? {
                return Ok(());
            }
            lines += wrap_range(&text, &mut output, start, end, width, dim, &self.alive)?;
            Ok(())
        };
        if !answer_only && let Some(id) = first {
            let (a, b): (i64, i64) = db.query_row(
                &format!("SELECT {columns} FROM blocks WHERE id=?1"),
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            emit(a as u64, b as u64, true)?;
        }
        let (a, b): (i64, i64) = db.query_row(
            &format!("SELECT {columns} FROM blocks WHERE kind=?1"),
            [if expanded { "expanded" } else { "collapsed" }],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if !answer_only {
            emit(a as u64, b as u64, true)?;
        }
        if expanded && !answer_only {
            let mut statement = db.prepare(&format!("SELECT kind,{columns} FROM blocks WHERE kind IN ('prompt','user','text','final','tool','mail','failure') AND duplicate=0 AND id<>coalesce(?1,-1) AND id<>coalesce(?2,-1) ORDER BY id"))?;
            for row in statement.query_map(params![first, last], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })? {
                let (kind, a, b) = row?;
                emit(
                    a as u64,
                    b as u64,
                    matches!(kind.as_str(), "prompt" | "tool"),
                )?;
            }
        }
        if let Some(id) = last {
            let (a, b): (i64, i64) = db.query_row(
                &format!("SELECT {columns} FROM blocks WHERE id=?1"),
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            emit(a as u64, b as u64, false)?;
        } else {
            let ended: bool = db.query_row(
                "SELECT EXISTS(SELECT 1 FROM blocks WHERE kind='end')",
                [],
                |r| r.get(0),
            )?;
            if ended {
                let (a, b): (i64, i64) = db.query_row(
                    &format!("SELECT {columns} FROM blocks WHERE kind='legacy'"),
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                emit(a as u64, b as u64, true)?;
            }
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
    fn page(
        document: &Document,
        width: usize,
        height: usize,
        scroll: usize,
        expanded: bool,
    ) -> Viewport {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            if let Some(page) = document
                .viewport(width, height, scroll, false, expanded)
                .unwrap()
            {
                return page;
            }
            assert!(std::time::Instant::now() < deadline, "layout timed out");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn large_history_renders_only_the_viewport_and_retains_every_byte_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let mut builder = Builder::new(dir.path()).unwrap();
        let chunk = "👩‍💻e\u{301} request\n".repeat(2000);
        builder
            .append("prompt", "Prompt\n", |sink| {
                for _ in 0..128 {
                    sink(&chunk)?;
                }
                Ok(())
            })
            .unwrap();
        builder
            .append("user", "User\n", |sink| sink("fix this"))
            .unwrap();
        builder
            .append("tool", "Tool call\n", |sink| sink("cargo check"))
            .unwrap();
        builder.append("text", "", |sink| sink("final ")).unwrap();
        builder
            .append("text", "", |sink| sink("answer 👩‍💻"))
            .unwrap();
        builder
            .append("final", "Final answer\n", |sink| sink("final answer 👩‍💻"))
            .unwrap();
        let document = builder.finish().unwrap();
        let cache = document.files.0.clone();
        let text = std::fs::metadata(cache.join("text")).unwrap().len();
        assert!(text > 4 * 1024 * 1024);
        let first = page(&document, 30, 8, 0, false);
        assert_eq!(first.lines.len(), 8);
        assert!(first.total > 200_000);
        assert!(first.lines.iter().map(|(_, s)| s.len()).sum::<usize>() < 1024);
        let last = page(&document, 30, 8, usize::MAX, true);
        let tail = last
            .lines
            .iter()
            .map(|(_, s)| s.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(tail.matches("final answer").count(), 1);
        assert!(tail.contains("Final answer"));
        let answer = page(&document.answer(), 30, 8, 0, false);
        assert!(answer.total < 8);
        assert_eq!(
            answer
                .lines
                .iter()
                .map(|(_, s)| s.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            "Final answer\nfinal answer 👩‍💻"
        );
        drop(document);
        assert!(!cache.exists());
    }

    #[test]
    fn wrapping_and_escape_sanitization_cross_chunk_boundaries() {
        let dir = tempfile::tempdir().unwrap();
        let mut builder = Builder::new(dir.path()).unwrap();
        builder
            .append("plain", "", |sink| {
                sink(&"x".repeat(CHUNK - 1))?;
                sink("👩")?;
                sink("\u{200d}")?;
                sink("💻e")?;
                sink("\u{301}\n")?;
                sink("\x1b]52;c;secret")?;
                sink("\x1b")?;
                sink("\\visible")
            })
            .unwrap();
        let document = builder.finish().unwrap();
        let result = page(&document, 10, 10, usize::MAX, false);
        let tail = result
            .lines
            .iter()
            .map(|(_, s)| s.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(tail.contains("👩‍💻e\u{301}"));
        assert!(tail.ends_with("visible"));
        assert!(!tail.contains("secret"));
        assert!(result.lines.iter().all(|(_, line)| line.width() <= 10));
    }

    #[test]
    fn earlier_turns_stay_chronological_and_latest_answer_stays_outside_trace() {
        let dir = tempfile::tempdir().unwrap();
        let mut builder = Builder::new(dir.path()).unwrap();
        for (kind, text) in [
            ("prompt", "first prompt"),
            ("text", "first answer"),
            ("final", "first answer"),
            ("prompt", "second prompt"),
            ("user", "follow up"),
            ("text", "second answer"),
            ("final", "second answer"),
        ] {
            builder
                .append(
                    kind,
                    if kind == "final" {
                        "Final answer\n"
                    } else {
                        ""
                    },
                    |sink| sink(text),
                )
                .unwrap();
        }
        let document = builder.finish().unwrap();
        let expanded = page(&document, 80, 40, 0, true)
            .lines
            .into_iter()
            .map(|(_, s)| s)
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(expanded.matches("first answer").count(), 1);
        assert_eq!(expanded.matches("second answer").count(), 1);
        assert!(expanded.find("first answer").unwrap() < expanded.find("second prompt").unwrap());
        let collapsed = page(&document, 80, 40, 0, false)
            .lines
            .into_iter()
            .map(|(_, s)| s)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!collapsed.contains("first answer"));
        assert!(collapsed.contains("first prompt"));
        assert!(collapsed.contains("second answer"));
    }
    #[test]
    fn search_and_raw_views_read_disk_backed_blocks_across_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let mut builder = Builder::new(dir.path()).unwrap();
        builder
            .append("plain", "", |sink| {
                sink(&"x".repeat(CHUNK - 1))?;
                sink("Ą👩‍💻needle\n")
            })
            .unwrap();
        builder
            .append("plain", "", |sink| sink("unrelated record"))
            .unwrap();
        let document = builder.finish().unwrap();
        for raw in [false, true] {
            let key = LayoutKey {
                width: 80,
                expanded: false,
                answer_only: false,
                query: "ą👩‍💻needle".into(),
                raw,
            };
            let index = document.build_index(&key).unwrap();
            document
                .layouts
                .lock()
                .unwrap()
                .push((key, LayoutState::Ready(Arc::new(index))));
            let page = document
                .viewport_options(80, 4, usize::MAX, false, false, "Ą👩‍💻needle", raw)
                .unwrap()
                .unwrap();
            let text = page
                .lines
                .into_iter()
                .map(|(_, s)| s)
                .collect::<Vec<_>>()
                .join("\n");
            assert!(text.contains("Ą👩‍💻needle"));
            assert!(!text.contains("unrelated"));
            if raw {
                assert!(text.contains("\\n\"}"));
            }
        }
    }
    #[test]
    fn replacing_a_view_cancels_background_layouts_and_releases_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let mut builder = Builder::new(dir.path()).unwrap();
        let chunk = "long transcript line\n".repeat(3000);
        builder
            .append("prompt", "Prompt\n", |sink| {
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
            let _ = document.viewport(width, 10, 0, false, true).unwrap();
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

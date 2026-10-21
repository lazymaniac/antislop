//! Disk-backed inspection documents. Source is transferred once in bounded
//! chunks. Wrapping runs off the UI thread and its index lives on disk; drawing
//! reads only the viewport, independently of the size of the transcript.
use crate::Result;
use rusqlite::{Connection, OptionalExtension, params};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
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
    layouts: Arc<Mutex<Vec<((usize, bool, bool), LayoutState)>>>,
}
impl PartialEq for Document {
    fn eq(&self, other: &Self) -> bool {
        self.files.0 == other.files.0 && self.answer_only == other.answer_only
    }
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
            CREATE TABLE blocks(id INTEGER PRIMARY KEY,kind TEXT NOT NULL,header INTEGER NOT NULL,start INTEGER NOT NULL,end INTEGER NOT NULL,duplicate INTEGER NOT NULL DEFAULT 0);")?;
        let text = private_file(&files.0.join("text"))?;
        let mut builder = Self {
            files,
            db,
            text,
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
        let header_offset = self.offset;
        self.text.write_all(header.as_bytes())?;
        self.offset += header.len() as u64;
        let start = self.offset;
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
        self.db.execute(
            "INSERT INTO blocks(kind,header,start,end) VALUES(?1,?2,?3,?4)",
            params![kind, header_offset as i64, start as i64, end as i64],
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
            let summary = super::view::truncate(
                std::str::from_utf8(&bytes[..valid])
                    .expect("UTF-8 prefix")
                    .lines()
                    .next()
                    .unwrap_or_default(),
                100,
            );
            self.db.execute(
                "UPDATE blocks SET kind='retired' WHERE kind='collapsed'",
                [],
            )?;
            self.append("collapsed", "Session trace [+] · ", |sink| sink(&summary))?;
        }
        self.text.flush()?;
        Ok(Document {
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
        let key = (width.max(1), expanded, self.answer_only);
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
                layouts.push((key, LayoutState::Building));
                let document = self.clone();
                std::thread::spawn(move || {
                    let result = document.build_index(key.0, key.1, key.2);
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
        let mut text = File::open(self.files.0.join("text"))?;
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

    fn build_index(&self, width: usize, expanded: bool, answer_only: bool) -> Result<LayoutIndex> {
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
            let mut statement =
                db.prepare("SELECT header,end FROM blocks WHERE kind='plain' ORDER BY id")?;
            for row in
                statement.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?
            {
                let (a, b) = row?;
                lines += wrap_range(
                    &self.files.0.join("text"),
                    &mut output,
                    a as u64,
                    b as u64,
                    width,
                    false,
                )?;
            }
            output.flush()?;
            return Ok(LayoutIndex { path, lines });
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
            lines += wrap_range(
                &self.files.0.join("text"),
                &mut output,
                start,
                end,
                width,
                dim,
            )?;
            Ok(())
        };
        if !answer_only && let Some(id) = first {
            let (a, b): (i64, i64) =
                db.query_row("SELECT header,end FROM blocks WHERE id=?1", [id], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })?;
            emit(a as u64, b as u64, true)?;
        }
        let (a, b): (i64, i64) = db.query_row(
            "SELECT header,end FROM blocks WHERE kind=?1",
            [if expanded { "expanded" } else { "collapsed" }],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if !answer_only {
            emit(a as u64, b as u64, true)?;
        }
        if expanded && !answer_only {
            let mut statement = db.prepare("SELECT kind,header,end FROM blocks WHERE kind IN ('prompt','user','text','final','tool','mail','failure') AND duplicate=0 AND id<>coalesce(?1,-1) AND id<>coalesce(?2,-1) ORDER BY id")?;
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
            let (a, b): (i64, i64) =
                db.query_row("SELECT header,end FROM blocks WHERE id=?1", [id], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })?;
            emit(a as u64, b as u64, false)?;
        } else {
            let ended: bool = db.query_row(
                "SELECT EXISTS(SELECT 1 FROM blocks WHERE kind='end')",
                [],
                |r| r.get(0),
            )?;
            if ended {
                let (a, b): (i64, i64) = db.query_row(
                    "SELECT header,end FROM blocks WHERE kind='legacy'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                emit(a as u64, b as u64, true)?;
            }
        }
        output.flush()?;
        Ok(LayoutIndex { path, lines })
    }
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

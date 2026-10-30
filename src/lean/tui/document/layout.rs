use super::INDEX_BYTES;
use super::builder::{Document, LayoutIndex, LayoutKey, LayoutState, Viewport, private_file};
use super::search::range_matches;
use super::wrap::wrap_range;
use crate::Result;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::Arc;

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
        let Some(index) = self.resolve_index(key)? else {
            return Ok(None);
        };
        let bottom = index.lines.saturating_sub(height);
        let scroll = if follow { bottom } else { scroll.min(bottom) };
        read_viewport(&index, height, scroll)
    }

    fn resolve_index(&self, key: LayoutKey) -> Result<Option<Arc<LayoutIndex>>> {
        let mut layouts = self.layouts.lock().expect("document layouts");
        let state = layouts
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, state)| state.clone());
        match state {
            Some(LayoutState::Ready(index)) => Ok(Some(index)),
            Some(LayoutState::Failed(error)) => Err(crate::Error::Invalid(error)),
            Some(LayoutState::Building) => Ok(None),
            None => self.schedule_index(&mut layouts, key),
        }
    }

    fn schedule_index(
        &self,
        layouts: &mut Vec<(LayoutKey, LayoutState)>,
        key: LayoutKey,
    ) -> Result<Option<Arc<LayoutIndex>>> {
        if !make_room(layouts) {
            return Ok(None);
        }
        layouts.push((key.clone(), LayoutState::Building));
        let mut document = self.clone();
        document.owner = None;
        std::thread::spawn(move || document.finish_index(key));
        Ok(None)
    }

    fn finish_index(&self, key: LayoutKey) {
        let result = self.build_index(&key);
        let mut layouts = self.layouts.lock().expect("document layouts");
        if let Some((_, state)) = layouts.iter_mut().find(|(k, _)| *k == key) {
            *state = match result {
                Ok(index) => LayoutState::Ready(Arc::new(index)),
                Err(error) => LayoutState::Failed(error.to_string()),
            };
        }
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

fn make_room(layouts: &mut Vec<(LayoutKey, LayoutState)>) -> bool {
    if layouts.len() < 4 {
        return true;
    }
    if let Some(index) = layouts
        .iter()
        .position(|(_, state)| !matches!(state, LayoutState::Building))
    {
        layouts.remove(index);
        true
    } else {
        false
    }
}

fn read_viewport(index: &LayoutIndex, height: usize, scroll: usize) -> Result<Option<Viewport>> {
    let mut positions = File::open(&index.path)?;
    positions.seek(SeekFrom::Start(scroll as u64 * INDEX_BYTES))?;
    let mut text = File::open(&index.text)?;
    let count = height.min(index.lines.saturating_sub(scroll));
    let lines = read_rows(&mut positions, &mut text, count)?;
    Ok(Some(Viewport {
        lines,
        total: index.lines,
        scroll,
    }))
}

fn read_rows(positions: &mut File, text: &mut File, count: usize) -> Result<Vec<(bool, String)>> {
    let mut lines = Vec::with_capacity(count);
    for _ in 0..count {
        let mut row = [0u8; INDEX_BYTES as usize];
        positions.read_exact(&mut row)?;
        lines.push(read_row(text, row)?);
    }
    Ok(lines)
}

fn read_row(text: &mut File, row: [u8; INDEX_BYTES as usize]) -> Result<(bool, String)> {
    let start = u64::from_le_bytes(row[..8].try_into().expect("offset"));
    let end = u64::from_le_bytes(row[8..16].try_into().expect("offset"));
    text.seek(SeekFrom::Start(start))?;
    let mut bytes = vec![0; (end - start) as usize];
    text.read_exact(&mut bytes)?;
    let text = String::from_utf8(bytes).map_err(|e| crate::Error::Invalid(e.to_string()))?;
    Ok((row[16] != 0, text))
}

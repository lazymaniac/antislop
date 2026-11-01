use super::super::*;
use std::collections::{HashMap, VecDeque};
use std::fmt::{self, Write as _};
use std::hash::{Hash, Hasher};

const ROW_BUDGET: usize = 4096;
const ENTRY_BUDGET: usize = 1024;

#[derive(Default)]
pub(in crate::lean::tui) struct TranscriptLayout {
    pub context: Option<(String, u16, Theme)>,
    pub entries: Vec<TranscriptLayoutEntry>,
    cached: HashMap<usize, CachedRows>,
    order: VecDeque<usize>,
    retained_rows: usize,
    #[cfg(test)]
    pub stores: usize,
}

#[derive(Default)]
pub(in crate::lean::tui) struct TranscriptLayoutEntry {
    pub fingerprint: u64,
    pub found: bool,
    pub height: usize,
    pub end: usize,
}

struct CachedRows {
    fingerprint: u64,
    found: bool,
    rows: Vec<Row>,
}

pub(super) fn fingerprint(
    card: &TranscriptCard,
    comment: &Option<(String, Option<String>)>,
) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let _ = write!(&mut HashWriter(&mut hasher), "{card:?}");
    comment.hash(&mut hasher);
    hasher.finish()
}

struct HashWriter<'a>(&'a mut std::collections::hash_map::DefaultHasher);

impl fmt::Write for HashWriter<'_> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.0.write(text.as_bytes());
        Ok(())
    }
}

impl TranscriptLayout {
    pub fn clear(&mut self) {
        self.entries.clear();
        self.cached.clear();
        self.order.clear();
        self.retained_rows = 0;
    }

    pub fn rows(&self, index: usize, fingerprint: u64, found: bool) -> Option<&[Row]> {
        self.cached
            .get(&index)
            .filter(|entry| entry.fingerprint == fingerprint && entry.found == found)
            .map(|entry| entry.rows.as_slice())
    }

    pub fn store(&mut self, index: usize, fingerprint: u64, found: bool, rows: Vec<Row>) {
        let height = rows.len();
        if height > ROW_BUDGET {
            return;
        }
        self.remove(index);
        while self.retained_rows + height > ROW_BUDGET || self.cached.len() >= ENTRY_BUDGET {
            let Some(index) = self.order.pop_front() else {
                break;
            };
            if let Some(oldest) = self.cached.remove(&index) {
                self.retained_rows -= oldest.rows.len();
            }
        }
        self.retained_rows += height;
        #[cfg(test)]
        {
            self.stores += 1;
        }
        self.cached.insert(
            index,
            CachedRows {
                fingerprint,
                found,
                rows,
            },
        );
        self.order.push_back(index);
    }

    #[cfg(test)]
    pub fn retained_rows(&self) -> usize {
        self.retained_rows
    }

    #[cfg(test)]
    pub fn cached_entries(&self) -> usize {
        self.cached.len()
    }

    fn remove(&mut self, index: usize) {
        if let Some(old) = self.cached.remove(&index) {
            self.retained_rows -= old.rows.len();
            self.order.retain(|cached| *cached != index);
        }
    }
}

impl App {
    pub(super) fn transcript_visible_rows(
        &mut self,
        start: usize,
        end: usize,
        history_height: usize,
        extra: &[Row],
        theme: &Theme,
    ) -> Vec<Row> {
        let mut visible = Vec::with_capacity(end.saturating_sub(start));
        self.append_visible_history(&mut visible, start, end, theme);
        let from = start.saturating_sub(history_height).min(extra.len());
        let to = end.saturating_sub(history_height).min(extra.len());
        visible.extend_from_slice(&extra[from..to]);
        visible
    }

    fn append_visible_history(
        &mut self,
        visible: &mut Vec<Row>,
        start: usize,
        end: usize,
        theme: &Theme,
    ) {
        let entries = &self.transcript_layout.entries;
        let first = entries.partition_point(|entry| entry.end <= start);
        let shown: Vec<_> = entries
            .iter()
            .enumerate()
            .skip(first)
            .take_while(|(_, entry)| entry.end - entry.height < end)
            .map(|(index, entry)| {
                let origin = entry.end - entry.height;
                let slice = RowSlice {
                    from: start.saturating_sub(origin),
                    to: end.saturating_sub(origin).min(entry.height),
                };
                (index, entry.fingerprint, entry.found, slice)
            })
            .collect();
        for (index, fingerprint, found, slice) in shown {
            self.append_card_rows(visible, index, fingerprint, found, slice, theme);
        }
    }

    fn append_card_rows(
        &mut self,
        visible: &mut Vec<Row>,
        index: usize,
        fingerprint: u64,
        found: bool,
        slice: RowSlice,
        theme: &Theme,
    ) {
        if self
            .transcript_layout
            .rows(index, fingerprint, found)
            .is_none()
        {
            let width = self
                .transcript_layout
                .context
                .as_ref()
                .map_or(0, |ctx| ctx.1);
            let rows = self.transcript_card_rows(&self.view.cards[index], found, width, theme);
            if rows.len() > ROW_BUDGET {
                visible.extend_from_slice(&rows[slice.from..slice.to]);
                return;
            }
            self.transcript_layout
                .store(index, fingerprint, found, rows);
        }
        if let Some(rows) = self.transcript_layout.rows(index, fingerprint, found) {
            visible.extend_from_slice(&rows[slice.from..slice.to]);
        }
    }
}

struct RowSlice {
    from: usize,
    to: usize,
}

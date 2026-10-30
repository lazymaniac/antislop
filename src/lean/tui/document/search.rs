use super::chunk::read_utf8_chunk;
use crate::Result;
use std::fs::File;
use std::io::{Seek, SeekFrom};
use std::path::Path;
use std::sync::Weak;

/// Streaming, case-insensitive substring search. Only the query and one
/// transfer chunk are resident, including matches across chunk boundaries.
pub(super) fn range_matches(
    path: &Path,
    start: u64,
    end: u64,
    query: &str,
    alive: &Weak<()>,
) -> Result<bool> {
    let pattern = query.chars().collect::<Vec<_>>();
    if pattern.is_empty() {
        return Ok(true);
    }
    let prefixes = prefix_table(&pattern);
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(start))?;
    let mut remaining = end - start;
    let mut matched = 0;
    while remaining > 0 {
        crate::require(alive.strong_count() > 0, "inspection view was closed")?;
        let chunk = read_utf8_chunk(&mut file, &mut remaining)?;
        if scan_chunk(&chunk, &pattern, &prefixes, &mut matched) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn prefix_table(pattern: &[char]) -> Vec<usize> {
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
    prefixes
}

fn scan_chunk(chunk: &str, pattern: &[char], prefixes: &[usize], matched: &mut usize) -> bool {
    for character in chunk.chars().flat_map(char::to_lowercase) {
        while *matched > 0 && character != pattern[*matched] {
            *matched = prefixes[*matched - 1];
        }
        if character == pattern[*matched] {
            *matched += 1;
        }
        if *matched == pattern.len() {
            return true;
        }
    }
    false
}

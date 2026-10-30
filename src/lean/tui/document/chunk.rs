use super::CHUNK;
use crate::Result;
use std::io::Read;

/// Reads a bounded transfer chunk and extends it through the next UTF-8 boundary.
pub(super) fn read_utf8_chunk(reader: &mut impl Read, remaining: &mut u64) -> Result<String> {
    let mut bytes = vec![0; (*remaining).min(CHUNK as u64) as usize];
    reader.read_exact(&mut bytes)?;
    *remaining -= bytes.len() as u64;
    while std::str::from_utf8(&bytes).is_err() && *remaining > 0 {
        let mut byte = [0];
        reader.read_exact(&mut byte)?;
        bytes.push(byte[0]);
        *remaining -= 1;
    }
    std::str::from_utf8(&bytes)
        .map(str::to_owned)
        .map_err(|error| crate::Error::Invalid(error.to_string()))
}

//! Read and write bounded JSON frames, including ordered large-message chunks.

use super::decode;
use crate::{Result, require};
use serde_json::Value;
use std::io::{BufRead, Read, Write};

pub const MAX_FRAME: usize = 4 * 1024 * 1024;

fn read_frame(reader: &mut impl BufRead) -> Result<Option<Value>> {
    let mut bytes = Vec::new();
    let count = reader
        .take((MAX_FRAME + 1) as u64)
        .read_until(b'\n', &mut bytes)?;
    if count == 0 {
        return Ok(None);
    }
    require(count <= MAX_FRAME, "JSON frame exceeds transport limit")?;
    Ok(Some(decode(&bytes)?))
}

/// Small messages use one JSON line; larger messages use ordered bounded frames.
pub fn write(writer: &mut impl Write, value: &impl serde::Serialize) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() < MAX_FRAME {
        writer.write_all(&bytes)?;
        writer.write_all(b"\n")?;
        return Ok(());
    }
    write_chunks(writer, &bytes)
}

fn write_chunks(writer: &mut impl Write, bytes: &[u8]) -> Result<()> {
    let text = std::str::from_utf8(bytes).expect("JSON is UTF-8");
    let mut offset = 0;
    let mut index = 0u64;
    while offset < text.len() {
        let end = chunk_end(text, offset);
        let frame = serde_json::json!({"antislop_chunk": {
            "index": index, "text": &text[offset..end], "done": end == text.len()
        }});
        serde_json::to_writer(&mut *writer, &frame)?;
        writer.write_all(b"\n")?;
        offset = end;
        index += 1;
    }
    Ok(())
}

fn chunk_end(text: &str, offset: usize) -> usize {
    let mut end = (offset + MAX_FRAME / 8).min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    end
}

pub fn read<T: serde::de::DeserializeOwned>(reader: &mut impl BufRead) -> Result<Option<T>> {
    let Some(first) = read_frame(reader)? else {
        return Ok(None);
    };
    if first.get("antislop_chunk").is_none() {
        return Ok(Some(serde_json::from_value(first)?));
    }
    read_chunks(reader, first).map(Some)
}

fn read_chunks<T: serde::de::DeserializeOwned>(
    reader: &mut impl BufRead,
    mut frame: Value,
) -> Result<T> {
    let mut text = String::new();
    let mut index = 0u64;
    loop {
        let chunk = &frame["antislop_chunk"];
        validate_chunk(&frame, chunk, index)?;
        text.push_str(chunk_text(chunk)?);
        if chunk_done(chunk)? {
            return decode(text.as_bytes());
        }
        frame = read_frame(reader)?
            .ok_or_else(|| crate::Error::Invalid("incomplete chunked message".into()))?;
        index += 1;
    }
}

fn validate_chunk(frame: &Value, chunk: &Value, index: u64) -> Result<()> {
    require(
        frame.as_object().is_some_and(|object| object.len() == 1)
            && chunk["index"].as_u64() == Some(index),
        "invalid chunk sequence",
    )
}

fn chunk_text(chunk: &Value) -> Result<&str> {
    chunk["text"]
        .as_str()
        .ok_or_else(|| crate::Error::Invalid("missing chunk text".into()))
}

fn chunk_done(chunk: &Value) -> Result<bool> {
    chunk["done"]
        .as_bool()
        .ok_or_else(|| crate::Error::Invalid("missing chunk completion flag".into()))
}

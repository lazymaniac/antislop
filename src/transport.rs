//! Bounded JSON frames. Reject duplicate keys before any intermediate Value
//! can discard them, including keys inside internally tagged enum payloads.
use crate::{Result, require};
use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use std::fmt;
use std::io::{BufRead, Read, Write};

pub const MAX_FRAME: usize = 4 * 1024 * 1024;

struct Unique(Value);
impl<'de> Deserialize<'de> for Unique {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct Json;
        impl<'de> Visitor<'de> for Json {
            type Value = Unique;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("JSON with unique object keys")
            }
            fn visit_bool<E: serde::de::Error>(self, v: bool) -> std::result::Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> std::result::Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> std::result::Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> std::result::Result<Unique, E> {
                Ok(Unique(
                    serde_json::Number::from_f64(v)
                        .ok_or_else(|| E::custom("invalid number"))?
                        .into(),
                ))
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> std::result::Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Unique, E> {
                Ok(Unique(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Unique, A::Error> {
                let mut values = Vec::new();
                while let Some(Unique(value)) = seq.next_element()? {
                    values.push(value);
                }
                Ok(Unique(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<Unique, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(serde::de::Error::custom("duplicate JSON object key"));
                    }
                    let Unique(value) = map.next_value()?;
                    values.insert(key, value);
                }
                Ok(Unique(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(Json)
    }
}

pub fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    let Unique(value) = serde_json::from_slice(bytes)?;
    Ok(serde_json::from_value(value)?)
}

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

/// Legacy small messages remain a single JSON line. Larger messages are sent
/// as ordered bounded frames; MAX_FRAME limits a frame, never the whole text.
pub fn write(writer: &mut impl Write, value: &impl serde::Serialize) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() < MAX_FRAME {
        writer.write_all(&bytes)?;
        writer.write_all(b"\n")?;
        return Ok(());
    }
    let text = std::str::from_utf8(&bytes).expect("JSON is UTF-8");
    let mut offset = 0;
    let mut index = 0u64;
    while offset < text.len() {
        let mut end = (offset + MAX_FRAME / 8).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
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

pub fn read<T: serde::de::DeserializeOwned>(reader: &mut impl BufRead) -> Result<Option<T>> {
    let Some(first) = read_frame(reader)? else {
        return Ok(None);
    };
    if first.get("antislop_chunk").is_none() {
        return Ok(Some(serde_json::from_value(first)?));
    }
    let mut frame = first;
    let mut text = String::new();
    let mut index = 0u64;
    loop {
        let chunk = &frame["antislop_chunk"];
        require(
            frame.as_object().is_some_and(|o| o.len() == 1)
                && chunk["index"].as_u64() == Some(index),
            "invalid chunk sequence",
        )?;
        text.push_str(
            chunk["text"]
                .as_str()
                .ok_or_else(|| crate::Error::Invalid("missing chunk text".into()))?,
        );
        let done = chunk["done"]
            .as_bool()
            .ok_or_else(|| crate::Error::Invalid("missing chunk completion flag".into()))?;
        if done {
            return Ok(Some(decode(text.as_bytes())?));
        }
        frame = read_frame(reader)?
            .ok_or_else(|| crate::Error::Invalid("incomplete chunked message".into()))?;
        index += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunked_messages_preserve_large_unicode_and_escaped_text() {
        let original = serde_json::json!({"text": "👩‍💻\n\"\\".repeat(MAX_FRAME / 6)});
        let mut wire = Vec::new();
        write(&mut wire, &original).unwrap();
        assert!(wire.len() > MAX_FRAME);
        assert!(
            wire.split_inclusive(|byte| *byte == b'\n')
                .all(|line| line.len() <= MAX_FRAME)
        );
        assert_eq!(
            read::<Value>(&mut std::io::Cursor::new(wire)).unwrap(),
            Some(original)
        );
    }

    #[test]
    fn incomplete_and_reordered_chunks_fail() {
        for wire in [
            r#"{"antislop_chunk":{"index":1,"text":"{}","done":true}}"#,
            r#"{"antislop_chunk":{"index":0,"text":"{","done":false}}"#,
        ] {
            assert!(read::<Value>(&mut std::io::Cursor::new(wire)).is_err());
        }
    }
}

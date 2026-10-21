//! Immutable session text outside SQLite. References are opaque UUIDs; callers
//! authorize access through the event that owns the reference.
use crate::{Result, require};
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub const CHUNK_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reference {
    pub id: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chunk {
    pub text: String,
    pub next_offset: u64,
    pub done: bool,
}

pub struct Content {
    directory: PathBuf,
}

impl Content {
    pub fn new(database: &Path) -> Self {
        Self {
            directory: database
                .parent()
                .unwrap_or(Path::new("."))
                .join("session-content"),
        }
    }

    fn path(&self, reference: &Reference) -> Result<PathBuf> {
        require(
            uuid::Uuid::parse_str(&reference.id).is_ok(),
            "invalid content reference",
        )?;
        Ok(self.directory.join(&reference.id))
    }

    /// A tagged reference in legacy text columns; untagged historical rows
    /// remain readable. New user text is always externalized, including text
    /// that happens to resemble this internal encoding.
    pub fn encode(&self, text: &str) -> Result<String> {
        Ok(format!(
            "\0antislop-content:{}",
            serde_json::to_string(&self.write(text)?)?
        ))
    }

    pub fn decode(&self, text: &str) -> Result<String> {
        match text.strip_prefix("\0antislop-content:") {
            Some(reference) => self.read(&serde_json::from_str(reference)?),
            None => Ok(text.to_owned()),
        }
    }

    pub fn sql_text(&self, row: &rusqlite::Row<'_>, column: usize) -> rusqlite::Result<String> {
        let text: String = row.get(column)?;
        self.decode(&text).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                column,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })
    }

    pub fn write(&self, text: &str) -> Result<Reference> {
        use std::os::unix::fs::OpenOptionsExt;
        super::daemon::private_directory(&self.directory)?;
        let reference = Reference {
            id: uuid::Uuid::new_v4().to_string(),
            bytes: text.len() as u64,
        };
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(self.path(&reference)?)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        Ok(reference)
    }

    pub fn chunk(&self, reference: &Reference, offset: u64) -> Result<Chunk> {
        require(offset <= reference.bytes, "content offset is out of range")?;
        let mut file = std::fs::File::open(self.path(reference)?)?;
        require(
            file.metadata()?.len() == reference.bytes,
            "session content size changed",
        )?;
        file.seek(SeekFrom::Start(offset))?;
        let mut bytes = Vec::with_capacity(CHUNK_BYTES);
        file.take(CHUNK_BYTES as u64).read_to_end(&mut bytes)?;
        // UTF-8 may end in the middle of a codepoint, but must start on a boundary.
        let valid = match std::str::from_utf8(&bytes) {
            Ok(_) => bytes.len(),
            Err(error) if error.error_len().is_none() => error.valid_up_to(),
            Err(_) => {
                return Err(crate::Error::Invalid(
                    "content offset is not a UTF-8 boundary".into(),
                ));
            }
        };
        bytes.truncate(valid);
        let next_offset = offset + valid as u64;
        Ok(Chunk {
            text: String::from_utf8(bytes).expect("validated UTF-8"),
            next_offset,
            done: next_offset == reference.bytes,
        })
    }

    /// Compatibility path for existing consumers. New history readers use chunks.
    pub fn read(&self, reference: &Reference) -> Result<String> {
        let mut result = String::new();
        let mut offset = 0;
        loop {
            let chunk = self.chunk(reference, offset)?;
            result.push_str(&chunk.text);
            if chunk.done {
                return Ok(result);
            }
            offset = chunk.next_offset;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_text_survives_chunk_boundaries_and_reopening() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("core.db");
        let content = Content::new(&database);
        let text = format!(
            "{}{}",
            "a".repeat(CHUNK_BYTES - 1),
            "👩‍💻e\u{301}".repeat(500_000)
        );
        let reference = content.write(&text).unwrap();
        let mut offset = 0;
        let mut restored = String::new();
        loop {
            let chunk = content.chunk(&reference, offset).unwrap();
            assert!(chunk.text.len() <= CHUNK_BYTES);
            restored.push_str(&chunk.text);
            if chunk.done {
                break;
            }
            assert!(chunk.next_offset > offset);
            offset = chunk.next_offset;
        }
        assert_eq!(restored, text);
        assert_eq!(Content::new(&database).read(&reference).unwrap(), text);
        assert!(content.chunk(&reference, CHUNK_BYTES as u64).is_err());
        assert!(content.chunk(&reference, reference.bytes + 1).is_err());
        assert!(
            content
                .chunk(
                    &Reference {
                        id: "../core.db".into(),
                        bytes: 0
                    },
                    0
                )
                .is_err()
        );
    }
}

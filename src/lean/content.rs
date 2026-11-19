//! Immutable session text outside SQLite. References are opaque UUIDs; callers
//! authorize access through the event that owns the reference.
use crate::{Result, require};
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

mod staging;
pub(in crate::lean) use staging::PreparedContent;
pub(in crate::lean) use staging::PublishedContent;

pub const CHUNK_BYTES: usize = 64 * 1024;
pub(in crate::lean) const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(in crate::lean) struct ImageReference {
    pub id: String,
    pub mime_type: String,
    pub bytes: u64,
    pub name: String,
}

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

#[cfg(test)]
type DirectorySyncHook = std::sync::Arc<dyn Fn() -> std::io::Result<()> + Send + Sync>;

pub(in crate::lean) fn image_type(bytes: &[u8]) -> Result<&'static str> {
    let format = image::guess_format(bytes).map_err(|error| {
        crate::Error::Invalid(format!(
            "unsupported or invalid image attachment; use PNG, JPEG, GIF, TIFF, or WebP ({error})"
        ))
    })?;
    let mime = match format {
        image::ImageFormat::Png => "image/png",
        image::ImageFormat::Jpeg => "image/jpeg",
        image::ImageFormat::Gif => "image/gif",
        image::ImageFormat::WebP => "image/webp",
        image::ImageFormat::Tiff => "image/tiff",
        _ => {
            return Err(crate::Error::Invalid(
                "unsupported image attachment; use PNG, JPEG, GIF, TIFF, or WebP".into(),
            ));
        }
    };
    let reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let (width, height) = reader
        .into_dimensions()
        .map_err(|error| crate::Error::Invalid(format!("invalid image attachment: {error}")))?;
    require(
        width > 0 && height > 0 && u64::from(width) * u64::from(height) <= 40_000_000,
        "image dimensions exceed the 40 megapixel limit",
    )?;
    image::ImageReader::with_format(std::io::Cursor::new(bytes), format)
        .decode()
        .map_err(|error| crate::Error::Invalid(format!("invalid image attachment: {error}")))?;
    Ok(mime)
}

#[cfg(test)]
pub(in crate::lean) fn valid_png_fixture() -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(1, 1, image::Rgba([10, 20, 30, 255]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}

pub struct Content {
    directory: PathBuf,
    image_directory: PathBuf,
    #[cfg(test)]
    directory_sync_hook: std::sync::Mutex<Option<DirectorySyncHook>>,
}

impl Content {
    pub(in crate::lean) fn store_image(&self, name: &str, bytes: &[u8]) -> Result<ImageReference> {
        let original_type = image_type(bytes)?;
        require(
            bytes.len() <= MAX_IMAGE_BYTES,
            "image attachment exceeds the 20 MiB limit",
        )?;
        let normalized;
        let bytes = if original_type == "image/tiff" {
            let decoded = image::load_from_memory(bytes).map_err(|error| {
                crate::Error::Invalid(format!("invalid TIFF image attachment: {error}"))
            })?;
            let mut cursor = std::io::Cursor::new(Vec::new());
            decoded
                .write_to(&mut cursor, image::ImageFormat::Png)
                .map_err(|error| {
                    crate::Error::Invalid(format!(
                        "cannot normalize TIFF image attachment: {error}"
                    ))
                })?;
            normalized = cursor.into_inner();
            normalized.as_slice()
        } else {
            bytes
        };
        require(
            bytes.len() <= MAX_IMAGE_BYTES,
            "normalized image attachment exceeds the 20 MiB limit",
        )?;
        let mime_type = image_type(bytes)?;
        use std::os::unix::fs::OpenOptionsExt;
        super::daemon::private_directory(&self.image_directory)?;
        let id = uuid::Uuid::new_v4().to_string();
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(self.image_directory.join(&id))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::File::open(&self.image_directory)?.sync_all()?;
        Ok(ImageReference {
            id,
            mime_type: mime_type.into(),
            bytes: bytes.len() as u64,
            name: name.chars().take(240).collect(),
        })
    }

    pub(in crate::lean) fn read_image(&self, reference: &ImageReference) -> Result<Vec<u8>> {
        require(
            uuid::Uuid::parse_str(&reference.id).is_ok(),
            "invalid image reference",
        )?;
        require(
            reference.bytes <= MAX_IMAGE_BYTES as u64,
            "image attachment exceeds the 20 MiB limit",
        )?;
        let bytes = std::fs::read(self.image_directory.join(&reference.id))?;
        require(
            bytes.len() as u64 == reference.bytes,
            "image attachment size changed",
        )?;
        require(
            image_type(&bytes)? == reference.mime_type,
            "image attachment type changed",
        )?;
        Ok(bytes)
    }

    pub fn new(database: &Path) -> Self {
        Self {
            directory: database
                .parent()
                .unwrap_or(Path::new("."))
                .join("session-content"),
            image_directory: database
                .parent()
                .unwrap_or(Path::new("."))
                .join("image-content"),
            #[cfg(test)]
            directory_sync_hook: std::sync::Mutex::new(None),
        }
    }

    pub(in crate::lean) fn sync_published_entries(&self) -> Result<()> {
        #[cfg(test)]
        let hook = self
            .directory_sync_hook
            .lock()
            .expect("directory sync hook mutex")
            .clone();
        #[cfg(test)]
        if let Some(hook) = hook {
            hook()?;
        }
        std::fs::File::open(&self.directory)?.sync_all()?;
        Ok(())
    }

    #[cfg(test)]
    pub(in crate::lean) fn set_directory_sync_hook(
        &self,
        hook: impl Fn() -> std::io::Result<()> + Send + Sync + 'static,
    ) {
        *self
            .directory_sync_hook
            .lock()
            .expect("directory sync hook mutex") = Some(std::sync::Arc::new(hook));
    }

    pub(in crate::lean) fn path(&self, reference: &Reference) -> Result<PathBuf> {
        require(
            uuid::Uuid::parse_str(&reference.id).is_ok(),
            "invalid content reference",
        )?;
        Ok(self.directory.join(&reference.id))
    }

    /// Externalize user text behind an opaque reference. Short runtime messages
    /// may be stored inline; user text is never interpreted as an encoding.
    pub fn encode(&self, text: &str) -> Result<String> {
        Ok(format!(
            "\0antislop-content:{}",
            serde_json::to_string(&self.write(text)?)?
        ))
    }

    pub fn reference(encoded: &str) -> Result<Reference> {
        let reference = encoded
            .strip_prefix("\0antislop-content:")
            .ok_or_else(|| crate::Error::Invalid("stored content reference is missing".into()))?;
        Ok(serde_json::from_str(reference)?)
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

    /// Read a complete recorded value; paginated history readers use `chunk`.
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

    /// Removes unreferenced content unless an active publisher holds its file lease.
    pub fn remove_unreferenced(&self, db: &mut rusqlite::Connection) -> Result<usize> {
        if !self.directory.exists() {
            return Ok(0);
        }
        // Prevent a stale reference scan from deleting content just committed
        // by another Store after the publisher released its per-file lease.
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut referenced = std::collections::HashSet::new();
        let tables = {
            let mut statement = tx.prepare(
                "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
            )?;
            statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        for table in tables {
            let columns = {
                let mut statement = tx.prepare(&format!(
                    "PRAGMA table_info(\"{}\")",
                    table.replace('"', "\"\"")
                ))?;
                statement
                    .query_map([], |row| row.get::<_, String>(1))?
                    .collect::<std::result::Result<Vec<_>, _>>()?
            };
            for column in columns {
                let sql = format!(
                    "SELECT CAST(\"{}\" AS TEXT) FROM \"{}\" WHERE \"{}\" IS NOT NULL",
                    column.replace('"', "\"\""),
                    table.replace('"', "\"\""),
                    column.replace('"', "\"\"")
                );
                let mut statement = tx.prepare(&sql)?;
                let values = statement.query_map([], |row| row.get::<_, Option<String>>(0))?;
                for value in values {
                    let Some(value) = value? else {
                        continue;
                    };
                    for part in value
                        .split(|character: char| !character.is_ascii_hexdigit() && character != '-')
                    {
                        if part.len() == 36 && uuid::Uuid::parse_str(part).is_ok() {
                            referenced.insert(part.to_owned());
                        }
                    }
                }
            }
        }
        let mut removed = 0;
        for entry in std::fs::read_dir(&self.directory)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if uuid::Uuid::parse_str(&name).is_ok() && !referenced.contains(&name) {
                use std::os::unix::fs::OpenOptionsExt;
                let file = std::fs::OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NOFOLLOW)
                    .open(entry.path())?;
                match staging::try_exclusive_lease(&file) {
                    Ok(()) => {
                        std::fs::remove_file(entry.path())?;
                        removed += 1;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }
        tx.commit()?;
        Ok(removed)
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

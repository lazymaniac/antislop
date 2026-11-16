use super::*;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;

/// A validated file stays leased outside the directory ordinary GC scans.
pub(in crate::lean) struct PreparedContent {
    file: tempfile::NamedTempFile,
    pub reference: Reference,
}

fn lock(file: &std::fs::File, operation: i32) -> std::io::Result<()> {
    // SAFETY: the descriptor stays owned for the entire lease.
    if unsafe { libc::flock(file.as_raw_fd(), operation) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

impl Content {
    fn staging(&self) -> PathBuf {
        self.directory.with_file_name("session-content-staging")
    }

    pub(in crate::lean) fn prepare_text(
        &self,
        mut reader: impl Read,
    ) -> Result<Option<PreparedContent>> {
        let staging = self.staging();
        super::super::daemon::private_directory(&staging)?;
        let directory = std::fs::File::open(&staging)?;
        lock(&directory, libc::LOCK_EX)?;
        let file = tempfile::NamedTempFile::new_in(&staging)?;
        lock(file.as_file(), libc::LOCK_EX)?;
        drop(directory);
        let mut prepared = PreparedContent {
            file,
            reference: Reference {
                id: uuid::Uuid::new_v4().to_string(),
                bytes: 0,
            },
        };
        let mut buffer = [0; CHUNK_BYTES];
        let mut pending = Vec::with_capacity(CHUNK_BYTES + 4);
        loop {
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                if !pending.is_empty() {
                    return Ok(None);
                }
                prepared.file.as_file().sync_all()?;
                return Ok(Some(prepared));
            }
            if buffer[..read].contains(&0) {
                return Ok(None);
            }
            pending.extend_from_slice(&buffer[..read]);
            let valid = match std::str::from_utf8(&pending) {
                Ok(_) => pending.len(),
                Err(error) if error.error_len().is_none() => error.valid_up_to(),
                Err(_) => return Ok(None),
            };
            prepared.file.write_all(&pending[..valid])?;
            prepared.reference.bytes += valid as u64;
            pending.drain(..valid);
        }
    }

    pub(in crate::lean) fn publish(&self, prepared: PreparedContent) -> Result<Reference> {
        super::super::daemon::private_directory(&self.directory)?;
        let directory = std::fs::File::open(self.staging())?;
        lock(&directory, libc::LOCK_EX)?;
        let destination = self.path(&prepared.reference)?;
        prepared
            .file
            .persist_noclobber(destination)
            .map_err(|e| e.error)?;
        std::fs::File::open(&self.directory)?.sync_all()?;
        Ok(prepared.reference)
    }

    pub(in crate::lean) fn cleanup_staging(&self) -> Result<usize> {
        let path = self.staging();
        if !path.try_exists()? {
            return Ok(0);
        }
        let directory = std::fs::File::open(&path)?;
        lock(&directory, libc::LOCK_EX)?;
        let mut removed = 0;
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let file = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(entry.path())?;
            match lock(&file, libc::LOCK_EX | libc::LOCK_NB) {
                Ok(()) => {
                    std::fs::remove_file(entry.path())?;
                    removed += 1;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FailingReader;

    impl Read for FailingReader {
        fn read(&mut self, _buffer: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("injected source read failure"))
        }
    }

    #[test]
    fn source_io_errors_are_returned_and_staging_files_are_removed() {
        let dir = tempfile::tempdir().unwrap();
        let content = Content::new(&dir.path().join("core.db"));
        let error = content
            .prepare_text(FailingReader)
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("injected source read failure"));
        assert_eq!(std::fs::read_dir(content.staging()).unwrap().count(), 0);
    }

    #[test]
    fn streamed_validation_keeps_utf8_boundaries_rejects_binary_and_leases_active_staging() {
        let dir = tempfile::tempdir().unwrap();
        let content = Content::new(&dir.path().join("core.db"));
        let text = format!(
            "{}{}",
            "a".repeat(CHUNK_BYTES - 1),
            "👩‍💻e\u{301}".repeat(200_000)
        );
        let prepared = content.prepare_text(text.as_bytes()).unwrap().unwrap();
        assert_eq!(prepared.reference.bytes, text.len() as u64);
        assert_eq!(content.cleanup_staging().unwrap(), 0);
        let abandoned = content.staging().join("abandoned");
        std::fs::write(&abandoned, "partial").unwrap();
        assert_eq!(content.cleanup_staging().unwrap(), 1);
        let reference = content.publish(prepared).unwrap();
        assert_eq!(content.read(&reference).unwrap(), text);
        for suffix in [b"\0".as_slice(), b"\xff".as_slice(), b"\xf0\x9f".as_slice()] {
            let input = std::io::Cursor::new(vec![b'a'; CHUNK_BYTES - 1]).chain(suffix);
            assert!(content.prepare_text(input).unwrap().is_none());
        }
        assert_eq!(std::fs::read_dir(content.staging()).unwrap().count(), 0);
    }
}

#[cfg(test)]
mod scaling_tests {
    use super::*;

    #[test]
    fn large_document_preparation_uses_a_bounded_reader_and_disk_storage() {
        let dir = tempfile::tempdir().unwrap();
        let content = Content::new(&dir.path().join("core.db"));
        let size = 128 * 1024 * 1024;
        let prepared = content
            .prepare_text(std::io::repeat(b'x').take(size))
            .unwrap()
            .unwrap();
        assert_eq!(prepared.reference.bytes, size);
        let reference = content.publish(prepared).unwrap();
        let chunk = content
            .chunk(&reference, size - CHUNK_BYTES as u64)
            .unwrap();
        assert!(chunk.done);
        assert_eq!(chunk.text.len(), CHUNK_BYTES);
    }
}

use super::*;

/// Metadata for one immutable, landed text output of a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResultDocument {
    pub id: String,
    pub producer: String,
    pub path: String,
    pub source_commit: String,
    pub created_ms: i64,
}

impl Store {
    /// Retain a textual declared output after it has successfully landed.
    #[cfg(test)]
    pub(in crate::lean) fn record_result_document(
        &self,
        conversation: &str,
        producer: &str,
        path: &str,
        source_commit: &str,
        text: &str,
    ) -> Result<()> {
        let Some(prepared) = self.content.prepare_text(text.as_bytes())? else {
            return Ok(());
        };
        self.record_spooled_document(conversation, producer, path, source_commit, prepared)
    }

    pub(in crate::lean) fn spool_text_content(
        &self,
        reader: impl std::io::Read,
    ) -> Result<Option<crate::lean::content::PreparedContent>> {
        self.content.prepare_text(reader)
    }

    pub(in crate::lean) fn record_spooled_document(
        &self,
        conversation: &str,
        producer: &str,
        path: &str,
        source_commit: &str,
        prepared: crate::lean::content::PreparedContent,
    ) -> Result<()> {
        let guard = self.content_write.lock().expect("content write mutex");
        let mut db = self.db();
        let tx = db.transaction()?;
        validate_document(&tx, conversation, producer, path)?;
        let exists: bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM session_documents WHERE producer_task=?1 AND path=?2 AND source_commit=?3)",params![producer,path,source_commit],|r|r.get(0))?;
        if exists {
            return Ok(());
        }

        // Rename under the staging lock, then release both broad locks before
        // fsync. The inode lease prevents GC from unlinking the published file.
        let published = self.content.publish(prepared)?;
        drop(tx);
        drop(db);
        drop(guard);
        if let Err(sync_error) = self.content.sync_published_entries() {
            return self.collect_orphan(published, sync_error);
        }

        // Deletion may have won during fsync. Recheck both authority and
        // idempotency after reacquiring the lock before making a durable ref.
        let guard = self.content_write.lock().expect("content write mutex");
        let mut db = self.db();
        let inserted = match commit_published_document(
            &mut db,
            conversation,
            producer,
            path,
            source_commit,
            &published.reference,
        ) {
            Ok(inserted) => inserted,
            Err(error) => return self.discard_after_error(published, &mut db, error),
        };
        if !inserted {
            drop(published);
            self.content.remove_unreferenced(&mut db)?;
            drop(db);
            drop(guard);
            return Ok(());
        }
        drop(published);
        drop(guard);
        Ok(())
    }

    fn collect_orphan(
        &self,
        published: crate::lean::content::PublishedContent,
        error: crate::Error,
    ) -> Result<()> {
        let _guard = self.content_write.lock().expect("content write mutex");
        let mut db = self.db();
        self.discard_after_error(published, &mut db, error)
    }

    fn discard_after_error(
        &self,
        published: crate::lean::content::PublishedContent,
        db: &mut rusqlite::Connection,
        error: crate::Error,
    ) -> Result<()> {
        drop(published);
        match self.content.remove_unreferenced(db) {
            Ok(_) => Err(error),
            Err(cleanup_error) => Err(crate::Error::Invalid(format!(
                "{error}; unreferenced content cleanup also failed: {cleanup_error}"
            ))),
        }
    }

    pub fn result_documents(&self, conversation: &str) -> Result<Vec<ResultDocument>> {
        let db = self.db();
        let mut query = db.prepare(
            "SELECT id,producer_task,path,source_commit,created_ms FROM session_documents WHERE conversation_id=?1 ORDER BY rowid",
        )?;
        Ok(query
            .query_map([conversation], |row| {
                Ok(ResultDocument {
                    id: row.get(0)?,
                    producer: row.get(1)?,
                    path: row.get(2)?,
                    source_commit: row.get(3)?,
                    created_ms: row.get(4)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn result_document_chunk(
        &self,
        conversation: &str,
        id: &str,
        offset: u64,
    ) -> Result<crate::lean::content::Chunk> {
        let encoded: String = self.db().query_row(
            "SELECT content_ref FROM session_documents WHERE conversation_id=?1 AND id=?2",
            params![conversation, id],
            |row| row.get(0),
        )?;
        let reference = serde_json::from_str(&encoded)?;
        self.content.chunk(&reference, offset)
    }
}

fn commit_published_document(
    db: &mut rusqlite::Connection,
    conversation: &str,
    producer: &str,
    path: &str,
    source_commit: &str,
    reference: &crate::lean::content::Reference,
) -> Result<bool> {
    let tx = db.transaction()?;
    validate_document(&tx, conversation, producer, path)?;
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM session_documents WHERE producer_task=?1 AND path=?2 AND source_commit=?3)",
        params![producer, path, source_commit],
        |row| row.get(0),
    )?;
    if exists {
        return Ok(false);
    }
    let encoded = serde_json::to_string(reference)?;
    tx.execute(
        "INSERT INTO session_documents(id,conversation_id,producer_task,path,source_commit,content_ref,created_ms) VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![crate::id(), conversation, producer, path, source_commit, encoded, now_ms()],
    )?;
    tx.commit()?;
    Ok(true)
}

fn validate_document(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    producer: &str,
    path: &str,
) -> Result<()> {
    crate::require(
        tx.query_row("SELECT EXISTS(SELECT 1 FROM tasks t, json_each(t.spec,'$.outputs') o WHERE t.id=?1 AND t.conversation_id=?2 AND o.value=?3)",params![producer,conversation,path],|row|row.get::<_,bool>(0))?,
        "result document must be a declared output of a task in this conversation",
    )
}

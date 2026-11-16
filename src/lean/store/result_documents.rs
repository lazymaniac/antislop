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
        self.publish_result_document(conversation, producer, path, source_commit, prepared)
    }

    pub(in crate::lean) fn prepare_result_document(
        &self,
        reader: impl std::io::Read,
    ) -> Result<Option<crate::lean::content::PreparedContent>> {
        self.content.prepare_text(reader)
    }

    pub(in crate::lean) fn publish_result_document(
        &self,
        conversation: &str,
        producer: &str,
        path: &str,
        source_commit: &str,
        prepared: crate::lean::content::PreparedContent,
    ) -> Result<()> {
        let _guard = self.content_write.lock().expect("content write mutex");
        let mut db = self.db();
        let tx = db.transaction()?;
        crate::require(tx.query_row("SELECT EXISTS(SELECT 1 FROM tasks t, json_each(t.spec,'$.outputs') o WHERE t.id=?1 AND t.conversation_id=?2 AND o.value=?3)",params![producer,conversation,path],|r|r.get::<_,bool>(0))?,
            "result document must be a declared output of a task in this conversation")?;
        let exists: bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM session_documents WHERE producer_task=?1 AND path=?2 AND source_commit=?3)",params![producer,path,source_commit],|r|r.get(0))?;
        if exists {
            return Ok(());
        }
        // A crash or SQL failure after this rename leaves only an orphan that
        // normal GC may collect. Deletion cannot race the reference commit.
        let reference = self.content.publish(prepared)?;
        tx.execute("INSERT INTO session_documents(id,conversation_id,producer_task,path,source_commit,content_ref,created_ms) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![crate::id(),conversation,producer,path,source_commit,serde_json::to_string(&reference)?,now_ms()])?;
        tx.commit()?;
        Ok(())
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

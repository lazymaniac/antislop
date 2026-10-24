use super::*;

impl Store {
    pub fn history_content(
        &self,
        conversation: &str,
        owner: &ContentOwner,
        offset: u64,
    ) -> Result<crate::lean::content::Chunk> {
        match self.history_reference(conversation, owner)? {
            HistoryReference::Stored(reference) => self.content.chunk(&reference, offset),
            HistoryReference::Legacy(text) => legacy_chunk(&text, offset),
        }
    }

    fn history_reference(
        &self,
        conversation: &str,
        owner: &ContentOwner,
    ) -> Result<HistoryReference> {
        match owner {
            ContentOwner::Mail { id } => self.mail_reference(conversation, *id),
            ContentOwner::Round { task, round, field } => {
                self.round_reference(conversation, task, *round, field)
            }
        }
    }

    fn mail_reference(&self, conversation: &str, id: u64) -> Result<HistoryReference> {
        let encoded: String = self.db().query_row(
            "SELECT text FROM mail WHERE id=?1 AND conversation_id=?2",
            params![id as i64, conversation],
            |r| r.get(0),
        )?;
        parse_reference_or_legacy(&encoded)
    }

    fn round_reference(
        &self,
        conversation: &str,
        task: &str,
        round: u32,
        field: &str,
    ) -> Result<HistoryReference> {
        crate::require(
            matches!(field, "report" | "verdict"),
            "invalid round content field",
        )?;
        let encoded: String = self.db().query_row(
            "SELECT r.content FROM task_rounds r JOIN tasks t ON t.id=r.task_id WHERE t.conversation_id=?1 AND t.id=?2 AND r.round=?3",
            params![conversation, task, round],
            |r| r.get(0),
        )?;
        let record = self.round_record(&encoded)?;
        if let Some(reference) = record[field].get("session_content") {
            Ok(HistoryReference::Stored(serde_json::from_value(
                reference.clone(),
            )?))
        } else {
            let text = record[field]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| record[field].to_string());
            Ok(HistoryReference::Legacy(text))
        }
    }

    fn round_record(&self, encoded: &str) -> Result<Value> {
        let reference = serde_json::from_str(encoded)?;
        let text = self.content.read(&reference)?;
        Ok(serde_json::from_str(&text)?)
    }
}

enum HistoryReference {
    Stored(crate::lean::content::Reference),
    Legacy(String),
}

fn parse_reference_or_legacy(encoded: &str) -> Result<HistoryReference> {
    match encoded.strip_prefix("\0antislop-content:") {
        Some(reference) => Ok(HistoryReference::Stored(serde_json::from_str(reference)?)),
        None => Ok(HistoryReference::Legacy(encoded.to_owned())),
    }
}

fn legacy_chunk(text: &str, offset: u64) -> Result<crate::lean::content::Chunk> {
    let start = usize::try_from(offset)
        .map_err(|_| Error::Invalid("content offset is out of range".into()))?;
    crate::require(
        start <= text.len() && text.is_char_boundary(start),
        "invalid content offset",
    )?;
    let mut end = start
        .saturating_add(crate::lean::content::CHUNK_BYTES)
        .min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    Ok(crate::lean::content::Chunk {
        text: text[start..end].into(),
        next_offset: end as u64,
        done: end == text.len(),
    })
}

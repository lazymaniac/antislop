//! Immutable command evidence lives on disk; SQLite holds only scoped references.
use super::*;

pub(in crate::lean) const MAX_EVIDENCE_BYTES: usize = 256 * 1024;
pub(in crate::lean) const MAX_RESULT_EVIDENCE: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::lean) struct EvidenceReference {
    pub id: String,
    pub label: String,
    pub bytes: u64,
    pub sha256: String,
    pub producer_turn: String,
}

impl Store {
    pub(in crate::lean) fn record_evidence(
        &self,
        turn: &str,
        label: &str,
        text: &str,
    ) -> Result<EvidenceReference> {
        crate::require(
            !label.trim().is_empty() && label.len() <= 160 && !label.chars().any(char::is_control),
            "label: use 1–160 bytes without control characters",
        )?;
        crate::require(
            !text.is_empty() && text.len() <= MAX_EVIDENCE_BYTES,
            "text: evidence must contain 1–262144 UTF-8 bytes; split larger outputs into separately labelled records",
        )?;
        let hash = crate::hash(text.as_bytes());
        let mut db = self.db();
        let tx = db.transaction()?;
        let existing = tx.query_row("SELECT id,label,bytes,sha256,turn_id FROM agent_evidence WHERE turn_id=?1 AND label=?2", params![turn,label], evidence_row).optional()?;
        if let Some(existing) = existing {
            crate::require(
                existing.sha256 == hash,
                "label: already records different immutable evidence; use a new label",
            )?;
            return Ok(existing);
        }
        let count: i64 = tx.query_row(
            "SELECT count(*) FROM agent_evidence WHERE turn_id=?1",
            [turn],
            |r| r.get(0),
        )?;
        crate::require(
            count < MAX_RESULT_EVIDENCE as i64,
            "evidence: at most 64 records per turn",
        )?;
        let reference = self.content.write(text)?;
        let evidence = EvidenceReference {
            id: reference.id,
            label: label.into(),
            bytes: reference.bytes,
            sha256: hash,
            producer_turn: turn.into(),
        };
        tx.execute(
            "INSERT INTO agent_evidence(id,turn_id,label,bytes,sha256) VALUES(?1,?2,?3,?4,?5)",
            params![
                evidence.id,
                turn,
                label,
                evidence.bytes as i64,
                evidence.sha256
            ],
        )?;
        tx.commit()?;
        Ok(evidence)
    }

    pub(in crate::lean) fn evidence(
        &self,
        conversation: &str,
        id: &str,
    ) -> Result<EvidenceReference> {
        self.db().query_row("SELECT e.id,e.label,e.bytes,e.sha256,e.turn_id FROM agent_evidence e JOIN turns t ON t.id=e.turn_id WHERE e.id=?1 AND t.conversation_id=?2", params![id,conversation], evidence_row).optional()?.ok_or_else(|| Error::Invalid("evidence: unknown in this conversation".into()))
    }

    pub(in crate::lean) fn evidence_text(&self, evidence: &EvidenceReference) -> Result<String> {
        let text = self.content.read(&super::super::content::Reference {
            id: evidence.id.clone(),
            bytes: evidence.bytes,
        })?;
        crate::require(
            crate::hash(text.as_bytes()) == evidence.sha256,
            "evidence: stored content checksum changed",
        )?;
        Ok(text)
    }

    /// Bounded reference-only context survives report amendments. Records are
    /// historical observations, never an attestation of the current code.
    pub(in crate::lean) fn task_evidence_context(
        &self,
        task: &str,
        generation: i64,
    ) -> Result<Vec<EvidenceReference>> {
        let db = self.db();
        Ok(db.prepare("SELECT e.id,e.label,e.bytes,e.sha256,e.turn_id FROM agent_evidence e
            JOIN turns t ON t.id=e.turn_id JOIN turn_scopes s ON s.turn_id=t.id JOIN turn_inputs i ON i.turn_id=t.id
            WHERE s.task_id=?1 AND i.generation=?2 AND t.role='worker:'||?1 AND t.status='done'
            ORDER BY t.started_ms DESC,e.rowid DESC LIMIT 64")?
            .query_map(params![task,generation], evidence_row)?.collect::<std::result::Result<_,_>>()?)
    }

    pub(in crate::lean) fn result_evidence(&self, turn: &str) -> Result<Vec<EvidenceReference>> {
        let db = self.db();
        Ok(db.prepare("SELECT id,label,bytes,sha256,turn_id FROM agent_evidence WHERE turn_id=?1 OR id IN (SELECT evidence_id FROM result_evidence WHERE turn_id=?1) ORDER BY label,id")?.query_map([turn], evidence_row)?.collect::<std::result::Result<_,_>>()?)
    }
}

fn evidence_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<EvidenceReference> {
    Ok(EvidenceReference {
        id: row.get(0)?,
        label: row.get(1)?,
        bytes: row.get::<_, i64>(2)? as u64,
        sha256: row.get(3)?,
        producer_turn: row.get(4)?,
    })
}

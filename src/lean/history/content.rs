//! History text is fetched only through its recorded immutable content reference.
use super::*;
use crate::lean::content::{Content, Reference};

impl Store {
    pub fn history_content(
        &self,
        conversation: &str,
        owner: &ContentOwner,
        offset: u64,
    ) -> Result<crate::lean::content::Chunk> {
        self.content
            .chunk(&self.history_reference(conversation, owner)?, offset)
    }

    fn history_reference(&self, conversation: &str, owner: &ContentOwner) -> Result<Reference> {
        match owner {
            ContentOwner::Mail { id } => {
                let encoded: String = self.db().query_row(
                    "SELECT text FROM mail WHERE id=?1 AND conversation_id=?2",
                    params![*id as i64, conversation],
                    |r| r.get(0),
                )?;
                Content::reference(&encoded)
            }
            ContentOwner::Round { task, round, field } => {
                self.round_reference(conversation, task, *round, field)
            }
        }
    }

    fn round_reference(
        &self,
        conversation: &str,
        task: &str,
        round: u32,
        field: &str,
    ) -> Result<Reference> {
        crate::require(
            matches!(field, "report" | "verdict"),
            "invalid round content field",
        )?;
        let encoded:String=self.db().query_row("SELECT r.content FROM task_rounds r JOIN tasks t ON t.id=r.task_id WHERE t.conversation_id=?1 AND t.id=?2 AND r.round=?3",params![conversation,task,round],|r|r.get(0))?;
        let reference = serde_json::from_str(&encoded)?;
        let record: Value = serde_json::from_str(&self.content.read(&reference)?)?;
        let reference = record[field].get("session_content").ok_or_else(|| {
            Error::Invalid(format!(
                "round {round} has no recorded {field} content reference"
            ))
        })?;
        Ok(serde_json::from_value(reference.clone())?)
    }
}

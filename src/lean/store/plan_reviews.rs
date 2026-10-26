//! Review progress is scoped to the current user request, including after restart.
use super::*;

impl Store {
    pub(in crate::lean) fn previous_plan_rejection(
        &self,
        conversation: &str,
    ) -> Result<Option<crate::lean::tasks::Verdict>> {
        let payload:Option<String>=self.db().query_row("SELECT payload FROM events WHERE conversation_id=?1 AND kind='plan_review' AND json_extract(payload,'$.approved')=0 AND seq>coalesce((SELECT max(seq) FROM events WHERE conversation_id=?1 AND kind='message' AND json_extract(payload,'$.author')='user'),0) ORDER BY seq DESC LIMIT 1",[conversation],|r|r.get(0)).optional()?;
        payload
            .map(|text| serde_json::from_str(&text).map_err(Into::into))
            .transpose()
    }
}

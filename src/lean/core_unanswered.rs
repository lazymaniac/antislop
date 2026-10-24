use super::*;

impl Core {
    /// The next message boundary for either orchestration session.
    pub(in crate::lean) fn unanswered(&self, conversation: &str) -> Result<Option<u64>> {
        // A failed turn counts as handled (the next message asks again); one
        // cut off by a restart does not.
        let after = match self.store.last_orchestration_turn(conversation)? {
            Some(turn) if turn.status == TurnStatus::Interrupted => {
                self.store.last_orchestration_answered(conversation)?
            }
            Some(turn) => turn.upto_message,
            None => 0,
        };
        let pending: Vec<_> = self
            .store
            .messages_after(conversation, after)?
            .into_iter()
            .filter(|message| message.author != ORCHESTRATOR)
            .collect();
        // Give each newly queued comment its own turn, including messages
        // preceding it. Comments arriving during a turn stay queued.
        Ok(pending
            .iter()
            .find(|message| message.text.starts_with(COMMENT_PREFIX))
            .or_else(|| pending.last())
            .map(|message| message.seq))
    }
}

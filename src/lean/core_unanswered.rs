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

    /// The messages a turn up to `upto` is asked, after what was answered.
    pub(in crate::lean) fn asked_since(
        &self,
        conversation: &str,
        answered: u64,
        upto: u64,
    ) -> Result<Vec<Message>> {
        Ok(self
            .store
            .messages_after(conversation, answered)?
            .into_iter()
            .filter(|message| message.author != ORCHESTRATOR && message.seq <= upto)
            .collect())
    }

    /// Whether the planner owns the next turn: this planning cycle has no plan
    /// yet, or the orchestrator asked for a new one. Comments and everything
    /// else after the first draft belong to the orchestrator.
    pub(in crate::lean) fn planner_owns_turn(&self, conversation: &str, upto: u64) -> Result<bool> {
        let answered = self.store.last_orchestration_answered(conversation)?;
        let asked = self.asked_since(conversation, answered, upto)?;
        if includes_comment(&asked) {
            return Ok(false);
        }
        Ok(self.store.awaiting_first_plan(conversation)? || includes_replan(&asked))
    }
}

/// Whether a message of `asked` is a user comment on the plan.
pub(in crate::lean) fn includes_comment(asked: &[Message]) -> bool {
    asked
        .iter()
        .any(|message| message.text.starts_with(COMMENT_PREFIX))
}

/// Whether a message of `asked` is the orchestrator's request for a new plan.
fn includes_replan(asked: &[Message]) -> bool {
    asked.iter().any(|message| message.author == REPLAN)
}

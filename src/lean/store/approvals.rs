//! Approval dialogs of agent CLIs, asked as questions and answered with the
//! id of one of the dialog's choices.
use super::*;

pub(in crate::lean) const SUPERVISED: &str = "supervised";
pub(in crate::lean) const AUTONOMOUS: &str = "autonomous";

/// A dialog waiting for a decision.
#[derive(Debug, Clone)]
pub(in crate::lean) struct ApprovalRequest<'a> {
    pub turn: &'a str,
    pub role: &'a str,
    pub provider: &'a str,
    pub kind: &'a str,
    pub summary: &'a str,
    pub options: &'a Value,
    pub fingerprint: &'a str,
    pub decider: &'a str,
}

impl Store {
    /// How the approvals of the conversation's agents are decided.
    pub(in crate::lean) fn approval_mode(&self, conversation: &str) -> Result<String> {
        Ok(self
            .db()
            .query_row(
                "SELECT mode FROM approval_modes WHERE conversation_id=?1",
                [conversation],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or_else(|| SUPERVISED.to_owned()))
    }

    pub(in crate::lean) fn set_approval_mode(&self, conversation: &str, mode: &str) -> Result<()> {
        crate::require(
            matches!(mode, SUPERVISED | AUTONOMOUS),
            "mode must be supervised or autonomous",
        )?;
        self.conversation(conversation)?;
        self.db().execute(
            "INSERT INTO approval_modes(conversation_id,mode) VALUES(?1,?2) ON CONFLICT(conversation_id) DO UPDATE SET mode=?2",
            params![conversation, mode],
        )?;
        self.append_event(
            conversation,
            None,
            None,
            "approval_mode_changed",
            &serde_json::json!({"mode":mode}),
        )?;
        Ok(())
    }

    pub(in crate::lean) fn open_approval(
        &self,
        conversation: &str,
        request: &ApprovalRequest<'_>,
    ) -> Result<String> {
        crate::require(
            request.options.as_array().is_some_and(|o| o.len() >= 2),
            "an approval needs at least two choices",
        )?;
        let id = crate::id();
        self.db().execute(
            "INSERT INTO approvals(id,conversation_id,turn_id,role,provider,kind,summary,options,fingerprint,decider,state,created_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'open',?11)",
            params![
                id,
                conversation,
                request.turn,
                request.role,
                request.provider,
                request.kind,
                request.summary,
                request.options.to_string(),
                request.fingerprint,
                request.decider,
                now_ms()
            ],
        )?;
        self.append_event(
            conversation,
            Some(request.turn),
            None,
            "approval_requested",
            &serde_json::json!({
                "request":id,"role":request.role,"provider":request.provider,
                "kind":request.kind,"summary":request.summary,"options":request.options,
                "decider":request.decider,
            }),
        )?;
        Ok(id)
    }

    /// `(state, choice, decider)` of an approval.
    pub(in crate::lean) fn approval_state(
        &self,
        id: &str,
    ) -> Result<(String, Option<String>, String)> {
        self.db()
            .query_row(
                "SELECT state,choice,decider FROM approvals WHERE id=?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?
            .ok_or_else(|| Error::Invalid("request: unknown approval ID".into()))
    }

    /// Records the decision. `by` must own the open approval; a user may
    /// override an orchestrator-owned approval. Same-actor retries of the
    /// same decision are idempotent.
    pub(in crate::lean) fn answer_approval(
        &self,
        conversation: &str,
        id: &str,
        choice: &str,
        by: &str,
    ) -> Result<()> {
        crate::require(
            matches!(by, "user" | "orchestrator"),
            "approval answerer must be user or orchestrator",
        )?;
        let (state, previous, previous_by, options): (String, Option<String>, Option<String>, String) = self
            .db()
            .query_row(
                "SELECT state,choice,answered_by,options FROM approvals WHERE id=?1 AND conversation_id=?2",
                params![id, conversation],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?
            .ok_or_else(|| {
                Error::Invalid("request: unknown approval ID in this conversation".into())
            })?;
        let options: Value = serde_json::from_str(&options)?;
        let known = options
            .as_array()
            .into_iter()
            .flatten()
            .any(|option| option["id"] == choice);
        crate::require(
            known,
            &format!(
                "choice must be one of the approval's choice ids ({})",
                options
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|option| option["id"].as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )?;
        match state.as_str() {
            "answered" => crate::require(
                previous.as_deref() == Some(choice) && previous_by.as_deref() == Some(by),
                "approval already has a different answer or answerer",
            ),
            "closed" => Err(Error::Invalid(
                "approval is closed; the agent no longer waits for it".into(),
            )),
            _ => {
                // Ownership is checked in the mutation predicate itself. A
                // concurrent escalation therefore cannot be bypassed by a
                // stale orchestrator decision. The user retains override.
                let changed = self.db().execute(
                    "UPDATE approvals SET state='answered',choice=?3,answered_by=?4 WHERE id=?1 AND conversation_id=?2 AND state='open' AND (decider=?4 OR ?4='user')",
                    params![id, conversation, choice, by],
                )?;
                crate::require(
                    changed == 1,
                    "approval is owned by another decider or changed while answering",
                )?;
                self.append_event(
                    conversation,
                    None,
                    None,
                    "approval_resolved",
                    &serde_json::json!({"request":id,"state":"answered","choice":choice,"by":by}),
                )?;
                Ok(())
            }
        }
    }

    /// Hands an approval the orchestrator was to decide to the user.
    pub(in crate::lean) fn escalate_approval(&self, conversation: &str, id: &str) -> Result<()> {
        let changed = self.db().execute(
            "UPDATE approvals SET decider='user' WHERE id=?1 AND conversation_id=?2 AND state='open'",
            params![id, conversation],
        )?;
        crate::require(changed == 1, "approval is not open")?;
        self.append_event(
            conversation,
            None,
            None,
            "approval_escalated",
            &serde_json::json!({"request":id}),
        )?;
        Ok(())
    }

    /// Ends an approval nobody answered: the dialog was settled in the
    /// terminal, or the turn ended.
    pub(in crate::lean) fn close_approval(
        &self,
        conversation: &str,
        id: &str,
        reason: &str,
    ) -> Result<()> {
        let changed = self.db().execute(
            "UPDATE approvals SET state='closed' WHERE id=?1 AND state='open'",
            [id],
        )?;
        if changed == 1 {
            self.append_event(
                conversation,
                None,
                None,
                "approval_resolved",
                &serde_json::json!({"request":id,"state":"closed","reason":reason}),
            )?;
        }
        Ok(())
    }
}

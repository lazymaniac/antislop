//! A draft the plan verifier sent back is corrected in the turn it reaches;
//! the turn cannot end as if it had been.
use super::*;
use rusqlite::OptionalExtension;

impl Core {
    /// Refuses `finish_turn` while this orchestrator turn was handed a draft
    /// the plan verifier sent back and neither resubmitted it (submit_plan
    /// ends the turn itself) nor asked the planner to redraft it. Without
    /// this, a turn whose correction was rejected could end claiming it was
    /// sent, and nothing would pick the draft up again.
    pub(in crate::lean) fn require_plan_correction(
        &self,
        conversation: &str,
        turn: &str,
    ) -> Result<()> {
        let upto: Option<i64> = self
            .store
            .db()
            .query_row(
                "SELECT upto_message FROM turns WHERE id=?1",
                [turn],
                |row| row.get(0),
            )
            .optional()?;
        let Some(upto) = upto else {
            return Ok(());
        };
        let answered = self.store.last_orchestration_answered(conversation)?;
        let asked = self.asked_since(conversation, answered, upto as u64)?;
        let sent_back = asked.iter().any(|message| message.author == PLAN_VERIFIER);
        if !sent_back
            || !self
                .store
                .messages_by(conversation, REPLAN, answered)?
                .is_empty()
        {
            return Ok(());
        }
        let draft = self.store.latest_plan(conversation)?;
        let uncorrected =
            !draft.is_empty() && draft.iter().all(|task| task.status == status::REVISED);
        require(
            !uncorrected,
            "the plan verifier sent the draft back and it is not corrected yet; finish_turn cannot end this turn. Submit the complete corrected plan with submit_plan (it ends the turn), call request_replan for the planner to redraft it, or ask_user for a decision you need",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use serde_json::json;

    #[test]
    fn a_sent_back_draft_must_be_corrected_before_the_turn_ends() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::from_toml(include_str!("../../examples/antislop.toml")).unwrap();
        let core = Core::open(&dir.path().join("state"), config).unwrap();
        let id = core
            .store
            .create_conversation(dir.path().to_str().unwrap())
            .unwrap()
            .id;
        let task =
            json!({"id":"t1","title":"T","goal":"g","kind":"analysis","verifier":{"focus":"f"}});
        let plan = core.store.add_plan(&id, &[task]).unwrap()[0]
            .plan_id
            .clone();
        core.store
            .set_plan_status(&plan, status::PROPOSED, status::REVISED)
            .unwrap();
        let findings = core
            .store
            .add_message(&id, PLAN_VERIFIER, "Add the test files to the scope.")
            .unwrap();
        let turn = core
            .store
            .start_turn(&id, ORCHESTRATOR, "codex", findings.seq, None)
            .unwrap();
        let done = json!({"text": "Submitted the correction."});

        let error = core
            .agent_tool(&id, ORCHESTRATOR, &turn.id, "finish_turn", &done)
            .unwrap_err()
            .to_string();
        assert!(error.contains("not corrected yet"), "{error}");

        // A redraft requested from the planner answers the findings too.
        core.agent_tool(
            &id,
            ORCHESTRATOR,
            &turn.id,
            "request_replan",
            &json!({"reason": "the scope needs a redraft"}),
        )
        .unwrap();
        core.agent_tool(&id, ORCHESTRATOR, &turn.id, "finish_turn", &done)
            .unwrap();
    }

    #[test]
    fn a_turn_without_findings_ends_freely() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::from_toml(include_str!("../../examples/antislop.toml")).unwrap();
        let core = Core::open(&dir.path().join("state"), config).unwrap();
        let id = core
            .store
            .create_conversation(dir.path().to_str().unwrap())
            .unwrap()
            .id;
        let task =
            json!({"id":"t1","title":"T","goal":"g","kind":"analysis","verifier":{"focus":"f"}});
        let plan = core.store.add_plan(&id, &[task]).unwrap()[0]
            .plan_id
            .clone();
        core.store
            .set_plan_status(&plan, status::PROPOSED, status::REVISED)
            .unwrap();
        let question = core.store.add_message(&id, "user", "Status?").unwrap();
        let turn = core
            .store
            .start_turn(&id, ORCHESTRATOR, "codex", question.seq, None)
            .unwrap();
        core.agent_tool(
            &id,
            ORCHESTRATOR,
            &turn.id,
            "finish_turn",
            &json!({"text": "Waiting for your review."}),
        )
        .unwrap();
    }
}

use super::*;

impl Store {
    #[cfg(test)]
    pub(in crate::lean) fn answer_agent_with_disposition(
        &self,
        conversation: &str,
        request: &str,
        answer: &str,
        disposition: &str,
        target: &str,
    ) -> Result<Value> {
        self.answer_agent_from_turn(conversation, request, answer, disposition, target, None)
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::lean) fn answer_agent_from_turn(
        &self,
        conversation: &str,
        request: &str,
        answer: &str,
        disposition: &str,
        target: &str,
        caller: Option<&str>,
    ) -> Result<Value> {
        crate::require(
            matches!(
                disposition,
                "continue" | "wait_for_revision" | "wait_for_dependency"
            ),
            "disposition: expected continue, wait_for_revision, wait_for_dependency or escalate_to_user",
        )?;
        crate::require(!answer.trim().is_empty(), "answer is required")?;
        let mut db = self.db();
        let tx = db.transaction()?;
        if let Some(turn) = caller {
            let current: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM turns t WHERE t.id=?1 AND t.conversation_id=?2 AND t.role IN ('orchestrator','comment-orchestrator') AND t.status='running' AND NOT EXISTS(SELECT 1 FROM turns newer WHERE newer.conversation_id=t.conversation_id AND newer.role=t.role AND newer.rowid>t.rowid))",params![turn,conversation],|r|r.get(0))?;
            crate::require(current, "answer caller turn is closed or superseded")?;
        }
        let signature = crate::hash(serde_json::to_vec(&serde_json::json!([
            answer,
            disposition,
            target
        ]))?);
        let previous: Option<(String, String)> = tx
            .query_row(
                "SELECT signature,result FROM agent_answer_receipts WHERE request=?1 AND conversation=?2",
                params![request,conversation],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((previous, result)) = previous {
            crate::require(
                previous == signature,
                "question already has a different answer or disposition",
            )?;
            return Ok(serde_json::from_str(&result)?);
        }
        if disposition == "continue" {
            let has_wait: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_answer_waits WHERE request=?1)",
                [request],
                |r| r.get(0),
            )?;
            crate::require(!has_wait, "question already has a different disposition")?;
        } else {
            let owner: String = tx.query_row("SELECT t.plan_id FROM agent_questions q JOIN turns t ON t.id=q.turn_id WHERE q.id=?1 AND q.conversation_id=?2",params![request,conversation],|r|r.get(0))?;
            let previous: Option<(String, String)> = tx
                .query_row(
                    "SELECT disposition,target FROM agent_answer_waits WHERE request=?1",
                    [request],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some(previous) = previous {
                crate::require(
                    previous == (disposition.into(), target.into()),
                    "question already has a different disposition",
                )?;
            } else {
                let valid: bool = if disposition == "wait_for_revision" {
                    tx.query_row("SELECT EXISTS(SELECT 1 FROM plan_revision_jobs WHERE id=?1 AND conversation_id=?2 AND plan_id=?3 AND status IN ('reviewing','applied'))",params![target,conversation,owner],|r|r.get(0))?
                } else {
                    tx.query_row("SELECT EXISTS(SELECT 1 FROM tasks WHERE id=?1 AND conversation_id=?2 AND plan_id=?3 AND status<>'superseded' AND id<>(SELECT task_id FROM turn_scopes WHERE turn_id=(SELECT turn_id FROM agent_questions WHERE id=?4)))",params![target,conversation,owner,request],|r|r.get(0))?
                };
                crate::require(
                    valid,
                    "target: expected an active revision/dependency of the same plan and conversation",
                )?;
                let answered: bool = tx.query_row(
                    "SELECT state='answered' FROM agent_questions WHERE id=?1",
                    [request],
                    |r| r.get(0),
                )?;
                crate::require(!answered, "question already has a different disposition")?;
                tx.execute(
                    "INSERT INTO agent_answer_waits(request,disposition,target) VALUES(?1,?2,?3)",
                    params![request, disposition, target],
                )?;
            }
        }
        super::agent_exchange::answer_question_tx(&tx, conversation, request, answer)?;
        let receipt = crate::id();
        let result = serde_json::json!({"request":request,"state":"answered","disposition":disposition,"target":target,"released":disposition=="continue","delivered":true,"receipt":receipt});
        tx.execute("INSERT INTO agent_answer_receipts(request,conversation,signature,result,resume_pending) VALUES(?1,?2,?3,?4,1)",params![request,conversation,signature,result.to_string()])?;
        tx.execute("INSERT INTO events(conversation_id,kind,payload,created_ms) VALUES(?1,'agent_answer_delivered',?2,?3)",params![conversation,result.to_string(),now_ms()])?;
        tx.commit()?;
        drop(db);
        self.refresh_answer_waits(conversation)?;
        Ok(result)
    }

    /// Idempotent transition. Only the exact parked request may be released.
    pub(in crate::lean) fn refresh_answer_waits(&self, conversation: &str) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        tx.execute("UPDATE agent_answer_waits SET released=1 WHERE released=0 AND request IN (SELECT id FROM agent_questions WHERE conversation_id=?1 AND state='answered') AND ((disposition='wait_for_revision' AND EXISTS(SELECT 1 FROM plan_revision_jobs j WHERE j.id=target AND j.status='applied')) OR (disposition='wait_for_dependency' AND EXISTS(SELECT 1 FROM tasks t WHERE t.id=target AND t.status='approved')))",[conversation])?;
        tx.execute("UPDATE tasks SET status='queued',feedback=json_set(coalesce(feedback,'{}'),'$.current_reason',NULL) WHERE conversation_id=?1 AND status='waiting' AND json_extract(feedback,'$.waiting_request') IN (SELECT request FROM agent_answer_waits WHERE released=1)",[conversation])?;
        tx.execute("UPDATE agent_answer_waits SET released=2 WHERE released=1 AND request IN (SELECT json_extract(feedback,'$.waiting_request') FROM tasks WHERE conversation_id=?1 AND status='queued')",[conversation])?;
        tx.commit()?;
        Ok(())
    }
}

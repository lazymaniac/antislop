//! Explicit, one-use recovery decisions. Generic interview answers grant nothing.
use super::*;
use crate::lean::core_task_recovery::{RecoveryScope, RecoveryScopeBinding};

pub(in crate::lean) struct RecoveryDecision {
    pub conversation: String,
    pub id: String,
    pub plan: String,
    pub task: String,
    pub generation: i64,
    pub round: u32,
    pub token: String,
    pub guidance: String,
    pub answer: Message,
    pub question: super::super::interview::Question,
    pub scope: RecoveryScope,
    pub authorization_hash: Option<String>,
}

pub(super) fn capture_answer(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    author: &str,
    text: &str,
    seq: u64,
) -> Result<()> {
    if author == "user"
        && let Some((decision, _)) = text
            .strip_prefix("Interview answer (")
            .and_then(|text| text.split_once(")\n"))
    {
        let question: Option<String> = tx.query_row(
            "SELECT e.payload FROM recovery_decisions d JOIN events e ON e.conversation_id=d.conversation_id AND e.kind='user_decision_requested' AND json_extract(e.payload,'$.id')=d.decision_id WHERE d.conversation_id=?1 AND d.decision_id=?2 AND d.state='pending' AND d.answer_seq IS NULL ORDER BY e.seq DESC LIMIT 1",
            params![conversation,decision], |row| row.get(0),
        ).optional()?;
        if let Some(question) = question {
            let question: super::super::interview::Question = serde_json::from_str(&question)?;
            crate::require(
                question.options.iter().any(|option| {
                    text == format!(
                        "Interview answer ({})\nQuestion: {}\nAnswer: {}",
                        question.id, question.question, option.label
                    )
                }),
                "recovery answer must select an exact displayed option; no answer was saved and the decision remains open",
            )?;
            tx.execute("UPDATE recovery_decisions SET answer_seq=?3 WHERE conversation_id=?1 AND decision_id=?2 AND state='pending' AND answer_seq IS NULL",params![conversation,decision,seq as i64])?;
        }
    }
    Ok(())
}

impl Store {
    pub(in crate::lean) fn bind_recovery_decision(
        &self,
        conversation: &str,
        task: &Task,
        token: &str,
        guidance: &str,
        question: &super::super::interview::Question,
        binding: &RecoveryScopeBinding,
    ) -> Result<()> {
        let stored = self.content.encode(guidance)?;
        let mut db = self.db();
        let tx = db.transaction()?;
        let previous: Option<(String, String)> = tx.query_row(
            "SELECT decision_id,freshness_token FROM recovery_decisions WHERE task_id=?1 AND generation=?2 AND round=?3",
            params![task.id,task.input_generation,task.round],|r|Ok((r.get(0)?,r.get(1)?)),
        ).optional()?;
        if let Some((id, previous_token)) = previous {
            crate::require(
                id == question.id && previous_token == token,
                "a recovery decision already exists for this task round; preserve its answer instead of asking again",
            )?;
        } else {
            tx.execute("INSERT INTO recovery_decisions(conversation_id,decision_id,plan_id,task_id,generation,round,freshness_token,guidance,created_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![conversation,question.id,task.plan_id,task.id,task.input_generation,task.round,token,stored,now_ms()])?;
        }
        let scope = serde_json::to_value(binding.scope)?
            .as_str()
            .unwrap()
            .to_owned();
        tx.execute("INSERT INTO recovery_scopes(conversation_id,decision_id,scope,authorization_hash) VALUES(?1,?2,?3,?4) ON CONFLICT(conversation_id,decision_id) DO NOTHING",params![conversation,question.id,scope,binding.authorization_hash])?;
        let stored: (String,Option<String>) = tx.query_row("SELECT scope,authorization_hash FROM recovery_scopes WHERE conversation_id=?1 AND decision_id=?2",params![conversation,question.id],|r|Ok((r.get(0)?,r.get(1)?)))?;
        crate::require(
            stored == (scope, binding.authorization_hash.clone()),
            "recovery decision scope cannot be changed",
        )?;
        Self::write_user_decision(&tx, conversation, question)?;
        tx.commit()?;
        Ok(())
    }

    pub(in crate::lean) fn answered_recovery_decisions(
        &self,
        conversation: &str,
    ) -> Result<Vec<RecoveryDecision>> {
        let db = self.db();
        let mut statement = db.prepare("SELECT d.decision_id,d.plan_id,d.task_id,d.generation,d.round,d.freshness_token,d.guidance,m.seq,m.author,m.text,(SELECT payload FROM events e WHERE e.conversation_id=d.conversation_id AND e.kind='user_decision_requested' AND json_extract(e.payload,'$.id')=d.decision_id ORDER BY seq DESC LIMIT 1),coalesce(s.scope,'edits_only'),s.authorization_hash FROM recovery_decisions d LEFT JOIN recovery_scopes s ON s.conversation_id=d.conversation_id AND s.decision_id=d.decision_id JOIN messages m ON m.seq=d.answer_seq AND m.conversation_id=d.conversation_id WHERE d.conversation_id=?1 AND d.state='pending' ORDER BY d.created_ms")?;
        let rows = statement.query_map([conversation], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, u32>(4)?,
                r.get::<_, String>(5)?,
                self.content.sql_text(r, 6)?,
                r.get::<_, i64>(7)? as u64,
                r.get::<_, String>(8)?,
                self.content.sql_text(r, 9)?,
                r.get::<_, String>(10)?,
                r.get::<_, String>(11)?,
                r.get::<_, Option<String>>(12)?,
            ))
        })?;
        rows.map(|row| {
            let (
                id,
                plan,
                task,
                generation,
                round,
                token,
                guidance,
                seq,
                author,
                text,
                question,
                scope,
                authorization_hash,
            ) = row?;
            Ok(RecoveryDecision {
                conversation: conversation.into(),
                id,
                plan,
                task,
                generation,
                round,
                token,
                guidance,
                answer: Message { seq, author, text },
                question: serde_json::from_str(&question)?,
                scope: serde_json::from_value(serde_json::json!(scope))?,
                authorization_hash,
            })
        })
        .collect()
    }

    /// Queue + guidance + approval consumption + event are one commit. A crash
    /// cannot consume permission without preserving the work to start.
    pub(in crate::lean) fn settle_recovery_decision(
        &self,
        decision: &RecoveryDecision,
        rejection: Option<&str>,
    ) -> Result<(bool, Option<String>)> {
        use super::comments_and_mail::{append_mail_event, insert_mail_row};
        let stored = self.content.encode(&decision.guidance)?;
        let mut db = self.db();
        let tx = db.transaction()?;
        let latest: Option<String> = tx.query_row("SELECT json_extract(payload,'$.id') FROM events WHERE conversation_id=?1 AND kind='user_decision_requested' ORDER BY seq DESC LIMIT 1",[&decision.conversation],|r|r.get(0)).optional()?;
        let rejection = rejection.or_else(|| {
            (latest.as_deref() != Some(decision.id.as_str()))
                .then_some("recovery decision was superseded; task left stopped")
        });
        let state = if rejection.is_some() {
            "rejected"
        } else {
            "consumed"
        };
        if tx.execute("UPDATE recovery_decisions SET state=?3 WHERE conversation_id=?1 AND decision_id=?2 AND state='pending' AND answer_seq=?4",params![decision.conversation,decision.id,state,decision.answer.seq as i64])? == 0 {
            return Ok((false, None));
        }
        let mut payload = serde_json::json!({"actor":"user","source":"recovery_decision","decision":decision.id,"answer":decision.answer.seq,"task":decision.task,"generation":decision.generation,"round":decision.round,"scope":decision.scope.event_scope()});
        let kind = if let Some(reason) = rejection {
            payload["reason"] = reason.into();
            "task_recovery_rejected"
        } else {
            let changed = tx.execute("UPDATE tasks SET status='queued',verdict=NULL,feedback=NULL WHERE id=?1 AND plan_id=?2 AND conversation_id=?3 AND round=?4 AND status IN ('coordinating','failed') AND coalesce(json_extract(spec,'$.deferred'),0)=0 AND EXISTS(SELECT 1 FROM task_inputs i WHERE i.task_id=tasks.id AND i.generation=?5)",params![decision.task,decision.plan,decision.conversation,decision.round,decision.generation])?;
            crate::require(
                changed == 1,
                "recovery task changed before approval was applied",
            )?;
            let role = format!("worker:{}", decision.task);
            let mail = Mail {
                id: 0,
                sender: super::super::RUNTIME.into(),
                recipient: role.clone(),
                kind: "guidance".into(),
                text: decision.guidance.clone(),
                created_ms: 0,
                delivered_turn: None,
                delivered_ms: None,
                state: "queued".into(),
            };
            let mail = insert_mail_row(&tx, &decision.conversation, &role, &mail, None, &stored)?;
            append_mail_event(&tx, &decision.conversation, &mail, &stored)?;
            payload["action"] = "retry_with_guidance".into();
            payload["status"] = "queued".into();
            "task_recovery"
        };
        tx.execute("INSERT INTO events(conversation_id,plan_id,kind,payload,created_ms) VALUES(?1,?2,?3,?4,?5)",params![decision.conversation,decision.plan,kind,payload.to_string(),now_ms()])?;
        tx.commit()?;
        Ok((rejection.is_none(), rejection.map(str::to_owned)))
    }
}

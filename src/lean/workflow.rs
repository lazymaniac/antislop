//! Durable interview checkpoint. Every conversation starts in its interview.
use super::{Message, Store, now_ms};
use crate::{Result, require};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;

#[path = "workflow/repair.rs"]
mod repair;

#[cfg(test)]
#[path = "workflow/tests.rs"]
mod tests;

#[derive(Debug, Serialize)]
pub struct Workflow {
    pub phase: String,
    pub summary: String,
    pub upto_message: u64,
}

impl Workflow {
    pub fn interviewing(&self) -> bool {
        self.phase == "interview"
    }

    pub fn planning(&self) -> bool {
        self.phase == "planning"
    }
}

impl Store {
    pub(crate) fn queue_requirements_format_repair(&self, conversation: &str) -> Result<bool> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let queued = repair::queue_requirements_format_repair(&tx, conversation)?;
        tx.commit()?;
        Ok(queued)
    }

    pub fn workflow(&self, conversation: &str) -> Result<Workflow> {
        self.db()
            .query_row(
                "SELECT phase,summary,upto_message FROM session_workflow WHERE conversation_id=?1",
                [conversation],
                |r| {
                    Ok(Workflow {
                        phase: r.get(0)?,
                        summary: r.get(1)?,
                        upto_message: r.get::<_, i64>(2)? as u64,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| crate::Error::Invalid("conversation has no workflow".into()))
    }

    /// Execution decisions are independent of worker questions and workflow approval.
    pub fn ask_user_decision(
        &self,
        conversation: &str,
        question: &crate::lean::interview::Question,
    ) -> Result<()> {
        question.validate()?;
        let payload = serde_json::to_string(question)?;
        crate::require(payload.len() <= 16384, "user decision exceeds 16 KiB")?;
        let mut db = self.db();
        let tx = db.transaction()?;
        let previous: Option<String> = tx.query_row(
            "SELECT payload FROM events WHERE conversation_id=?1 AND kind='user_decision_requested' AND json_extract(payload,'$.id')=?2 ORDER BY seq DESC LIMIT 1",
            params![conversation, question.id], |row| row.get(0),
        ).optional()?;
        if let Some(previous) = previous {
            crate::require(
                previous == payload,
                "decision id already used for another question; use a new id",
            )?;
        } else {
            tx.execute("INSERT INTO events(conversation_id,kind,payload,created_ms) VALUES(?1,'user_decision_requested',?2,?3)", params![conversation,payload,now_ms()])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Whether the planner still owes this planning cycle its first plan: the
    /// conversation is planning, and no plan was drafted since the interview
    /// that led here ended. Plans of earlier cycles do not count.
    pub fn awaiting_first_plan(&self, conversation: &str) -> Result<bool> {
        Ok(self.db().query_row(
            "SELECT w.phase='planning' AND NOT EXISTS(
                 SELECT 1 FROM tasks t WHERE t.conversation_id=?1 AND t.created_ms >= COALESCE(
                     (SELECT max(created_ms) FROM events
                      WHERE conversation_id=?1 AND kind='workflow'
                        AND json_extract(payload,'$.phase')='planning'),0))
             FROM session_workflow w WHERE w.conversation_id=?1",
            [conversation],
            |row| row.get(0),
        )?)
    }

    pub fn ask_interview_question(
        &self,
        conversation: &str,
        question: &crate::lean::interview::Question,
    ) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let changed = tx.execute(
            "UPDATE session_workflow SET summary='' WHERE conversation_id=?1 AND phase='interview'",
            [conversation],
        )?;
        require(changed == 1, "session is not in interview")?;
        tx.execute("INSERT INTO events(conversation_id,kind,payload,created_ms) VALUES(?1,'interview_question',?2,?3)", params![conversation,serde_json::to_string(question)?,now_ms()])?;
        tx.commit()?;
        Ok(())
    }

    pub fn interview_summary(&self, conversation: &str, summary: &str, upto: u64) -> Result<()> {
        require(!summary.trim().is_empty(), "requirements summary is empty")?;
        let mut db = self.db();
        let tx = db.transaction()?;
        let changed = tx.execute("UPDATE session_workflow SET summary=?2,upto_message=?3 WHERE conversation_id=?1 AND phase='interview'", params![conversation,summary,upto as i64])?;
        require(changed == 1, "session is not in interview")?;
        tx.execute("INSERT INTO events(conversation_id,kind,payload,created_ms) VALUES(?1,'workflow',?2,?3)",
            params![conversation,serde_json::json!({"phase":"interview","summary":summary,"upto_message":upto}).to_string(),now_ms()])?;
        tx.commit()?;
        Ok(())
    }

    /// Finishing the interview queues planning atomically, without inventing user approval.
    /// A newer answer keeps the interview open until its requirements are incorporated.
    pub fn finish_interview(&self, conversation: &str) -> Result<bool> {
        let mut db = self.db();
        let tx = db.transaction()?;
        if !promote_current_interview(&tx, conversation)? {
            return Ok(false);
        }
        queue_planning_message(&tx, conversation)?;
        tx.commit()?;
        Ok(true)
    }

    /// Acceptance and its wake-up message share one transaction; new user input
    /// invalidates the checkpoint even if it arrived while the agent answered.
    pub fn approve_requirements(&self, conversation: &str) -> Result<Message> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let summary = approve_current_requirements(&tx, conversation)?;
        let text = format!(
            "I approve the following requirements:\n\n{summary}\n\nPrepare the plan for my review; implementation requires separate plan approval."
        );
        let stored_text = self.content.encode(&text)?;
        let seq = store_approval_message(&tx, conversation, &stored_text)?;
        tx.commit()?;
        Ok(Message {
            seq,
            author: "user".into(),
            text,
        })
    }
}

fn promote_current_interview(tx: &rusqlite::Transaction<'_>, conversation: &str) -> Result<bool> {
    let changed = tx.execute(
        "UPDATE session_workflow SET phase='planning' WHERE conversation_id=?1 AND phase='interview' AND summary<>'' AND upto_message >= COALESCE((SELECT MAX(seq) FROM messages WHERE conversation_id=?1 AND author='user'),0)",
        [conversation],
    )?;
    Ok(changed == 1)
}

fn queue_planning_message(tx: &rusqlite::Transaction<'_>, conversation: &str) -> Result<()> {
    let text = "Interview complete. Prepare the plan from the collected requirements for review. The runtime starts implementation once the reviewed plan is accepted.";
    tx.execute(
        "INSERT INTO messages(conversation_id,author,text,created_ms) VALUES(?1,'runtime',?2,?3)",
        params![conversation, text, now_ms()],
    )?;
    let seq = tx.last_insert_rowid();
    record_event(
        tx,
        conversation,
        "workflow",
        serde_json::json!({"phase":"planning"}),
    )?;
    record_event(
        tx,
        conversation,
        "message",
        serde_json::json!({"seq":seq,"author":"runtime","text":text,"internal":true}),
    )?;
    record_event(
        tx,
        conversation,
        "planning_progress",
        serde_json::json!({"text":"Working on the plan draft"}),
    )
}

fn approve_current_requirements(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
) -> Result<String> {
    require(
        promote_current_interview(tx, conversation)?,
        "wait for an up-to-date requirements summary before approving with /planning",
    )?;
    Ok(tx.query_row(
        "SELECT summary FROM session_workflow WHERE conversation_id=?1",
        [conversation],
        |row| row.get(0),
    )?)
}

fn store_approval_message(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    text: &str,
) -> Result<u64> {
    tx.execute(
        "INSERT INTO messages(conversation_id,author,text,created_ms) VALUES(?1,'user',?2,?3)",
        params![conversation, text, now_ms()],
    )?;
    let seq = tx.last_insert_rowid() as u64;
    record_event(
        tx,
        conversation,
        "message",
        serde_json::json!({"seq":seq,"author":"user","text":text}),
    )?;
    record_event(
        tx,
        conversation,
        "workflow",
        serde_json::json!({"phase":"planning"}),
    )?;
    Ok(seq)
}

/// Every conversation begins in its interview; planning follows from it.
pub(super) fn begin_interview(tx: &rusqlite::Transaction<'_>, conversation: &str) -> Result<()> {
    tx.execute(
        "INSERT INTO session_workflow(conversation_id,phase) VALUES(?1,'interview')",
        [conversation],
    )?;
    record_event(
        tx,
        conversation,
        "workflow",
        serde_json::json!({"phase":"interview","summary":""}),
    )
}

fn record_event(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    kind: &str,
    payload: serde_json::Value,
) -> Result<()> {
    tx.execute(
        "INSERT INTO events(conversation_id,kind,payload,created_ms) VALUES(?1,?2,?3,?4)",
        params![conversation, kind, payload.to_string(), now_ms()],
    )?;
    Ok(())
}

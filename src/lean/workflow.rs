//! Durable interview checkpoint. Every conversation starts in its interview.
use super::{Store, now_ms};
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
    #[cfg(test)]
    pub(crate) fn confirm_scope_for_test(&self, conversation: &str) {
        assert!(!self.finish_interview(conversation).unwrap());
        let request = self
            .events_since(conversation, 0, 100)
            .unwrap()
            .into_iter()
            .rev()
            .find(|event| event.kind == "scope_confirmation_requested")
            .unwrap();
        let question: crate::lean::interview::Question =
            serde_json::from_value(request.payload["question"].clone()).unwrap();
        self.add_message(
            conversation,
            "user",
            &format!("Interview answer ({})\nAnswer: Confirm scope", question.id),
        )
        .unwrap();
        assert!(self.finish_interview(conversation).unwrap());
    }

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
        let mut db = self.db();
        let tx = db.transaction()?;
        Self::write_user_decision(&tx, conversation, question)?;
        tx.commit()?;
        Ok(())
    }

    pub(super) fn write_user_decision(
        tx: &rusqlite::Transaction<'_>,
        conversation: &str,
        question: &crate::lean::interview::Question,
    ) -> Result<()> {
        question.validate()?;
        let payload = serde_json::to_string(question)?;
        crate::require(payload.len() <= 16384, "user decision exceeds 16 KiB")?;
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
        Ok(())
    }

    /// The decision put to the user now: the newest one, until an answer
    /// names it. A newer decision supersedes an unanswered older one, which
    /// the interface no longer shows either. `None` when none was ever asked.
    pub fn open_user_decision(
        &self,
        conversation: &str,
    ) -> Result<Option<Option<crate::lean::interview::Question>>> {
        let latest: Option<String> = self
            .db()
            .query_row(
                "SELECT payload FROM events WHERE conversation_id=?1 AND kind='user_decision_requested' ORDER BY seq DESC LIMIT 1",
                [conversation],
                |row| row.get(0),
            )
            .optional()?;
        let Some(latest) = latest else {
            return Ok(None);
        };
        let question: crate::lean::interview::Question = serde_json::from_str(&latest)?;
        let answer = format!("Interview answer ({})\n", question.id);
        let answered = self
            .messages_by(conversation, "user", 0)?
            .iter()
            .any(|message| message.text.starts_with(&answer));
        Ok(Some((!answered).then_some(question)))
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

    /// Finishing the interview requests scope confirmation, then queues planning
    /// atomically only after the user confirms the current summary.
    pub fn finish_interview(&self, conversation: &str) -> Result<bool> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let Some((summary, upto_message)) = current_interview(&tx, conversation)? else {
            return Ok(false);
        };
        let latest_user: i64 = tx.query_row(
            "SELECT COALESCE(MAX(seq),0) FROM messages WHERE conversation_id=?1 AND author='user'",
            [conversation],
            |row| row.get(0),
        )?;
        if summary.is_empty() {
            return Ok(false);
        }

        if scope_is_confirmed(&tx, conversation, &summary, upto_message)? {
            if promote_current_interview(&tx, conversation)? {
                queue_planning_message(&tx, conversation)?;
                tx.commit()?;
                return Ok(true);
            }
            return Ok(false);
        }

        if let Some((_, requested_summary, requested_upto)) =
            latest_scope_confirmation(&tx, conversation)?
            && requested_summary == summary
            && requested_upto == upto_message
        {
            return Ok(false);
        }

        if upto_message != latest_user as u64 {
            return Ok(false);
        }

        request_scope_confirmation(&tx, conversation, &summary, upto_message)?;
        tx.commit()?;
        Ok(false)
    }
}

impl Store {
    pub(super) fn record_scope_confirmation_answer(
        tx: &rusqlite::Transaction<'_>,
        conversation: &str,
        text: &str,
        seq: u64,
    ) -> Result<()> {
        let Some((question, summary, upto_message)) = latest_scope_confirmation(tx, conversation)?
        else {
            return Ok(());
        };
        let answer_prefix = format!("Interview answer ({})\n", question.id);
        let Some(selection) = text.strip_prefix(&answer_prefix).and_then(|answer| {
            answer
                .lines()
                .find_map(|line| line.strip_prefix("Answer: "))
        }) else {
            return Ok(());
        };
        if !selection.trim().eq_ignore_ascii_case("Confirm scope") {
            return Ok(());
        }
        let current = current_interview(tx, conversation)?;
        let previous_user: i64 = tx.query_row(
            "SELECT COALESCE(MAX(seq),0) FROM messages WHERE conversation_id=?1 AND author='user' AND seq<?2",
            params![conversation, seq as i64],
            |row| row.get(0),
        )?;
        if current.as_ref() != Some(&(summary.clone(), upto_message))
            || previous_user as u64 != upto_message
        {
            return Ok(());
        }
        tx.execute(
            "UPDATE session_workflow SET upto_message=?2 WHERE conversation_id=?1 AND phase='interview' AND summary=?3 AND upto_message=?4",
            params![conversation, seq as i64, summary, upto_message as i64],
        )?;
        tx.execute(
            "INSERT INTO events(conversation_id,kind,payload,created_ms) VALUES(?1,'workflow',?2,?3)",
            params![conversation,serde_json::json!({"phase":"interview","summary":summary,"upto_message":seq}).to_string(),now_ms()],
        )?;
        tx.execute(
            "INSERT INTO events(conversation_id,kind,payload,created_ms) VALUES(?1,'scope_confirmed',?2,?3)",
            params![conversation,serde_json::json!({"question_id":question.id,"summary":summary,"upto_message":seq,"answer_message":seq}).to_string(),now_ms()],
        )?;
        Ok(())
    }
}

fn current_interview(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
) -> Result<Option<(String, u64)>> {
    Ok(tx
        .query_row(
            "SELECT summary,upto_message FROM session_workflow WHERE conversation_id=?1 AND phase='interview'",
            [conversation],
            |row| Ok((row.get(0)?, row.get::<_, i64>(1)? as u64)),
        )
        .optional()?)
}

fn latest_scope_confirmation(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
) -> Result<Option<(crate::lean::interview::Question, String, u64)>> {
    let payload: Option<String> = tx
        .query_row(
            "SELECT payload FROM events WHERE conversation_id=?1 AND kind='scope_confirmation_requested' ORDER BY seq DESC LIMIT 1",
            [conversation],
            |row| row.get(0),
        )
        .optional()?;
    payload
        .map(|payload| {
            let value: serde_json::Value = serde_json::from_str(&payload)?;
            Ok((
                serde_json::from_value(value["question"].clone())?,
                value["summary"].as_str().unwrap_or_default().to_owned(),
                value["upto_message"].as_u64().unwrap_or_default(),
            ))
        })
        .transpose()
}

fn scope_is_confirmed(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    summary: &str,
    upto_message: u64,
) -> Result<bool> {
    let payload: Option<String> = tx
        .query_row(
            "SELECT payload FROM events WHERE conversation_id=?1 AND kind='scope_confirmed' ORDER BY seq DESC LIMIT 1",
            [conversation],
            |row| row.get(0),
        )
        .optional()?;
    let Some(payload) = payload else {
        return Ok(false);
    };
    let payload: serde_json::Value = serde_json::from_str(&payload)?;
    let latest_user: i64 = tx.query_row(
        "SELECT COALESCE(MAX(seq),0) FROM messages WHERE conversation_id=?1 AND author='user'",
        [conversation],
        |row| row.get(0),
    )?;
    Ok(payload["summary"] == summary
        && payload["upto_message"].as_u64() == Some(upto_message)
        && payload["answer_message"].as_i64() == Some(latest_user)
        && !summary.trim().is_empty())
}

fn request_scope_confirmation(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    summary: &str,
    upto_message: u64,
) -> Result<()> {
    let question = crate::lean::interview::Question {
        id: format!("scope-confirmation-{}", crate::id()),
        context: "Planning must use the scope and acceptance criteria in this summary.".into(),
        question: "Do you confirm this scope so planning can begin?".into(),
        options: vec![
            crate::lean::interview::Choice {
                label: "Confirm scope".into(),
                description: "Proceed to planning with this summary.".into(),
            },
            crate::lean::interview::Choice {
                label: "Request changes".into(),
                description: "Keep the interview open so you can clarify the scope.".into(),
            },
        ],
    };
    Store::write_user_decision(tx, conversation, &question)?;
    tx.execute(
        "INSERT INTO events(conversation_id,kind,payload,created_ms) VALUES(?1,'scope_confirmation_requested',?2,?3)",
        params![conversation,serde_json::json!({"question":question,"summary":summary,"upto_message":upto_message}).to_string(),now_ms()],
    )?;
    Ok(())
}

fn promote_current_interview(tx: &rusqlite::Transaction<'_>, conversation: &str) -> Result<bool> {
    let changed = tx.execute(
        "UPDATE session_workflow SET phase='planning' WHERE conversation_id=?1 AND phase='interview' AND summary<>'' AND EXISTS(
             SELECT 1 FROM events e WHERE e.conversation_id=?1 AND e.kind='scope_confirmed'
               AND json_extract(e.payload,'$.summary')=session_workflow.summary
               AND json_extract(e.payload,'$.upto_message')=session_workflow.upto_message
               AND json_extract(e.payload,'$.answer_message')=(SELECT COALESCE(MAX(seq),0) FROM messages WHERE conversation_id=?1 AND author='user'))",
        [conversation],
    )?;
    Ok(changed == 1)
}

fn queue_planning_message(tx: &rusqlite::Transaction<'_>, conversation: &str) -> Result<()> {
    let text = "Interview complete. Prepare the plan from the collected requirements for review. After successful review, present the plan for the user's explicit acceptance before implementation starts.";
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

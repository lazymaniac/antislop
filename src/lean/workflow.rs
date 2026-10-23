//! Durable interview checkpoint. Missing rows preserve pre-workflow sessions.
use super::{Message, Store, now_ms};
use crate::{Result, require};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Workflow {
    pub phase: String,
    pub summary: String,
    pub upto_message: u64,
}

impl Store {
    pub fn queue_plan_format_repair(
        &self,
        conversation: &str,
        turn_id: &str,
        error: &str,
    ) -> Result<bool> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let upto:i64=tx.query_row("SELECT upto_message FROM turns WHERE id=?1 AND conversation_id=?2 AND role IN ('orchestrator','comment-orchestrator') AND status='done'",params![turn_id,conversation],|row|row.get(0))?;
        let user: i64 = tx.query_row(
            "SELECT coalesce(max(seq),0) FROM messages WHERE conversation_id=?1 AND author='user'",
            [conversation],
            |row| row.get(0),
        )?;
        if user > upto {
            return Ok(false);
        }
        let repairs:i64=tx.query_row("SELECT count(*) FROM messages WHERE conversation_id=?1 AND author='plan-format-repair' AND seq>?2",params![conversation,user],|row|row.get(0))?;
        let queued:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM messages WHERE conversation_id=?1 AND author='plan-format-repair' AND seq>?2)",params![conversation,upto],|row|row.get(0))?;
        if repairs >= 2 || queued {
            return Ok(false);
        }
        let text = format!(
            "Repair the plan JSON in your previous response. Parser/validation error: {error}. Return the complete corrected ```plan JSON block for the SAME plan. Preserve the agreed requirements, selected strategy, tasks and dependencies. For JSON syntax errors, repair serialization only: escape ASCII double quotes inside string values, or use typographic quotes in prose. Do not repeat repository analysis, requirements extraction or options comparison, do not create another run, and do not ask the user to repair JSON. Your previous turn was {turn_id}."
        );
        tx.execute("INSERT INTO messages(conversation_id,author,text,created_ms) VALUES(?1,'plan-format-repair',?2,?3)",params![conversation,text,now_ms()])?;
        let seq = tx.last_insert_rowid();
        for (kind, payload) in [
            (
                "message",
                serde_json::json!({"seq":seq,"author":"plan-format-repair","text":text,"internal":true}),
            ),
            (
                "planning_progress",
                serde_json::json!({"text":"Correcting the plan format. Requirements and solution comparison are preserved."}),
            ),
        ] {
            tx.execute(
                "INSERT INTO events(conversation_id,kind,payload,created_ms) VALUES(?1,?2,?3,?4)",
                params![conversation, kind, payload.to_string(), now_ms()],
            )?;
        }
        tx.commit()?;
        Ok(true)
    }

    pub fn workflow(&self, conversation: &str) -> Result<Option<Workflow>> {
        Ok(self
            .db()
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
            .optional()?)
    }

    pub fn begin_interview(&self, conversation: &str) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        tx.execute(
            "INSERT INTO session_workflow(conversation_id,phase) VALUES(?1,'interview')",
            [conversation],
        )?;
        tx.execute("INSERT INTO events(conversation_id,kind,payload,created_ms) VALUES(?1,'workflow',?2,?3)",
            params![conversation, serde_json::json!({"phase":"interview","summary":""}).to_string(), now_ms()])?;
        tx.commit()?;
        Ok(())
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
        let changed = tx.execute("UPDATE session_workflow SET phase='planning' WHERE conversation_id=?1 AND phase='interview' AND summary<>'' AND upto_message >= COALESCE((SELECT MAX(seq) FROM messages WHERE conversation_id=?1 AND author='user'),0)", [conversation])?;
        if changed == 0 {
            return Ok(false);
        }
        let text = "Interview complete. Prepare the plan from the collected requirements for review. Implementation requires separate user approval of the plan.";
        tx.execute("INSERT INTO messages(conversation_id,author,text,created_ms) VALUES(?1,'runtime',?2,?3)", params![conversation,text,now_ms()])?;
        let seq = tx.last_insert_rowid();
        for (kind, payload) in [
            ("workflow", serde_json::json!({"phase":"planning"})),
            (
                "message",
                serde_json::json!({"seq":seq,"author":"runtime","text":text,"internal":true}),
            ),
            (
                "planning_progress",
                serde_json::json!({"text":"Working on the plan draft"}),
            ),
        ] {
            tx.execute(
                "INSERT INTO events(conversation_id,kind,payload,created_ms) VALUES(?1,?2,?3,?4)",
                params![conversation, kind, payload.to_string(), now_ms()],
            )?;
        }
        tx.commit()?;
        Ok(true)
    }

    /// Acceptance and its wake-up message share one transaction; new user input
    /// invalidates the checkpoint even if it arrived while the agent answered.
    pub fn approve_requirements(&self, conversation: &str) -> Result<Message> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let changed = tx.execute("UPDATE session_workflow SET phase='planning' WHERE conversation_id=?1 AND phase='interview' AND summary<>'' AND upto_message >= COALESCE((SELECT MAX(seq) FROM messages WHERE conversation_id=?1 AND author='user'),0)", [conversation])?;
        require(
            changed == 1,
            "wait for an up-to-date requirements summary before approving with /planning",
        )?;
        let summary: String = tx.query_row(
            "SELECT summary FROM session_workflow WHERE conversation_id=?1",
            [conversation],
            |row| row.get(0),
        )?;
        let text = format!(
            "I approve the following requirements:\n\n{summary}\n\nPrepare the plan for my review; implementation requires separate plan approval."
        );
        let stored_text = self.content.encode(&text)?;
        tx.execute(
            "INSERT INTO messages(conversation_id,author,text,created_ms) VALUES(?1,'user',?2,?3)",
            params![conversation, stored_text, now_ms()],
        )?;
        let seq = tx.last_insert_rowid() as u64;
        for (kind, payload) in [
            (
                "message",
                serde_json::json!({"seq":seq,"author":"user","text":stored_text}),
            ),
            ("workflow", serde_json::json!({"phase":"planning"})),
        ] {
            tx.execute(
                "INSERT INTO events(conversation_id,kind,payload,created_ms) VALUES(?1,?2,?3,?4)",
                params![conversation, kind, payload.to_string(), now_ms()],
            )?;
        }
        tx.commit()?;
        Ok(Message {
            seq,
            author: "user".into(),
            text,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_followup_restarts_interview_without_erasing_history() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("state.db")).unwrap();
        let conversation = store.create_conversation("project").unwrap();
        store.begin_interview(&conversation.id).unwrap();
        let first = store
            .add_message(&conversation.id, "user", "First goal")
            .unwrap();
        store
            .interview_summary(&conversation.id, "First scope", first.seq)
            .unwrap();
        store.finish_interview(&conversation.id).unwrap();
        let run = store.create_run(&conversation.id, "First goal").unwrap();
        store
            .select_planning_run(&conversation.id, &run.id)
            .unwrap();
        let tasks = store
            .add_plan(
                &conversation.id,
                &[serde_json::json!({"id":"implementation"})],
            )
            .unwrap();
        let plan = &tasks[0].plan_id;
        store.queue_plan(&conversation.id, plan).unwrap();
        // A message while work remains must not reset its workflow.
        store
            .add_message(&conversation.id, "user", "More context")
            .unwrap();
        assert_eq!(
            store.workflow(&conversation.id).unwrap().unwrap().phase,
            "implementation"
        );
        store.set_plan_status(plan, "queued", "approved").unwrap();
        store
            .add_message(&conversation.id, "runtime", "Finished")
            .unwrap();
        assert_eq!(
            store.workflow(&conversation.id).unwrap().unwrap().phase,
            "implementation"
        );
        store
            .add_message(&conversation.id, "user", "Next goal")
            .unwrap();
        let workflow = store.workflow(&conversation.id).unwrap().unwrap();
        assert_eq!(workflow.phase, "interview");
        assert!(workflow.summary.is_empty());
        assert_eq!(workflow.upto_message, 0);
        let next_run = store.planning_run(&conversation.id).unwrap().unwrap();
        assert_ne!(next_run.id, run.id);
        assert_eq!(next_run.goal, "Next goal");
        assert!(store.run(&conversation.id, &run.id).is_ok());
        assert_eq!(store.plan(plan).unwrap()[0].status, "approved");
        store
            .add_message(&conversation.id, "user", "Next constraint")
            .unwrap();
        assert_eq!(
            store.workflow(&conversation.id).unwrap().unwrap().phase,
            "interview"
        );
    }

    #[test]
    fn acceptance_requires_current_summary_and_survives_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let store = Store::open(&path).unwrap();
        let conversation = store.create_conversation("project").unwrap();
        store.begin_interview(&conversation.id).unwrap();
        assert!(store.approve_requirements(&conversation.id).is_err());
        let first = store
            .add_message(&conversation.id, "user", "Goal 👩‍💻")
            .unwrap();
        store
            .interview_summary(&conversation.id, "Scope and acceptance criteria", first.seq)
            .unwrap();
        let next = store
            .add_message(&conversation.id, "user", "Also preserve drafts")
            .unwrap();
        assert!(store.approve_requirements(&conversation.id).is_err());
        store
            .interview_summary(
                &conversation.id,
                "Scope, draft preservation, criteria",
                next.seq,
            )
            .unwrap();
        let accepted = store.approve_requirements(&conversation.id).unwrap();
        assert!(
            accepted
                .text
                .contains("Scope, draft preservation, criteria")
        );
        assert!(
            store
                .messages_by(&conversation.id, "user", 0)
                .unwrap()
                .last()
                .unwrap()
                .text
                .contains("Scope, draft preservation, criteria")
        );
        assert!(store.approve_requirements(&conversation.id).is_err());
        drop(store);
        let store = Store::open(&path).unwrap();
        let workflow = store.workflow(&conversation.id).unwrap().unwrap();
        assert_eq!(workflow.phase, "planning");
        assert!(workflow.summary.contains("draft preservation"));
        let events = store.events_since(&conversation.id, 0, 100).unwrap();
        assert_eq!(events.last().unwrap().payload["phase"], "planning");
    }

    #[test]
    fn automatic_planning_is_fresh_idempotent_and_does_not_forge_user_approval() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("state.db")).unwrap();
        let conversation = store.create_conversation("project").unwrap();
        store.begin_interview(&conversation.id).unwrap();
        assert!(!store.finish_interview(&conversation.id).unwrap());
        let first = store
            .add_message(&conversation.id, "user", "Inspect code")
            .unwrap();
        store
            .interview_summary(&conversation.id, "Inspect source", first.seq)
            .unwrap();
        let newer = store
            .add_message(&conversation.id, "user", "Also docs")
            .unwrap();
        assert!(!store.finish_interview(&conversation.id).unwrap());
        store
            .interview_summary(&conversation.id, "Inspect source and docs", newer.seq)
            .unwrap();
        assert!(store.finish_interview(&conversation.id).unwrap());
        assert!(!store.finish_interview(&conversation.id).unwrap());
        assert_eq!(
            store.workflow(&conversation.id).unwrap().unwrap().phase,
            "planning"
        );
        assert_eq!(
            store
                .messages_by(&conversation.id, "user", 0)
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            store
                .messages_by(&conversation.id, "runtime", 0)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn pre_workflow_sessions_keep_their_existing_flow() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("state.db")).unwrap();
        let conversation = store.create_conversation("project").unwrap();
        assert!(store.workflow(&conversation.id).unwrap().is_none());
    }
}

//! Durable, scoped question/answer exchange and submitted turn results.
use super::*;

impl Store {
    pub(in crate::lean) fn ask_agent_question(
        &self,
        conversation: &str,
        turn: &str,
        key: &str,
        question: &str,
    ) -> Result<String> {
        crate::require(
            !key.trim().is_empty() && key.len() <= 128 && !question.trim().is_empty(),
            "request_key and question are required",
        )?;
        crate::require(question.len() <= 16384, "question exceeds 16 KiB")?;
        let mut db = self.db();
        let tx = db.transaction()?;
        let previous: Option<(String, String)> = tx
            .query_row(
                "SELECT id,question FROM agent_questions WHERE turn_id=?1 AND request_key=?2",
                params![turn, key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((id, text)) = previous {
            crate::require(
                text == question,
                "request_key already used with another question",
            )?;
            return Ok(id);
        }
        let pending:i64=tx.query_row("SELECT count(*) FROM agent_questions q JOIN turns old ON old.id=q.turn_id JOIN turns cur ON cur.id=?1 WHERE old.role=cur.role AND old.conversation_id=cur.conversation_id AND q.state IN ('pending','awaiting_user')",[turn],|r|r.get(0))?;
        crate::require(
            pending < 8,
            "resolve an outstanding question before asking more (limit 8)",
        )?;
        let id = crate::id();
        let role: String =
            tx.query_row("SELECT role FROM turns WHERE id=?1", [turn], |r| r.get(0))?;
        tx.execute("INSERT INTO agent_questions(id,conversation_id,turn_id,request_key,question,state) VALUES(?1,?2,?3,?4,?5,'pending')", params![id,conversation,turn,key,question])?;
        tx.execute("INSERT INTO messages(conversation_id,author,text,created_ms) VALUES(?1,'runtime',?2,?3)",params![conversation,format!("Agent question {id}, source {role}, turn {turn}: {question}\nUse reply_to_agent to answer. An answer does not change the task contract or permissions."),now_ms()])?;
        tx.commit()?;
        Ok(id)
    }

    pub(in crate::lean) fn agent_question(
        &self,
        conversation: &str,
        request: &str,
        turn: Option<&str>,
    ) -> Result<Value> {
        let db = self.db();
        let (owner, state, answer): (String, String, Option<String>) = db.query_row(
            "SELECT turn_id,state,answer FROM agent_questions WHERE conversation_id=?1 AND id=?2",
            params![conversation, request],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).optional()?.ok_or_else(|| Error::Invalid("request: unknown question ID in this conversation; use the request ID returned by ask_orchestrator, not question text".into()))?;
        if let Some(current) = turn
            && current != owner
        {
            let same: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM turns old JOIN turn_scopes os ON os.turn_id=old.id JOIN turn_inputs oi ON oi.turn_id=old.id JOIN turns cur ON cur.id=?2 JOIN turn_scopes cs ON cs.turn_id=cur.id JOIN turn_inputs ci ON ci.turn_id=cur.id WHERE old.id=?1 AND old.role=cur.role AND os.task_id=cs.task_id AND oi.generation=ci.generation)",params![owner,current],|r|r.get(0))?;
            crate::require(
                same,
                "question belongs to another task, role or input version",
            )?;
        }
        Ok(serde_json::json!({"request":request,"state":state,"answer":answer}))
    }

    pub(in crate::lean) fn answer_agent_question(
        &self,
        conversation: &str,
        request: &str,
        answer: &str,
    ) -> Result<Value> {
        crate::require(!answer.trim().is_empty(), "answer is required")?;
        let mut db = self.db();
        let tx = db.transaction()?;
        let (state, previous): (String, Option<String>) = tx.query_row(
            "SELECT state,answer FROM agent_questions WHERE conversation_id=?1 AND id=?2",
            params![conversation, request], |r| Ok((r.get(0)?, r.get(1)?)),
        ).optional()?.ok_or_else(|| Error::Invalid("request: unknown question ID in this conversation; use the request ID returned by ask_orchestrator".into()))?;
        crate::require(
            state != "invalidated",
            "question was invalidated; await a fresh task turn",
        )?;
        let answered = state == "answered";
        if answered && let Some(previous) = previous {
            crate::require(
                previous == answer,
                "question already has a different answer",
            )?;
        } else {
            tx.execute("UPDATE tasks SET status='queued' WHERE id IN (SELECT task_id FROM turn_scopes WHERE turn_id=(SELECT turn_id FROM agent_questions WHERE id=?1)) AND status='waiting' AND json_extract(feedback,'$.waiting_request')=?1",[request])?;
            tx.execute("UPDATE agent_questions SET state='answered',answer=?3 WHERE conversation_id=?1 AND id=?2",params![conversation,request,answer])?;
            tx.execute("INSERT INTO events(conversation_id,kind,payload,created_ms) VALUES(?1,'agent_question_answered',?2,?3)",params![conversation,serde_json::json!({"request":request}).to_string(),now_ms()])?;
        }
        tx.commit()?;
        Ok(serde_json::json!({"request":request,"state":"answered"}))
    }

    pub(in crate::lean) fn save_turn_result(
        &self,
        turn: &str,
        result: &super::super::turn_result::TurnResult,
    ) -> Result<()> {
        let encoded = serde_json::to_string(result)?;
        let mut db = self.db();
        let tx = db.transaction()?;
        let previous: Option<String> = tx
            .query_row(
                "SELECT result FROM agent_results WHERE turn_id=?1",
                [turn],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(previous) = previous {
            crate::require(previous == encoded, "turn already has a different result")?;
        } else {
            tx.execute(
                "INSERT INTO agent_results(turn_id,result) VALUES(?1,?2)",
                params![turn, encoded],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

impl Store {
    pub(in crate::lean) fn local_questions(&self, turn: &str) -> Result<Vec<Value>> {
        let db = self.db();
        let mut query = db.prepare("SELECT q.id,q.state,q.question,q.answer FROM agent_questions q JOIN turns old ON old.id=q.turn_id JOIN turn_scopes os ON os.turn_id=old.id JOIN turn_inputs oi ON oi.turn_id=old.id JOIN turns cur ON cur.id=?1 JOIN turn_scopes cs ON cs.turn_id=cur.id JOIN turn_inputs ci ON ci.turn_id=cur.id WHERE old.role=cur.role AND os.task_id=cs.task_id AND oi.generation=ci.generation ORDER BY (q.state IN ('pending','awaiting_user')) DESC,q.rowid DESC LIMIT 32")?;
        Ok(query.query_map([turn],|r|Ok(serde_json::json!({"request":r.get::<_,String>(0)?,"state":r.get::<_,String>(1)?,"question":r.get::<_,String>(2)?,"answer":r.get::<_,Option<String>>(3)?})))?.collect::<std::result::Result<_,_>>()?)
    }
}

impl Store {
    pub(in crate::lean) fn defer_question_to_user(
        &self,
        conversation: &str,
        request: &str,
        explanation: &str,
    ) -> Result<Value> {
        crate::require(!explanation.trim().is_empty(), "explanation is required")?;
        let changed = self.db().execute("UPDATE agent_questions SET state='awaiting_user',answer=?3 WHERE conversation_id=?1 AND id=?2 AND state IN ('pending','awaiting_user')",params![conversation,request,explanation])?;
        crate::require(
            changed == 1,
            "question is not awaiting a decision; answered or invalidated worker questions cannot be reopened. Ask the user with a fenced question block in your final report instead",
        )?;
        Ok(serde_json::json!({"request":request,"state":"awaiting_user","answer":explanation}))
    }

    pub(in crate::lean) fn park_task_for_question(
        &self,
        task: &Task,
        request: &str,
    ) -> Result<String> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let question: String = tx.query_row("SELECT q.state FROM agent_questions q JOIN turn_scopes s ON s.turn_id=q.turn_id WHERE q.id=?1 AND s.task_id=?2",params![request,task.id],|r|r.get(0)).optional()?.ok_or_else(|| Error::Invalid("request: no question belongs to this task; use the request ID returned by ask_orchestrator".into()))?;
        crate::require(
            matches!(question.as_str(), "pending" | "awaiting_user" | "answered"),
            "question was invalidated",
        )?;
        let state = if question == "answered" {
            "queued"
        } else {
            "waiting"
        };
        let changed = tx.execute("UPDATE tasks SET status=?2 WHERE id=?1 AND coalesce((SELECT generation FROM task_inputs WHERE task_id=?1),0)=?3",params![task.id,state,task.input_generation])?;
        crate::require(changed == 1, "task inputs changed while saving checkpoint")?;
        tx.commit()?;
        Ok(state.into())
    }
}

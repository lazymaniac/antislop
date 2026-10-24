use super::*;

#[path = "history_queries/plan_pages.rs"]
mod plan_pages;

type EncodedTaskRow = (
    String,
    String,
    String,
    String,
    i64,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

impl Store {
    pub fn turn_final_answer(&self, conversation: &str, turn: &str) -> Result<Option<String>> {
        let encoded:Option<String>=self.db().query_row("SELECT payload FROM events WHERE conversation_id=?1 AND turn_id=?2 AND kind='agent_final' ORDER BY seq DESC LIMIT 1",params![conversation,turn],|row|row.get(0)).optional()?;
        encoded
            .map(|encoded| {
                self.hydrate_event_payload("agent_final", &encoded)
                    .map(|payload| payload["text"].as_str().unwrap_or("").to_owned())
            })
            .transpose()
    }

    pub(in crate::lean) fn hydrate_event_payload(
        &self,
        kind: &str,
        encoded: &str,
    ) -> Result<Value> {
        let mut payload: Value = serde_json::from_str(encoded)?;
        for field in ["text", "delta", "report"] {
            if let Some(text) = payload[field].as_str() {
                payload[field] = self.content.decode(text)?.into();
            }
        }
        if kind == "agent" || kind == "agent_final" {
            for field in ["text", "delta"] {
                if let Some(reference) = payload[field].get("session_content") {
                    let reference = serde_json::from_value(reference.clone())?;
                    payload[field] = self.content.read(&reference)?.into();
                }
            }
        }
        Ok(payload)
    }

    /// Reads only one bounded piece of an event's text after checking ownership.
    pub fn session_content(
        &self,
        conversation: &str,
        event: u64,
        field: &str,
        offset: u64,
    ) -> Result<crate::lean::content::Chunk> {
        crate::require(
            matches!(field, "text" | "delta"),
            "invalid session content field",
        )?;
        let encoded: String = self.db().query_row(
            "SELECT payload FROM events WHERE conversation_id=?1 AND seq=?2 AND kind IN ('agent','agent_final')",
            params![conversation, event as i64], |row| row.get(0))?;
        let payload: Value = serde_json::from_str(&encoded)?;
        let reference = payload[field]
            .get("session_content")
            .ok_or_else(|| Error::Invalid("historical event has no content reference".into()))?;
        self.content
            .chunk(&serde_json::from_value(reference.clone())?, offset)
    }

    /// Finds one task within a plan by UUID or the orchestrator's local id.
    pub fn task_in_plan_by_id(&self, plan_id: &str, task_id: &str) -> Result<Option<Task>> {
        // Return just one id, rather than decoding every task's reports and
        // specs. Preserve the same first-in-plan resolution as plan().
        let id: Option<String> = self
            .db()
            .query_row(
                "SELECT id FROM tasks WHERE plan_id=?1 AND (id=?2 OR json_extract(spec, '$.id')=?2)
             ORDER BY position LIMIT 1",
                params![plan_id, task_id],
                |row| row.get(0),
            )
            .optional()?;
        id.map(|id| self.task_by_id(&id))
            .transpose()
            .map(Option::flatten)
    }

    /// Finds a task by its globally unique UUID across all plans.
    pub fn task_by_id(&self, task_id: &str) -> Result<Option<Task>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT id,plan_id,spec,status,round,report,verdict,base,result,feedback FROM tasks
             WHERE id=?1 LIMIT 1",
        )?;
        let task = statement
            .query_row([task_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                ))
            })
            .optional()?;
        task.map(|row| self.decode_task_row(&db, row)).transpose()
    }

    fn decode_task_row(
        &self,
        db: &rusqlite::Connection,
        (id, plan_id, spec, status, round, report, verdict, base, result, feedback): EncodedTaskRow,
    ) -> Result<Task> {
        let input_generation = Self::task_input_generation(db, &id)?;
        Ok(Task {
            input_generation,
            id,
            plan_id,
            spec: serde_json::from_str(&spec)?,
            status,
            round: round as u32,
            report: report
                .as_deref()
                .map(|text| self.content.decode(text))
                .transpose()?,
            verdict: verdict
                .map(|encoded| self.decode_content_json(&encoded))
                .transpose()?,
            base,
            result,
            feedback: feedback
                .map(|encoded| serde_json::from_str(&encoded))
                .transpose()?,
            freshness_token: None,
            checks: None,
        })
    }

    fn task_input_generation(db: &rusqlite::Connection, task_id: &str) -> Result<i64> {
        Ok(db
            .query_row(
                "SELECT generation FROM task_inputs WHERE task_id=?1",
                [task_id],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0))
    }

    fn decode_content_json(&self, encoded: &str) -> Result<Value> {
        Ok(serde_json::from_str(&self.content.decode(encoded)?)?)
    }

    /// Checks if a plan belongs to the conversation.
    pub fn plan_belongs_to_conversation(&self, conversation: &str, plan_id: &str) -> Result<bool> {
        let db = self.db();
        let in_tasks: bool = db
            .query_row(
                "SELECT 1 FROM tasks WHERE plan_id=?1 AND conversation_id=?2 LIMIT 1",
                params![plan_id, conversation],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if in_tasks {
            return Ok(true);
        }
        let in_plans: bool = db
            .query_row(
                "SELECT 1 FROM plans WHERE id=?1 AND conversation_id=?2 LIMIT 1",
                params![plan_id, conversation],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        Ok(in_plans)
    }

    /// Finds the conversation_id for a plan.
    pub fn plan_conversation(&self, plan_id: &str) -> Result<String> {
        let db = self.db();
        let conv: Option<String> = db
            .query_row(
                "SELECT conversation_id FROM tasks WHERE plan_id=?1 LIMIT 1",
                [plan_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(c) = conv {
            return Ok(c);
        }
        let conv2: Option<String> = db
            .query_row(
                "SELECT conversation_id FROM plans WHERE id=?1 LIMIT 1",
                [plan_id],
                |row| row.get(0),
            )
            .optional()?;
        conv2.ok_or_else(|| Error::Invalid(format!("no plan {plan_id} found")))
    }

    /// Checks if a task belongs to a plan (by UUID or spec id).
    pub fn task_in_plan(&self, plan_id: &str, task_id: &str) -> Result<bool> {
        let db = self.db();
        let found: bool = db
            .query_row(
                "SELECT 1 FROM tasks WHERE plan_id=?1 AND (id=?2 OR json_extract(spec, '$.id')=?2) LIMIT 1",
                params![plan_id, task_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        Ok(found)
    }

    /// All comments in the conversation, optionally filtered by status.
    pub fn comments_all(&self, conversation: &str, status: Option<&str>) -> Result<Vec<Comment>> {
        if let Some(st) = status {
            self.comment_rows(
                &self.db(),
                "conversation_id=?1 AND status=?2",
                params![conversation, st],
            )
        } else {
            self.comment_rows(&self.db(), "conversation_id=?1", params![conversation])
        }
    }

    /// Comments for a specific plan in the conversation, optionally filtered by status.
    pub fn comments_by_plan(
        &self,
        conversation: &str,
        plan_id: &str,
        status: Option<&str>,
    ) -> Result<Vec<Comment>> {
        if let Some(st) = status {
            self.comment_rows(
                &self.db(),
                "conversation_id=?1 AND plan_id=?2 AND status=?3",
                params![conversation, plan_id, st],
            )
        } else {
            self.comment_rows(
                &self.db(),
                "conversation_id=?1 AND plan_id=?2",
                params![conversation, plan_id],
            )
        }
    }

    /// Paginated list of all stored plans for a conversation.
    pub fn plans_page(&self, conversation: &str, offset: usize, limit: usize) -> Result<PlansPage> {
        self.conversation(conversation)?;
        let (total, plan_ids, active_plan_id) = self.plan_page_rows(conversation, offset, limit)?;
        let plans = self.load_plan_histories(conversation, &plan_ids)?;
        let has_more = offset + plans.len() < total;
        Ok(PlansPage {
            plans,
            offset,
            limit,
            total,
            has_more,
            active_plan_id,
        })
    }
}

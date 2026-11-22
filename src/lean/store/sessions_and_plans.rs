use super::*;

#[path = "sessions_and_plans/carry.rs"]
pub(in crate::lean::store) mod carry;
#[path = "sessions_and_plans/recovery.rs"]
mod recovery;

impl Store {
    /// The last user message a successful turn of `role` answered.
    pub fn last_answered(&self, conversation: &str, role: &str) -> Result<u64> {
        Ok(self.db().query_row(
            "SELECT COALESCE(MAX(upto_message),0) FROM turns
             WHERE conversation_id=?1 AND role=?2 AND status='done'",
            params![conversation, role],
            |row| row.get::<_, i64>(0),
        )? as u64)
    }

    pub fn last_orchestration_answered(&self, conversation: &str) -> Result<u64> {
        let mut answered = 0;
        for role in crate::lean::ORCHESTRATION_ROLES {
            answered = answered.max(self.last_answered(conversation, role)?);
        }
        Ok(answered)
    }

    /// Forgets the CLI sessions of a task’s worker and reviewer turns.
    pub fn forget_task_sessions(&self, task: &str) -> Result<()> {
        self.db().execute(
            "UPDATE turns SET session_id=NULL WHERE session_id IS NOT NULL AND id IN
             (SELECT turn_id FROM turn_scopes WHERE task_id=?1)",
            [task],
        )?;
        Ok(())
    }

    /// When a turn other than `current` last used `session`.
    pub fn session_last_used_ms(
        &self,
        conversation: &str,
        session: &str,
        current: &str,
    ) -> Result<Option<i64>> {
        Ok(self.db().query_row(
            "SELECT MAX(started_ms) FROM turns WHERE conversation_id=?1 AND session_id=?2 AND id<>?3",
            params![conversation, session, current],
            |row| row.get(0),
        )?)
    }

    /// The session of the latest turn of `role` with `provider` that got one,
    /// even if that turn later failed: the CLI keeps what it already saw.
    /// Another CLI's session id would mean nothing to it.
    pub fn last_session(
        &self,
        conversation: &str,
        role: &str,
        provider: &str,
    ) -> Result<Option<String>> {
        Ok(self
            .db()
            .query_row(
                "SELECT t.session_id FROM turns t WHERE t.conversation_id=?1 AND t.role=?2 AND t.provider=?3
                 AND t.session_id IS NOT NULL
                 AND NOT EXISTS (
                    SELECT 1 FROM events e JOIN turns retired ON retired.id=e.turn_id
                    WHERE e.conversation_id=t.conversation_id AND e.kind='provider_session_lost'
                      AND retired.role=t.role AND retired.provider=t.provider
                      AND (retired.started_ms>t.started_ms
                        OR (retired.started_ms=t.started_ms AND retired.rowid>t.rowid))
                 ) ORDER BY t.started_ms DESC, t.rowid DESC LIMIT 1",
                params![conversation, role, provider],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Retires a provider session that the CLI explicitly rejected. The event
    /// prevents older IDs for this role/provider from becoming active again.
    pub fn retire_session(&self, turn: &str, reason: &str) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let (conversation, role, provider): (String, String, String) = tx.query_row(
            "SELECT conversation_id,role,provider FROM turns WHERE id=?1",
            [turn],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        tx.execute("UPDATE turns SET session_id=NULL WHERE id=?1", [turn])?;
        tx.execute(
            "UPDATE run_contexts SET provider_thread_id=NULL,provider_session_id=NULL WHERE turn_id=?1",
            [turn],
        )?;
        tx.execute(
            "INSERT INTO events(conversation_id,turn_id,kind,payload,created_ms)
             VALUES(?1,?2,'provider_session_lost',?3,?4)",
            params![
                conversation,
                turn,
                serde_json::json!({"role":role,"provider":provider,"reason":reason}).to_string(),
                now_ms()
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Turns left running by a daemon that stopped become interrupted; nothing
    /// else changes and nothing waits for a manual reconciliation.
    /// Returns the conversations whose orchestrator was cut off, so its
    /// unanswered messages can be taken up again.
    pub fn interrupt_running_turns(&self) -> Result<Vec<String>> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let conversations = recovery::running_orchestrators(&tx)?;
        recovery::record_turn_interruptions(&tx)?;
        recovery::close_open_approvals(&tx)?;
        tx.commit()?;
        Ok(conversations)
    }

    /// Running tasks become queued with an interruption reason; queued
    /// checkpoints remain unchanged. Returns candidate (conversation, plan)
    /// pairs; callers must check dependencies and deferral before resuming.
    pub fn interrupt_unfinished_tasks(&self) -> Result<Vec<(String, String)>> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let plans = recovery::unfinished_plans(&tx)?;
        self.mark_tasks_interrupted(&tx)?;
        tx.commit()?;
        Ok(plans)
    }

    /// Queues again every task of a plan that ended without approval, keeping
    /// its rounds, report, verdict and work. Returns how many.
    pub fn requeue_plan(&self, conversation: &str, plan_id: &str) -> Result<usize> {
        self.refresh_answer_waits(conversation)?;
        Ok(self.db().execute(
            "UPDATE tasks SET status='queued' WHERE conversation_id=?1 AND plan_id=?2
               AND (status IN ('coordinating','cancelled','failed','needs_user','skipped','waiting','queued','working','reviewing'))
               AND NOT EXISTS(SELECT 1 FROM agent_questions q WHERE q.id=json_extract(tasks.feedback,'$.waiting_request') AND q.state IN ('pending','awaiting_user'))
               AND NOT EXISTS(SELECT 1 FROM agent_answer_waits w WHERE w.request=json_extract(tasks.feedback,'$.waiting_request') AND w.released=0)",
            params![conversation, plan_id],
        )?)
    }

    /// After a restart, queues again the tasks of a plan the daemon was
    /// running that only waited for it: those it interrupted are queued
    /// already, and a task waiting for a CLI login or usage limit tries
    /// again. A task waiting for an answer to its question, or stopped for
    /// the orchestrator or the user, stays as it was. Returns how many.
    pub fn requeue_after_restart(&self, conversation: &str, plan_id: &str) -> Result<usize> {
        Ok(self.db().execute(
            "UPDATE tasks SET status='queued' WHERE conversation_id=?1 AND plan_id=?2 AND status='waiting'
               AND NOT EXISTS (SELECT 1 FROM agent_questions q
                 WHERE q.id=json_extract(tasks.feedback,'$.waiting_request') AND q.state IN ('pending','awaiting_user'))
               AND NOT EXISTS(SELECT 1 FROM agent_answer_waits w WHERE w.request=json_extract(tasks.feedback,'$.waiting_request') AND w.released=0)",
            params![conversation, plan_id],
        )?)
    }

    /// The conversation's most recently accepted plan, the one its work runs on.
    pub fn accepted_plan(&self, conversation: &str) -> Result<Option<String>> {
        Ok(self
            .db()
            .query_row(
                "SELECT p.id FROM plans p JOIN plan_activations a ON a.plan_id=p.id
                 WHERE p.conversation_id=?1 ORDER BY p.created_ms DESC,p.rowid DESC LIMIT 1",
                [conversation],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn plan_review_approved(&self, conversation: &str, plan_id: &str) -> Result<bool> {
        let reviewed: Option<bool> = self
            .db()
            .query_row(
                "SELECT json_extract(payload,'$.review.approved') FROM events
                 WHERE conversation_id=?1 AND plan_id=?2 AND kind='plan_proposed'
                 ORDER BY seq DESC LIMIT 1",
                params![conversation, plan_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(reviewed == Some(true))
    }

    /// Queues every task of a proposed plan, all or none; tasks approved in
    /// the previous plan with the same contract stay approved. A repeated
    /// activation is rejected without writes; resumption uses `requeue_plan`.
    /// Runtime callers retain the integration lock through source revalidation
    /// and publication. External editors are observed, not locked, by snapshots.
    pub fn queue_plan(&self, conversation: &str, plan_id: &str) -> Result<usize> {
        let (all, captured) = {
            let mut db = self.db();
            let tx = db.transaction()?;
            let all = proposed_task_count(&tx, conversation, plan_id)?;
            let captured = carry::capture(&tx, conversation, plan_id)?;
            tx.commit()?;
            (all, captured)
        };
        let prepared = captured.compare_sources()?;
        prepared.validate_sources()?;
        let mut db = self.db();
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        crate::require(
            carry::capture(&tx, conversation, plan_id)? == captured,
            "carry inputs changed during preparation; approve the plan again",
        )?;
        proposed_task_count(&tx, conversation, plan_id)?;
        carry::execute(
            &tx,
            "UPDATE tasks SET status='queued' WHERE conversation_id=?1 AND plan_id=?2 AND status<>'superseded'",
            params![conversation, plan_id],
        )?;
        if let Some(text) = carry::apply(&tx, &prepared)? {
            carry::execute(
                &tx,
                "INSERT INTO events(conversation_id,plan_id,kind,payload,created_ms) VALUES(?1,?2,'notice',?3,?4)",
                params![
                    conversation,
                    plan_id,
                    serde_json::json!({"text": text}).to_string(),
                    now_ms()
                ],
            )?;
        }
        record_plan_activation(&tx, conversation, plan_id)?;
        advance_workflow(&tx, conversation, plan_id)?;
        tx.commit()?;
        Ok(all as usize)
    }

    /// Moves every task of a plan that is still `from` to `to`.
    pub fn set_plan_status(&self, plan_id: &str, from: &str, to: &str) -> Result<usize> {
        Ok(self.db().execute(
            "UPDATE tasks SET status=?3 WHERE plan_id=?1 AND status=?2",
            params![plan_id, from, to],
        )?)
    }

    pub fn add_plan(&self, conversation: &str, specs: &[Value]) -> Result<Vec<Task>> {
        let plan_id = crate::id();
        let mut db = self.db();
        let tx = db.transaction()?;
        let now = now_ms();
        tx.execute(
            "INSERT INTO plans(id,conversation_id,created_ms) VALUES(?1,?2,?3)",
            params![plan_id, conversation, now],
        )?;
        tx.execute("INSERT INTO plan_heads(plan_id) VALUES(?1)", [&plan_id])?;
        let tasks = recovery::insert_proposed_tasks(&tx, conversation, &plan_id, specs, now)?;
        for task in &tasks {
            tx.execute("INSERT INTO task_inputs(task_id) VALUES(?1)", [&task.id])?;
            tx.execute(
                "INSERT INTO task_contracts(task_id,version,spec) VALUES(?1,0,?2)",
                params![task.id, task.spec.to_string()],
            )?;
        }
        tx.commit()?;
        Ok(tasks)
    }

    /// Tasks of the conversation's latest plan, in the orchestrator's order.
    pub fn latest_plan(&self, conversation: &str) -> Result<Vec<Task>> {
        let latest: Option<String> = self
            .db()
            .query_row(
                "SELECT plan_id FROM tasks WHERE conversation_id=?1
                 ORDER BY created_ms DESC, rowid DESC LIMIT 1",
                [conversation],
                |row| row.get(0),
            )
            .optional()?;
        match latest {
            Some(plan_id) => self.plan(&plan_id),
            None => Ok(vec![]),
        }
    }
}

fn proposed_task_count(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    plan_id: &str,
) -> Result<i64> {
    let proposed: i64 = carry::one(
        tx,
        "SELECT COUNT(*) FROM tasks WHERE conversation_id=?1 AND plan_id=?2 AND status='proposed'",
        params![conversation, plan_id],
        |row| row.get(0),
    )?;
    let all: i64 = carry::one(
        tx,
        "SELECT COUNT(*) FROM tasks WHERE conversation_id=?1 AND plan_id=?2 AND status<>'superseded'",
        params![conversation, plan_id],
        |row| row.get(0),
    )?;
    crate::require(
        all > 0 && proposed == all,
        "this plan is not waiting for approval",
    )?;
    Ok(all)
}

fn record_plan_activation(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    plan_id: &str,
) -> Result<()> {
    // Activation records the reviewed plan used by scheduling and recovery.
    carry::execute(
        tx,
        "INSERT INTO plan_activations(plan_id,conversation_id) VALUES(?1,?2)",
        params![plan_id, conversation],
    )?;
    Ok(())
}

fn advance_workflow(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    plan_id: &str,
) -> Result<()> {
    let changed = carry::execute(
        tx,
        "UPDATE session_workflow SET phase='implementation' WHERE conversation_id=?1 AND phase<>'interview'",
        [conversation],
    )?;
    if changed > 0 {
        carry::execute(
            tx,
            "INSERT INTO events(conversation_id,plan_id,kind,payload,created_ms) VALUES(?1,?2,'workflow',?3,?4)",
            params![
                conversation,
                plan_id,
                serde_json::json!({"phase":"implementation"}).to_string(),
                now_ms()
            ],
        )?;
    }
    Ok(())
}

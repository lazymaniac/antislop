use super::*;
use crate::lean::tasks::TaskContract;

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

    /// Starts subsequent orchestration turns without resuming the old sessions.
    pub fn forget_orchestrator_sessions(&self, conversation: &str) -> Result<()> {
        self.db().execute(
            "UPDATE turns SET session_id=NULL WHERE conversation_id=?1 AND role IN (?2,?3,?4)",
            params![
                conversation,
                crate::lean::ORCHESTRATION_ROLES[0],
                crate::lean::ORCHESTRATION_ROLES[1],
                crate::lean::ORCHESTRATION_ROLES[2]
            ],
        )?;
        Ok(())
    }

    /// Forgets the CLI sessions of a task’s worker and verifier turns.
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
                "SELECT session_id FROM turns WHERE conversation_id=?1 AND role=?2 AND provider=?3
                 AND session_id IS NOT NULL ORDER BY started_ms DESC, rowid DESC LIMIT 1",
                params![conversation, role, provider],
                |row| row.get(0),
            )
            .optional()?)
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
        tx.commit()?;
        Ok(conversations)
    }

    /// Unfinished tasks stay queued with the interruption reason. Returns each
    /// affected (conversation, plan), so the plan can be resumed.
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
        Ok(self.db().execute(
            "UPDATE tasks SET status='queued' WHERE conversation_id=?1 AND plan_id=?2
               AND (status IN ('coordinating','cancelled','failed','needs_user','skipped','waiting','queued','working','verifying'))",
            params![conversation, plan_id],
        )?)
    }

    /// Queues every task of a proposed plan, all or none.
    pub fn queue_plan(&self, conversation: &str, plan_id: &str) -> Result<usize> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let proposed: i64 = tx.query_row(
            "SELECT COUNT(*) FROM tasks WHERE conversation_id=?1 AND plan_id=?2 AND status='proposed'",
            params![conversation, plan_id],
            |row| row.get(0),
        )?;
        let all: i64 = tx.query_row(
            "SELECT COUNT(*) FROM tasks WHERE conversation_id=?1 AND plan_id=?2",
            params![conversation, plan_id],
            |row| row.get(0),
        )?;
        crate::require(
            all > 0 && proposed == all,
            "this plan is not waiting for approval",
        )?;
        tx.execute(
            "UPDATE tasks SET status='queued' WHERE conversation_id=?1 AND plan_id=?2",
            params![conversation, plan_id],
        )?;
        authorize_plan(&tx, conversation, plan_id)?;
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

fn authorize_plan(tx: &rusqlite::Transaction<'_>, conversation: &str, plan_id: &str) -> Result<()> {
    let contracts = plan_contracts(tx, plan_id)?;
    let scope = plan_scope(&contracts, false);
    let write_scope = plan_scope(&contracts, true);
    tx.execute(
        "INSERT INTO plan_authorizations(plan_id,conversation_id,scope,write_scope) VALUES(?1,?2,?3,?4)",
        params![plan_id, conversation, serde_json::to_string(&scope)?, serde_json::to_string(&write_scope)?],
    )?;
    Ok(())
}

fn plan_contracts(tx: &rusqlite::Transaction<'_>, plan: &str) -> Result<Vec<TaskContract>> {
    tx.prepare("SELECT spec FROM tasks WHERE plan_id=?1")?
        .query_map([plan], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?
        .iter()
        .map(|spec| serde_json::from_str(spec).map_err(Into::into))
        .collect()
}

fn plan_scope(contracts: &[TaskContract], writers_only: bool) -> Vec<String> {
    let mut paths = std::collections::BTreeSet::new();
    for task in contracts.iter().filter(|task| !writers_only || task.writes) {
        if task.scope.is_empty() {
            paths.insert(".".to_owned());
        }
        paths.extend(task.scope.iter().cloned());
    }
    paths.into_iter().collect()
}

fn advance_workflow(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    plan_id: &str,
) -> Result<()> {
    let changed = tx.execute(
        "UPDATE session_workflow SET phase='implementation' WHERE conversation_id=?1 AND phase<>'interview'",
        [conversation],
    )?;
    if changed > 0 {
        tx.execute(
            "INSERT INTO events(conversation_id,plan_id,kind,payload,created_ms) VALUES(?1,?2,'workflow',?3,?4)",
            params![conversation, plan_id, serde_json::json!({"phase":"implementation"}).to_string(), now_ms()],
        )?;
    }
    Ok(())
}

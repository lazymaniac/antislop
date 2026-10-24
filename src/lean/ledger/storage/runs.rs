use super::*;

impl Store {
    pub fn create_run(&self, conversation: &str, goal: &str) -> Result<Run> {
        self.conversation(conversation)?;
        require(!goal.trim().is_empty(), "run goal is empty")?;
        let r = Run {
            id: crate::id(),
            conversation_id: conversation.into(),
            goal: goal.into(),
            spec_revision: 0,
            state: "active".into(),
            start_message: {
                let db = self.db();
                let exists: bool = db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM runs WHERE conversation_id=?1)",
                    [conversation],
                    |r| r.get(0),
                )?;
                db.query_row(if exists {"SELECT coalesce(max(seq),0) FROM messages WHERE conversation_id=?1 AND author='user'"} else {"SELECT coalesce(min(seq),0) FROM messages WHERE conversation_id=?1 AND author='user'"},[conversation],|r|r.get::<_,i64>(0))? as u64
            },
        };
        let mut db = self.db();
        let tx = db.transaction()?;
        tx.execute(
                "INSERT INTO runs(id,conversation_id,goal,created_ms,start_message) VALUES(?1,?2,?3,?4,?5)",
                params![r.id, conversation, goal, now_ms(),r.start_message as i64],
            )?;
        event(
            &tx,
            conversation,
            "run_created",
            &serde_json::json!({"run":r.id}),
        )?;
        tx.commit()?;
        Ok(r)
    }
    pub fn run(&self, conversation: &str, id: &str) -> Result<Run> {
        run(&self.db(), conversation, id)
    }
    pub fn planning_run(&self, conversation: &str) -> Result<Option<Run>> {
        let db = self.db();
        let id: Option<String> = db
            .query_row(
                "SELECT run_id FROM planning_runs WHERE conversation_id=?1",
                [conversation],
                |row| row.get(0),
            )
            .optional()?;
        let id = match id {
                Some(id) => Some(id),
                None => db.query_row(
                    "SELECT r.id FROM runs r WHERE r.conversation_id=?1 ORDER BY EXISTS(SELECT 1 FROM ledger_sources s WHERE s.run_id=r.id) DESC,r.created_ms DESC,r.rowid DESC LIMIT 1",
                    [conversation], |row| row.get(0)
                ).optional()?,
            };
        id.map(|id| run(&db, conversation, &id)).transpose()
    }
    pub fn select_planning_run(&self, conversation: &str, run_id: &str) -> Result<()> {
        let db = self.db();
        run(&db, conversation, run_id)?;
        db.execute("INSERT INTO planning_runs VALUES(?1,?2) ON CONFLICT(conversation_id) DO UPDATE SET run_id=excluded.run_id", params![conversation,run_id])?;
        Ok(())
    }
    pub fn approved_requirements_producer(
        &self,
        conversation: &str,
        run_id: &str,
        revision: i64,
    ) -> Result<Option<String>> {
        self.run(conversation, run_id)?;
        Ok(self.db().query_row(
                "SELECT substr(p.author,6) FROM ledger_proposals p JOIN ledger_decisions d ON d.operation_id=json_extract(p.request,'$.operation_id') JOIN requirements_resumptions x ON x.operation_id=d.operation_id WHERE p.run_id=?1 AND d.spec_revision=?2 AND d.author='user' AND p.author LIKE 'turn:%' AND NOT EXISTS(SELECT 1 FROM ledger_reviews v WHERE v.run_id=p.run_id AND v.spec_revision=?2) ORDER BY d.created_ms DESC,d.rowid DESC LIMIT 1",
                params![run_id,revision],|row|row.get(0)
            ).optional()?)
    }
    pub fn has_pending_requirements_approval(&self, conversation: &str) -> Result<bool> {
        Ok(self.db().query_row(
                "SELECT EXISTS(SELECT 1 FROM messages WHERE conversation_id=?1 AND author IN ('requirements-approval','options-planner','options-plan-repair','plan-format-repair') AND seq>COALESCE((SELECT MAX(upto_message) FROM turns WHERE conversation_id=?1 AND role IN ('orchestrator','comment-orchestrator') AND status<>'interrupted'),0))",
                [conversation], |row| row.get(0)
            )?)
    }
    pub fn recover_approved_requirements(&self) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let changes = tx.prepare(
                "SELECT p.request FROM ledger_proposals p JOIN ledger_decisions d ON d.operation_id=json_extract(p.request,'$.operation_id') JOIN runs r ON r.id=p.run_id JOIN session_workflow w ON w.conversation_id=r.conversation_id WHERE p.author LIKE 'turn:%' AND d.author='user' AND d.spec_revision=r.spec_revision AND w.phase='planning' AND NOT EXISTS(SELECT 1 FROM requirements_resumptions x WHERE x.operation_id=d.operation_id) AND NOT EXISTS(SELECT 1 FROM ledger_reviews v WHERE v.run_id=r.id AND v.spec_revision=r.spec_revision) AND NOT EXISTS(SELECT 1 FROM runs newer WHERE newer.conversation_id=r.conversation_id AND (newer.created_ms>r.created_ms OR (newer.created_ms=r.created_ms AND newer.rowid>r.rowid)) AND EXISTS(SELECT 1 FROM ledger_sources s WHERE s.run_id=newer.id))"
            )?.query_map([], |row| row.get::<_,String>(0))?.collect::<std::result::Result<Vec<_>,_>>()?;
        for encoded in changes {
            let change: LedgerChange = serde_json::from_str(&encoded)?;
            enqueue_requirements_resume(&tx, &change)?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn latest_run(&self, conversation: &str) -> Result<Option<Run>> {
        let id:Option<String>=self.db().query_row("SELECT id FROM runs WHERE conversation_id=?1 ORDER BY created_ms DESC,rowid DESC LIMIT 1",[conversation],|r|r.get(0)).optional()?;
        id.map(|id| self.run(conversation, &id)).transpose()
    }
    pub fn plan_run(&self, plan: &str) -> Result<Option<String>> {
        Ok(self
            .db()
            .query_row(
                "SELECT run_id FROM plan_runs WHERE plan_id=?1",
                [plan],
                |r| r.get(0),
            )
            .optional()?)
    }
    pub fn link_plan_run(&self, conversation: &str, plan: &str, run_id: &str) -> Result<()> {
        self.run(conversation, run_id)?;
        require(
            self.plan(plan)?
                .iter()
                .all(|t| self.task_in_plan(&t.plan_id, &t.id).unwrap_or(false)),
            "unknown plan",
        )?;
        let db = self.db();
        let owner: String = db.query_row(
            "SELECT conversation_id FROM plans WHERE id=?1",
            [plan],
            |r| r.get(0),
        )?;
        require(
            owner == conversation,
            "plan belongs to another conversation",
        )?;
        db.execute(
            "INSERT INTO plan_runs(plan_id,run_id) VALUES(?1,?2)",
            params![plan, run_id],
        )?;
        Ok(())
    }
}

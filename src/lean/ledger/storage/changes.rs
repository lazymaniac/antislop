use super::validation::{impact, validate_change};
use super::*;

impl Store {
    pub fn ledger_entries(
        &self,
        conversation: &str,
        run_id: &str,
        revision: Option<i64>,
    ) -> Result<Vec<LedgerEntry>> {
        let db = self.db();
        let r = run(&db, conversation, run_id)?;
        let revision = revision.unwrap_or(r.spec_revision);
        require(revision <= r.spec_revision, "unknown spec revision")?;
        entries(&db, run_id, revision)
    }
    pub fn preview_ledger_change(
        &self,
        change: &LedgerChange,
        state: &std::path::Path,
    ) -> Result<LedgerDiff> {
        validate_change(&self.db(), change, state, true)
    }
    pub fn propose_ledger_change(
        &self,
        change: &LedgerChange,
        state: &std::path::Path,
    ) -> Result<(String, LedgerDiff)> {
        self.propose_ledger_change_as(change, state, "user")
    }
    pub fn propose_agent_ledger_change(
        &self,
        change: &LedgerChange,
        state: &std::path::Path,
        turn: &str,
    ) -> Result<(String, LedgerDiff)> {
        self.check_analyst_author(change, turn)?;
        self.propose_ledger_change_as(change, state, &format!("turn:{turn}"))
    }
    fn propose_ledger_change_as(
        &self,
        change: &LedgerChange,
        state: &std::path::Path,
        author: &str,
    ) -> Result<(String, LedgerDiff)> {
        let mut db = self.db();
        let tx = db.transaction()?;
        if let Some((id,request))=tx.query_row("SELECT id,request FROM ledger_proposals WHERE json_extract(request,'$.operation_id')=?1",[&change.operation_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()? {
                require(serde_json::from_str::<LedgerChange>(&request)?==*change,"operation identity reused with a different proposal")?;
                return Ok((id.clone(),saved_proposal_diff(&tx,&id,change)?));
            }
        let diff = validate_change(&tx, change, state, true)?;
        let id = crate::id();
        let hash = super::diff_hash(&diff)?;
        tx.execute("INSERT INTO ledger_proposals(id,run_id,request,diff_hash,created_ms,author) VALUES(?1,?2,?3,?4,?5,?6)",params![id,change.run_id,serde_json::to_string(change)?,hash,now_ms(),author])?;
        for task in &diff.affected_tasks {
            tx.execute(
                "INSERT INTO ledger_proposal_tasks VALUES(?1,?2)",
                params![id, task],
            )?;
        }
        event(
            &tx,
            &change.conversation,
            "ledger_change_proposed",
            &serde_json::json!({"proposal":id,"run":change.run_id,"revision":change.expected_spec_revision,"author":author}),
        )?;
        tx.commit()?;
        Ok((id, diff))
    }
    pub fn apply_ledger_change(
        &self,
        change: &LedgerChange,
        state: &std::path::Path,
        user: bool,
    ) -> Result<ChangeResult> {
        require(
            user,
            "agent changes require a runtime-bound analyst session",
        )?;
        self.apply_ledger_change_as(change, state, true, "user")
    }
    pub fn apply_analyst_ledger_change(
        &self,
        change: &LedgerChange,
        state: &std::path::Path,
        turn: &str,
    ) -> Result<ChangeResult> {
        self.check_analyst_author(change, turn)?;
        self.apply_ledger_change_as(change, state, false, &format!("turn:{turn}"))
    }
    fn apply_ledger_change_as(
        &self,
        change: &LedgerChange,
        state: &std::path::Path,
        user: bool,
        author: &str,
    ) -> Result<ChangeResult> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let request_hash = crate::hash(serde_json::to_vec(change)?);
        if let Some((hash, result)) = tx
            .query_row(
                "SELECT request_hash,result FROM ledger_decisions WHERE operation_id=?1",
                [&change.operation_id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?
        {
            require(
                hash == request_hash,
                "operation identity reused with a different payload",
            )?;
            return Ok(serde_json::from_str(&result)?);
        }
        let diff = validate_change(&tx, change, state, user)?;
        let mut changed_versions = Vec::new();
        for e in &change.entries {
            let text = source_text(&tx, &change.run_id, &e.source.source_id, state)?;
            e.source.text(&text)?;
            // The immutable artifact/message owns the source bytes. Version rows
            // retain the span, never duplicate a repository document.
            let original = "";
            tx.execute(
                "INSERT OR IGNORE INTO ledger_entries(run_id,id) VALUES(?1,?2)",
                params![change.run_id, e.id],
            )?;
            let version:i64=tx.query_row("SELECT coalesce(max(version),0)+1 FROM ledger_versions WHERE run_id=?1 AND entry_id=?2",params![change.run_id,e.id],|r|r.get(0))?;
            changed_versions.push(serde_json::json!({"id":e.id,"version":version}));
            let disposition = serde_json::to_value(e.disposition)?
                .as_str()
                .unwrap()
                .to_owned();
            tx.execute(
                "INSERT INTO ledger_versions VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                params![
                    change.run_id,
                    e.id,
                    version,
                    diff.to_revision,
                    e.source.source_id,
                    e.source.start as i64,
                    e.source.end as i64,
                    original,
                    e.interpretation,
                    e.mandatory,
                    e.position,
                    disposition
                ],
            )?;
            for c in &e.criteria {
                tx.execute(
                    "INSERT INTO ledger_criteria VALUES(?1,?2,?3,?4,?5,?6,?7)",
                    params![
                        change.run_id,
                        e.id,
                        version,
                        c.id,
                        c.text,
                        c.mandatory,
                        c.integration
                    ],
                )?;
            }
            for (kind, values) in [
                ("parent", e.parent.iter().cloned().collect()),
                ("dependency", e.depends_on.clone()),
                ("task", e.task_ids.clone()),
                ("qualifier", e.qualifiers.clone()),
                ("implementer", e.implementation_owners.clone()),
                ("verifier", e.verification_owners.clone()),
            ] {
                for value in values {
                    tx.execute(
                        "INSERT INTO ledger_relations VALUES(?1,?2,?3,?4,?5)",
                        params![change.run_id, e.id, version, kind, value],
                    )?;
                }
            }
        }
        for task in &diff.affected_tasks {
            tx.execute(
                "UPDATE task_inputs SET generation=generation+1 WHERE task_id=?1",
                [task],
            )?;
            tx.execute("UPDATE tasks SET status='stale' WHERE id=?1", [task])?;
        }
        tx.execute(
            "UPDATE runs SET spec_revision=?2,state='active' WHERE id=?1",
            params![change.run_id, diff.to_revision],
        )?;
        let result = ChangeResult {
            run_id: change.run_id.clone(),
            spec_revision: diff.to_revision,
            affected_tasks: diff.affected_tasks,
        };
        tx.execute(
            "INSERT INTO ledger_decisions VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                change.operation_id,
                change.run_id,
                result.spec_revision,
                author,
                change.reason,
                request_hash,
                serde_json::to_string(&result)?,
                now_ms()
            ],
        )?;
        event(
            &tx,
            &change.conversation,
            "ledger_changed",
            &serde_json::json!({"run":result.run_id,"revision":result.spec_revision,"entries":changed_versions,"affected_tasks":result.affected_tasks}),
        )?;
        if user {
            let analyst_proposal: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM ledger_proposals WHERE run_id=?1 AND json_extract(request,'$.operation_id')=?2 AND author LIKE 'turn:%')",
                    params![change.run_id,change.operation_id], |row| row.get(0)
                )?;
            if analyst_proposal {
                enqueue_requirements_resume(&tx, change)?;
            }
        }
        tx.commit()?;
        Ok(result)
    }
    pub fn ledger_diff(
        &self,
        conversation: &str,
        run_id: &str,
        from: i64,
        to: i64,
    ) -> Result<LedgerDiff> {
        let before = self.ledger_entries(conversation, run_id, Some(from))?;
        let after = self.ledger_entries(conversation, run_id, Some(to))?;
        let changed: HashSet<_> = after
            .iter()
            .filter(|e| {
                !before
                    .iter()
                    .any(|b| b.spec.id == e.spec.id && b.version == e.version)
            })
            .map(|e| e.spec.id.clone())
            .collect();
        let all: Vec<_> = after.iter().map(|e| e.spec.clone()).collect();
        Ok(LedgerDiff {
            run_id: run_id.into(),
            from_revision: from,
            to_revision: to,
            before: before
                .into_iter()
                .filter(|e| changed.contains(&e.spec.id))
                .collect(),
            after: after
                .into_iter()
                .filter(|e| changed.contains(&e.spec.id))
                .map(|e| e.spec)
                .collect(),
            affected_tasks: impact(&self.db(), run_id, &changed, &all)?,
        })
    }
}

use super::*;

impl Store {
    /// Persist the decision and work queue together. The store mutex serializes
    /// this transaction with acceptance, including direct store callers.
    pub fn reject_ledger_proposal(
        &self,
        conversation: &str,
        run_id: &str,
        id: &str,
        hash: &str,
        comment: &str,
    ) -> Result<()> {
        require(!comment.trim().is_empty(), "rejection requires a comment")?;
        let mut db = self.db();
        let tx = db.transaction()?;
        let current = run(&tx, conversation, run_id)?;
        let (request, expected): (String, String) = tx.query_row(
            "SELECT request,diff_hash FROM ledger_proposals WHERE id=?1 AND run_id=?2",
            params![id, run_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        require(
            expected == hash,
            "rejection must identify the exact proposed ledger diff",
        )?;
        let change: LedgerChange = serde_json::from_str(&request)?;
        require(
            change.conversation == conversation,
            "proposal belongs to another conversation",
        )?;
        let previous: Option<String> = tx
            .query_row(
                "SELECT m.text FROM ledger_rejections x JOIN messages m ON m.seq=x.comment_message WHERE x.proposal_id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(previous) = previous {
            require(
                self.decode_content(&previous)? == comment,
                "proposal already rejected with another comment",
            )?;
            return Ok(());
        }
        require(
            current.spec_revision == change.expected_spec_revision,
            "scope changed before rejection",
        )?;
        let accepted: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM ledger_decisions WHERE operation_id=?1)",
            [&change.operation_id],
            |r| r.get(0),
        )?;
        require(!accepted, "proposal already accepted")?;
        require(
            super::diff_hash(&saved_proposal_diff(&tx, id, &change)?)? == hash,
            "proposal impact changed; reload the proposal",
        )?;
        // The user's correction is an immutable source on the next extraction.
        tx.execute(
            "INSERT INTO messages(conversation_id,author,text,created_ms) VALUES(?1,'user',?2,?3)",
            params![conversation, comment, now_ms()],
        )?;
        let message = tx.last_insert_rowid();
        event(
            &tx,
            conversation,
            "message",
            &serde_json::json!({"seq":message,"author":"user","text":comment}),
        )?;
        let text = format!(
            "The user rejected requirements proposal {id} for run {run_id}. User feedback on that rejected proposal:\n{comment}\nPrepare a revised proposal for explicit user approval. Rejection does not authorize applying a correction."
        );
        let resume = insert_resume_message(&tx, &change, &text)?;
        tx.execute(
            "INSERT INTO ledger_rejections VALUES(?1,?2,?3,?4,?5)",
            params![id, message, resume, hash, now_ms()],
        )?;
        tx.execute("INSERT INTO planning_runs VALUES(?1,?2) ON CONFLICT(conversation_id) DO UPDATE SET run_id=excluded.run_id", params![conversation,run_id])?;
        event(
            &tx,
            conversation,
            "ledger_proposal_rejected",
            &serde_json::json!({"run":run_id,"proposal":id,"comment_message":message}),
        )?;
        event(
            &tx,
            conversation,
            "message",
            &serde_json::json!({"seq":resume,"author":"requirements-approval","text":text}),
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn has_pending_ledger_proposal(&self, run_id: &str) -> Result<bool> {
        Ok(self.db().query_row("SELECT EXISTS(SELECT 1 FROM ledger_proposals p JOIN runs r ON r.id=p.run_id WHERE p.run_id=?1 AND p.author LIKE 'turn:%' AND json_extract(p.request,'$.expected_spec_revision')=r.spec_revision AND NOT EXISTS(SELECT 1 FROM ledger_rejections x WHERE x.proposal_id=p.id) AND NOT EXISTS(SELECT 1 FROM ledger_decisions d WHERE d.operation_id=json_extract(p.request,'$.operation_id')))", [run_id], |r|r.get(0))?)
    }
    pub fn has_rejected_ledger_proposal(&self, run_id: &str) -> Result<bool> {
        Ok(self.db().query_row("SELECT EXISTS(SELECT 1 FROM ledger_rejections x JOIN ledger_proposals p ON p.id=x.proposal_id WHERE p.run_id=?1)", [run_id], |r|r.get(0))?)
    }
    pub fn check_ledger_proposal(&self, id: &str, hash: &str, change: &LedgerChange) -> Result<()> {
        let db = self.db();
        let (request, expected): (String, String) = db.query_row(
            "SELECT request,diff_hash FROM ledger_proposals WHERE id=?1 AND run_id=?2",
            params![id, change.run_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        require(
            expected == hash && serde_json::from_str::<LedgerChange>(&request)? == *change,
            "approval must identify the exact proposed ledger diff",
        )
    }
    pub fn ledger_entry_history(
        &self,
        conversation: &str,
        run_id: &str,
        id: &str,
    ) -> Result<Vec<LedgerEntry>> {
        let db = self.db();
        run(&db, conversation, run_id)?;
        let revisions:Vec<i64>=db.prepare("SELECT spec_revision FROM ledger_versions WHERE run_id=?1 AND entry_id=?2 ORDER BY version")?.query_map(params![run_id,id],|r|r.get(0))?.collect::<std::result::Result<_,_>>()?;
        let mut out = Vec::new();
        for revision in revisions {
            if let Some(e) = entries(&db, run_id, revision)?
                .into_iter()
                .find(|e| e.spec.id == id)
            {
                out.push(e);
            }
        }
        Ok(out)
    }
    pub fn bind_ledger_turn(
        &self,
        conversation: &str,
        run_id: &str,
        turn: &str,
        revision: i64,
        phase: &str,
        sources: &[String],
    ) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        validate_turn_revision(&tx, conversation, run_id, revision)?;
        validate_turn_owner(&tx, conversation, turn)?;
        tx.execute(
            "INSERT INTO ledger_turns VALUES(?1,?2,?3,?4)",
            params![turn, run_id, revision, phase],
        )?;
        record_source_coverage(&tx, turn, run_id, sources)?;
        tx.commit()?;
        Ok(())
    }
    pub fn task_has_ledger_inputs(&self, task: &str) -> Result<bool> {
        Ok(self.db().query_row(
            "SELECT EXISTS(SELECT 1 FROM task_inputs WHERE task_id=?1)",
            [task],
            |r| r.get(0),
        )?)
    }
    pub fn set_run_completion(
        &self,
        conversation: &str,
        run_id: &str,
        revision: i64,
        complete: bool,
    ) -> Result<()> {
        let db = self.db();
        let r = run(&db, conversation, run_id)?;
        require(
            r.spec_revision == revision,
            "scope changed before completion",
        )?;
        db.execute(
            "UPDATE runs SET state=?2 WHERE id=?1",
            params![run_id, if complete { "complete" } else { "active" }],
        )?;
        Ok(())
    }
    pub fn applied_ledger_change(&self, change: &LedgerChange) -> Result<Option<ChangeResult>> {
        let row: Option<(String, String)> = self
            .db()
            .query_row(
                "SELECT request_hash,result FROM ledger_decisions WHERE operation_id=?1",
                [&change.operation_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        row.map(|(hash, result)| {
            require(
                hash == crate::hash(serde_json::to_vec(change)?),
                "operation identity reused with a different payload",
            )?;
            Ok(serde_json::from_str(&result)?)
        })
        .transpose()
    }
    pub fn list_runs(&self, conversation: &str) -> Result<Vec<Run>> {
        self.conversation(conversation)?;
        let db = self.db();
        let ids = db
            .prepare(
                "SELECT id FROM runs WHERE conversation_id=?1 ORDER BY created_ms DESC,rowid DESC",
            )?
            .query_map([conversation], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ids.iter().map(|id| run(&db, conversation, id)).collect()
    }
    pub fn latest_ledger_proposal(
        &self,
        conversation: &str,
        run_id: &str,
    ) -> Result<serde_json::Value> {
        let db = self.db();
        run(&db, conversation, run_id)?;
        let (id,request,hash):(String,String,String)=db.query_row("SELECT p.id,p.request,p.diff_hash FROM ledger_proposals p WHERE p.run_id=?1 AND json_extract(p.request,'$.expected_spec_revision')=(SELECT spec_revision FROM runs WHERE id=p.run_id) AND NOT EXISTS(SELECT 1 FROM ledger_rejections x WHERE x.proposal_id=p.id) AND NOT EXISTS(SELECT 1 FROM ledger_decisions d WHERE d.operation_id=json_extract(p.request,'$.operation_id')) ORDER BY p.created_ms DESC,p.rowid DESC LIMIT 1",[run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        let change: LedgerChange = serde_json::from_str(&request)?;
        let diff = saved_proposal_diff(&db, &id, &change)?;
        require(
            super::diff_hash(&diff)? == hash,
            "proposal impact changed; prepare a new proposal from the preserved changes",
        )?;
        Ok(
            serde_json::json!({"change":change,"proposal":{"proposal_id":id,"diff_hash":hash,"diff":diff}}),
        )
    }
    pub(super) fn check_analyst_author(&self, change: &LedgerChange, turn: &str) -> Result<()> {
        let db = self.db();
        run(&db, &change.conversation, &change.run_id)?;
        let authorized:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM ledger_turns l JOIN turns t ON t.id=l.turn_id WHERE l.turn_id=?1 AND l.run_id=?2 AND l.spec_revision=?3 AND l.phase='extraction' AND t.role='requirements-extraction-analyst' AND t.conversation_id=?4 AND t.status='done')",params![turn,change.run_id,change.expected_spec_revision,change.conversation],|r|r.get(0))?;
        require(
            authorized,
            "ledger author is not the runtime-bound requirements analyst",
        )
    }
    pub fn run_tasks(
        &self,
        conversation: &str,
        run_id: &str,
    ) -> Result<Vec<crate::lean::store::Task>> {
        let plans = {
            let db = self.db();
            run(&db, conversation, run_id)?;
            db.prepare("SELECT plan_id FROM plan_runs WHERE run_id=?1 ORDER BY rowid")?
                .query_map([run_id], |r| r.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        let mut tasks = Vec::new();
        for plan in plans {
            tasks.extend(self.plan(&plan)?);
        }
        Ok(tasks)
    }
    pub fn ledger_entry_tasks(
        &self,
        conversation: &str,
        run_id: &str,
        entry: &str,
    ) -> Result<Vec<serde_json::Value>> {
        let db = self.db();
        run(&db, conversation, run_id)?;
        Ok(db.prepare("SELECT t.id,t.plan_id,json_extract(t.spec,'$.id'),json_extract(t.spec,'$.title'),t.status,i.generation,l.version FROM task_ledger_inputs l JOIN task_inputs i ON i.task_id=l.task_id JOIN tasks t ON t.id=i.task_id WHERE i.run_id=?1 AND l.entry_id=?2 ORDER BY t.created_ms DESC,t.rowid DESC")?.query_map(params![run_id,entry],|r|Ok(serde_json::json!({"id":r.get::<_,String>(0)?,"plan":r.get::<_,String>(1)?,"task":r.get::<_,String>(2)?,"title":r.get::<_,String>(3)?,"status":r.get::<_,String>(4)?,"generation":r.get::<_,i64>(5)?,"entry_version":r.get::<_,i64>(6)?})))?.collect::<std::result::Result<_,_>>()?)
    }
}

fn validate_turn_revision(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    run_id: &str,
    revision: i64,
) -> Result<()> {
    let current = run(tx, conversation, run_id)?.spec_revision;
    require(
        (0..=current).contains(&revision),
        "invalid ledger turn revision",
    )
}

fn validate_turn_owner(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    turn: &str,
) -> Result<()> {
    let owner: String = tx.query_row(
        "SELECT conversation_id FROM turns WHERE id=?1",
        [turn],
        |row| row.get(0),
    )?;
    require(
        owner == conversation,
        "turn belongs to another conversation",
    )
}

fn record_source_coverage(
    tx: &rusqlite::Transaction<'_>,
    turn: &str,
    run: &str,
    sources: &[String],
) -> Result<()> {
    for source in sources {
        let belongs: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM ledger_sources WHERE id=?1 AND run_id=?2)",
            params![source, run],
            |row| row.get(0),
        )?;
        require(belongs, "source belongs to another run")?;
        tx.execute(
            "INSERT INTO ledger_source_coverage VALUES(?1,?2)",
            params![turn, source],
        )?;
    }
    Ok(())
}

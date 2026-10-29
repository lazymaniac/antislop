use super::validation::{impact, same_meaning, validate_change};
mod apply;
use super::*;
use apply::*;

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
        if let Some(saved) = existing_proposal(&tx, change)? {
            return Ok(saved);
        }
        let diff = validate_change(&tx, change, state, true)?;
        let id = crate::id();
        let hash = super::diff_hash(&diff)?;
        persist_proposal(&tx, change, &diff, &id, &hash, author)?;
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
        if let Some(result) = saved_decision(&tx, change, &request_hash)? {
            return Ok(result);
        }
        let rejected: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM ledger_rejections x JOIN ledger_proposals p ON p.id=x.proposal_id WHERE json_extract(p.request,'$.operation_id')=?1)", [&change.operation_id], |r|r.get(0))?;
        require(!rejected, "proposal already rejected")?;
        let diff = validate_change(&tx, change, state, user)?;
        let changed_versions = persist_entries(&tx, change, &diff.before, state, diff.to_revision)?;
        mark_affected_tasks(&tx, &diff.affected_tasks)?;
        carry_unaffected_inputs(&tx, change, &changed_versions, &diff.affected_tasks)?;
        let result = ChangeResult {
            run_id: change.run_id.clone(),
            spec_revision: diff.to_revision,
            affected_tasks: diff.affected_tasks,
        };
        save_decision(&tx, change, &result, author, &request_hash)?;
        emit_change(&tx, change, &result, &changed_versions)?;
        resume_if_approved(&tx, change, user)?;
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
        let changed = changed_entry_ids(&before, &after);
        let all: Vec<_> = after.iter().map(|e| e.spec.clone()).collect();
        let tasks = impact(&self.db(), run_id, &changed, &all)?;
        Ok(ledger_diff_result(
            run_id, from, to, before, after, &changed, tasks,
        ))
    }
}

fn changed_entry_ids(before: &[LedgerEntry], after: &[LedgerEntry]) -> HashSet<String> {
    after
        .iter()
        .filter(|entry| {
            !before.iter().any(|old| {
                old.spec.id == entry.spec.id
                    && (old.version == entry.version || same_meaning(&old.spec, &entry.spec))
            })
        })
        .map(|entry| entry.spec.id.clone())
        .collect()
}

fn ledger_diff_result(
    run: &str,
    from: i64,
    to: i64,
    before: Vec<LedgerEntry>,
    after: Vec<LedgerEntry>,
    changed: &HashSet<String>,
    tasks: Vec<String>,
) -> LedgerDiff {
    LedgerDiff {
        run_id: run.into(),
        from_revision: from,
        to_revision: to,
        before: before
            .into_iter()
            .filter(|entry| changed.contains(&entry.spec.id))
            .collect(),
        after: after
            .into_iter()
            .filter(|entry| changed.contains(&entry.spec.id))
            .map(|entry| entry.spec)
            .collect(),
        affected_tasks: tasks,
    }
}

fn existing_proposal(
    tx: &rusqlite::Transaction<'_>,
    change: &LedgerChange,
) -> Result<Option<(String, LedgerDiff)>> {
    let saved = tx.query_row("SELECT id,request FROM ledger_proposals WHERE json_extract(request,'$.operation_id')=?1", [&change.operation_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))).optional()?;
    let Some((id, request)) = saved else {
        return Ok(None);
    };
    require(
        serde_json::from_str::<LedgerChange>(&request)? == *change,
        "operation identity reused with a different proposal",
    )?;
    Ok(Some((id.clone(), saved_proposal_diff(tx, &id, change)?)))
}

fn persist_proposal(
    tx: &rusqlite::Transaction<'_>,
    change: &LedgerChange,
    diff: &LedgerDiff,
    id: &str,
    hash: &str,
    author: &str,
) -> Result<()> {
    tx.execute("INSERT INTO ledger_proposals(id,run_id,request,diff_hash,created_ms,author) VALUES(?1,?2,?3,?4,?5,?6)", params![id,change.run_id,serde_json::to_string(change)?,hash,now_ms(),author])?;
    for task in &diff.affected_tasks {
        tx.execute(
            "INSERT INTO ledger_proposal_tasks VALUES(?1,?2)",
            params![id, task],
        )?;
    }
    event(
        tx,
        &change.conversation,
        "ledger_change_proposed",
        &serde_json::json!({"proposal":id,"run":change.run_id,"revision":change.expected_spec_revision,"author":author}),
    )
}

use super::*;

mod status;
use status::{enforce_dependency_statuses, set_ledger_status};

fn validate_evidence_content(proposal: &EvidenceProposal, note: &str) -> Result<()> {
    require(
        !proposal.result.trim().is_empty()
            && !proposal.integrated_fingerprint.trim().is_empty()
            && !note.trim().is_empty(),
        "empty evidence or assessment",
    )?;
    require(
        !matches!(
            proposal.result.trim().to_ascii_lowercase().as_str(),
            "done" | "completed" | "ok"
        ),
        "completion assertion is not evidence",
    )
}

fn validate_evidence_sessions(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    run_id: &str,
    revision: i64,
    verifier: &str,
    proposal: &EvidenceProposal,
) -> Result<()> {
    independent(tx, &proposal.producer_turn, verifier)?;
    require(
        audit_turns_bound(tx, run_id, revision, &proposal.producer_turn, verifier)?,
        "evidence sessions are not bound to the active run revision",
    )?;
    let role: String = tx.query_row(
        "SELECT role FROM turns WHERE id=?1 AND conversation_id=?2",
        params![verifier, conversation],
        |row| row.get(0),
    )?;
    require(
        role == "requirements-audit-verifier",
        "only integrated-result verifier may assess completion evidence",
    )
}

fn audit_turns_bound(
    tx: &rusqlite::Transaction<'_>,
    run: &str,
    revision: i64,
    producer: &str,
    verifier: &str,
) -> Result<bool> {
    Ok(tx.query_row("SELECT EXISTS(SELECT 1 FROM ledger_turns p JOIN ledger_turns v ON p.run_id=v.run_id WHERE p.turn_id=?1 AND v.turn_id=?2 AND p.run_id=?3 AND p.spec_revision=?4 AND v.spec_revision=?4 AND p.phase='audit' AND v.phase='audit')", params![producer,verifier,run,revision], |row| row.get(0))?)
}

fn validate_evidence_task(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    run: &str,
    verifier: &str,
    proposal: &EvidenceProposal,
) -> Result<()> {
    let valid = evidence_task_is_current(tx, conversation, run, verifier, proposal)?;
    require(
        valid,
        "evidence has stale task inputs, attempt, result, or unauthorized producer",
    )
}

fn evidence_task_is_current(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    run: &str,
    _verifier: &str,
    proposal: &EvidenceProposal,
) -> Result<bool> {
    Ok(tx.query_row("SELECT EXISTS(SELECT 1 FROM task_inputs i JOIN tasks t ON t.id=i.task_id JOIN turns u ON u.id=?4 WHERE i.task_id=?1 AND i.run_id=?2 AND i.generation=?3 AND t.round=?5 AND t.status='approved' AND u.conversation_id=?6 AND u.role='requirements-audit-analyst')", params![proposal.task_id,run,proposal.generation,proposal.producer_turn,proposal.attempt,conversation], |row| row.get(0))?)
}

fn validate_evidence_criterion(
    tx: &rusqlite::Transaction<'_>,
    run_id: &str,
    revision: i64,
    proposal: &EvidenceProposal,
) -> Result<()> {
    let all = entries(tx, run_id, revision)?;
    let entry = active_evidence_entry(&all, proposal)?;
    require(
        entry
            .spec
            .criteria
            .iter()
            .any(|criterion| criterion.id == proposal.criterion_id),
        "unknown evidence criterion",
    )?;
    validate_evidence_task_links(tx, proposal)
}

fn active_evidence_entry<'a>(
    entries: &'a [LedgerEntry],
    proposal: &EvidenceProposal,
) -> Result<&'a LedgerEntry> {
    entries
        .iter()
        .find(|entry| {
            entry.spec.id == proposal.entry_id
                && entry.version == proposal.entry_version
                && entry.spec.disposition == Disposition::Active
        })
        .ok_or_else(|| invalid("evidence refers to an old or inactive entry"))
}

fn validate_evidence_task_links(
    tx: &rusqlite::Transaction<'_>,
    proposal: &EvidenceProposal,
) -> Result<()> {
    let linked: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM task_ledger_inputs WHERE task_id=?1 AND entry_id=?2 AND version=?3)", params![proposal.task_id,proposal.entry_id,proposal.entry_version], |row| row.get(0))?;
    require(linked, "evidence task does not implement this obligation")?;
    let spec: String = tx.query_row(
        "SELECT spec FROM tasks WHERE id=?1",
        [&proposal.task_id],
        |row| row.get(0),
    )?;
    require(
        serde_json::from_str::<TaskSpec>(&spec)?
            .criterion_ids
            .contains(&proposal.criterion_id),
        "evidence task does not implement this criterion",
    )
}

fn insert_evidence(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    evidence: &Evidence,
) -> Result<()> {
    let p = &evidence.proposal;
    tx.execute("INSERT INTO ledger_evidence VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)", params![evidence.id,evidence.run_id,p.entry_id,p.entry_version,p.criterion_id,p.task_id,p.generation,p.attempt,p.producer_turn,evidence.verifier_turn,p.result,p.artifact,p.integrated_fingerprint,evidence.assessment,evidence.approved,now_ms()])?;
    event(
        tx,
        conversation,
        "ledger_evidence",
        &serde_json::json!({"run":evidence.run_id,"entry":p.entry_id,"version":p.entry_version,"criterion":p.criterion_id,"evidence":evidence.id,"approved":evidence.approved}),
    )
}

mod queries;
use queries::*;

impl Store {
    pub fn add_ledger_evidence(
        &self,
        conversation: &str,
        run_id: &str,
        verifier_turn: &str,
        assessment: &Assessment,
    ) -> Result<Evidence> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let r = run(&tx, conversation, run_id)?;
        let p = &assessment.evidence;
        validate_evidence_content(p, &assessment.note)?;
        validate_evidence_sessions(&tx, conversation, run_id, r.spec_revision, verifier_turn, p)?;
        validate_evidence_task(&tx, conversation, run_id, verifier_turn, p)?;
        validate_evidence_criterion(&tx, run_id, r.spec_revision, p)?;
        let evidence = Evidence {
            id: crate::id(),
            run_id: run_id.into(),
            proposal: p.clone(),
            verifier_turn: verifier_turn.into(),
            assessment: assessment.note.clone(),
            approved: assessment.approved,
        };
        insert_evidence(&tx, conversation, &evidence)?;
        tx.commit()?;
        Ok(evidence)
    }
    pub fn ledger_evidence(
        &self,
        conversation: &str,
        run_id: &str,
        entry: Option<&str>,
    ) -> Result<Vec<Evidence>> {
        let db = self.db();
        run(&db, conversation, run_id)?;
        Ok(db.prepare("SELECT id,entry_id,entry_version,criterion_id,task_id,generation,attempt,producer_turn,verifier_turn,result,artifact,fingerprint,assessment,approved FROM ledger_evidence WHERE run_id=?1 AND (?2 IS NULL OR entry_id=?2) ORDER BY created_ms,rowid")?.query_map(params![run_id,entry],|r|Ok(Evidence{id:r.get(0)?,run_id:run_id.into(),proposal:EvidenceProposal{entry_id:r.get(1)?,entry_version:r.get(2)?,criterion_id:r.get(3)?,task_id:r.get(4)?,generation:r.get(5)?,attempt:r.get(6)?,producer_turn:r.get(7)?,result:r.get(9)?,artifact:r.get(10)?,integrated_fingerprint:r.get(11)?},verifier_turn:r.get(8)?,assessment:r.get(12)?,approved:r.get(13)?}))?.collect::<std::result::Result<_,_>>()?)
    }
    pub fn ledger_statuses(
        &self,
        conversation: &str,
        run_id: &str,
        revision: Option<i64>,
        fingerprint: &str,
    ) -> Result<Vec<LedgerEntry>> {
        let mut all = self.ledger_entries(conversation, run_id, revision)?;
        let evidence = self.ledger_evidence(conversation, run_id, None)?;
        let task_rows = self.ledger_status_tasks(run_id)?;
        for e in &mut all {
            set_ledger_status(e, &task_rows, &evidence, fingerprint)?;
        }
        enforce_dependency_statuses(&mut all);
        Ok(all)
    }
    pub fn list_ledger(&self, query: &LedgerQuery, fingerprint: &str) -> Result<LedgerPage> {
        let run = self.run(&query.conversation, &query.run_id)?;
        let linked = linked_entry_ids(self, query)?;
        let entries = self.ledger_statuses(
            &query.conversation,
            &query.run_id,
            query.revision,
            fingerprint,
        )?;
        Ok(build_ledger_page(
            run,
            filter_ledger_entries(entries, query, linked),
            query.offset,
            query.limit,
        ))
    }
    pub fn requirements_gaps(
        &self,
        conversation: &str,
        run_id: &str,
        fingerprint: &str,
    ) -> Result<Vec<String>> {
        let r = self.run(conversation, run_id)?;
        let mut gaps = extraction_gap(self, conversation, run_id)?;
        let all = self.ledger_statuses(conversation, run_id, None, fingerprint)?;
        gaps.extend(entry_gaps(&all));
        if audit_missing(self, run_id, r.spec_revision, fingerprint)? {
            gaps.push(
                "integrated compliance audit and independent verification are missing or stale"
                    .into(),
            );
        }
        Ok(gaps)
    }
}

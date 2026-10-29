use super::super::*;

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
        let change = load_rejection(&tx, conversation, run_id, id, hash)?;
        if verify_rejection_retry(self, &tx, id, comment)? {
            return Ok(());
        }
        validate_proposal_state(&tx, conversation, run_id, id, hash, &change)?;
        persist_rejection(&tx, conversation, run_id, id, hash, comment, &change)?;
        tx.commit()?;
        Ok(())
    }
}

fn load_rejection(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    run_id: &str,
    id: &str,
    hash: &str,
) -> Result<LedgerChange> {
    run(tx, conversation, run_id)?;
    let (request, expected): (String, String) = tx.query_row(
        "SELECT request,diff_hash FROM ledger_proposals WHERE id=?1 AND run_id=?2",
        params![id, run_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
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
    Ok(change)
}

fn validate_proposal_state(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    run_id: &str,
    id: &str,
    hash: &str,
    change: &LedgerChange,
) -> Result<()> {
    require(
        run(tx, conversation, run_id)?.spec_revision == change.expected_spec_revision,
        "scope changed before rejection",
    )?;
    let accepted: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM ledger_decisions WHERE operation_id=?1)",
        [&change.operation_id],
        |row| row.get(0),
    )?;
    require(!accepted, "proposal already accepted")?;
    require(
        crate::lean::ledger::diff_hash(&saved_proposal_diff(tx, id, change)?)? == hash,
        "proposal impact changed; reload the proposal",
    )?;
    Ok(())
}

fn verify_rejection_retry(
    store: &Store,
    tx: &rusqlite::Transaction<'_>,
    id: &str,
    comment: &str,
) -> Result<bool> {
    let previous: Option<String> = tx
        .query_row(
            "SELECT m.text FROM ledger_rejections x JOIN messages m ON m.seq=x.comment_message WHERE x.proposal_id=?1",
            [id],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(previous) = previous {
        require(
            store.decode_content(&previous)? == comment,
            "proposal already rejected with another comment",
        )?;
        return Ok(true);
    }
    Ok(false)
}

fn persist_rejection(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    run_id: &str,
    id: &str,
    hash: &str,
    comment: &str,
    change: &LedgerChange,
) -> Result<()> {
    let message = insert_user_correction(tx, conversation, comment)?;
    let (resume, text) = queue_rejection_resume(tx, change, id, run_id, comment)?;
    tx.execute(
        "INSERT INTO ledger_rejections VALUES(?1,?2,?3,?4,?5)",
        params![id, message, resume, hash, now_ms()],
    )?;
    tx.execute("INSERT INTO planning_runs VALUES(?1,?2) ON CONFLICT(conversation_id) DO UPDATE SET run_id=excluded.run_id", params![conversation,run_id])?;
    event(
        tx,
        conversation,
        "ledger_proposal_rejected",
        &serde_json::json!({"run":run_id,"proposal":id,"comment_message":message}),
    )?;
    event(
        tx,
        conversation,
        "message",
        &serde_json::json!({"seq":resume,"author":"requirements-approval","text":text}),
    )
}

fn insert_user_correction(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    comment: &str,
) -> Result<i64> {
    tx.execute(
        "INSERT INTO messages(conversation_id,author,text,created_ms) VALUES(?1,'user',?2,?3)",
        params![conversation, comment, now_ms()],
    )?;
    let message = tx.last_insert_rowid();
    event(
        tx,
        conversation,
        "message",
        &serde_json::json!({"seq":message,"author":"user","text":comment}),
    )?;
    Ok(message)
}

fn queue_rejection_resume(
    tx: &rusqlite::Transaction<'_>,
    change: &LedgerChange,
    id: &str,
    run_id: &str,
    comment: &str,
) -> Result<(i64, String)> {
    let text = format!(
        "The user rejected requirements proposal {id} for run {run_id}. User feedback on that rejected proposal:\n{comment}\nPrepare a revised proposal for explicit user approval. Rejection does not authorize applying a correction."
    );
    Ok((insert_resume_message(tx, change, &text)?, text))
}

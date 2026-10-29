//! Durable revision submission receipts; a daemon restart never loses a proposal.
use super::*;

impl Store {
    pub(in crate::lean) fn reserve_revision_job(
        &self,
        conversation: &str,
        key: &str,
        revision: &PlanRevision,
    ) -> Result<(String, bool)> {
        crate::require(
            !key.is_empty() && key.len() <= 128,
            "request_key must contain 1–128 bytes",
        )?;
        let payload = serde_json::to_string(revision)?;
        let mut db = self.db();
        let tx = db.transaction()?;
        if let Some(result) = resume_revision_job(&tx, conversation, key, &payload)? {
            tx.commit()?;
            return Ok(result);
        }
        require_no_review(&tx, conversation)?;
        let id = insert_revision_job(&tx, conversation, key, revision, &payload)?;
        tx.commit()?;
        Ok((id, true))
    }

    pub(in crate::lean) fn revision_job(&self, id: &str) -> Result<Value> {
        let (state, error): (String, Option<String>) = self.db().query_row(
            "SELECT status,error FROM plan_revision_jobs WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok(serde_json::json!({"request":id,"state":state,"error":error}))
    }

    pub(in crate::lean) fn finish_revision_job(&self, id: &str, result: &Result<()>) -> Result<()> {
        self.db().execute(
            "UPDATE plan_revision_jobs SET status=?2,error=?3 WHERE id=?1 AND status='reviewing'",
            params![
                id,
                if result.is_ok() {
                    "applied"
                } else {
                    "rejected"
                },
                result.as_ref().err().map(ToString::to_string)
            ],
        )?;
        Ok(())
    }

    /// Revision reviews of this plan rejected since one was last applied.
    pub(in crate::lean) fn rejected_revisions_in_a_row(&self, plan: &str) -> Result<usize> {
        let count: i64 = self.db().query_row(
            "SELECT count(*) FROM plan_revision_jobs WHERE plan_id=?1 AND status='rejected' \
             AND error LIKE 'revision review rejected%' \
             AND rowid>coalesce((SELECT max(rowid) FROM plan_revision_jobs WHERE plan_id=?1 AND status='applied'),0)",
            [plan],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    pub(in crate::lean) fn interrupt_revision_jobs(&self) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        tx.execute("INSERT INTO messages(conversation_id,author,text,created_ms) SELECT conversation_id,'runtime','Plan revision interrupted. Resubmit the same request_key to resume independent review; no approval is implied.',?1 FROM plan_revision_jobs WHERE status='reviewing'",[now_ms()])?;
        tx.execute("UPDATE plan_revision_jobs SET status='interrupted',error='daemon interrupted revision review' WHERE status='reviewing'",[])?;
        tx.commit()?;
        Ok(())
    }
}

fn resume_revision_job(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    key: &str,
    payload: &str,
) -> Result<Option<(String, bool)>> {
    let previous: Option<(String, String, String)> = tx.query_row(
        "SELECT id,proposal,status FROM plan_revision_jobs WHERE conversation_id=?1 AND request_key=?2",
        params![conversation,key],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).optional()?;
    let Some((id, previous, status)) = previous else {
        return Ok(None);
    };
    crate::require(
        previous == payload,
        "request_key already names a different revision",
    )?;
    let resume = status == "interrupted";
    if resume {
        require_no_review(tx, conversation)?;
        tx.execute(
            "UPDATE plan_revision_jobs SET status='reviewing',error=NULL WHERE id=?1",
            [&id],
        )?;
    }
    Ok(Some((id, resume)))
}

fn insert_revision_job(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    key: &str,
    revision: &PlanRevision,
    payload: &str,
) -> Result<String> {
    let id = crate::id();
    tx.execute(
        "INSERT INTO plan_revision_jobs(id,conversation_id,plan_id,request_key,proposal,status) VALUES(?1,?2,?3,?4,?5,'reviewing')",
        params![id,conversation,revision.plan,key,payload],
    )?;
    Ok(id)
}

fn require_no_review(tx: &rusqlite::Transaction<'_>, conversation: &str) -> Result<()> {
    let busy:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM plan_revision_jobs WHERE conversation_id=?1 AND status='reviewing')",[conversation],|r|r.get(0))?;
    crate::require(!busy, "another plan revision is still under review")
}

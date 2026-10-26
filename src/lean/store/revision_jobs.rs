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
        let previous: Option<(String,String,String)> = tx.query_row("SELECT id,proposal,status FROM plan_revision_jobs WHERE conversation_id=?1 AND request_key=?2",params![conversation,key],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        if let Some((id, previous, status)) = previous {
            crate::require(
                previous == payload,
                "request_key already names a different revision",
            )?;
            let resume = status == "interrupted";
            if resume {
                require_no_review(&tx, conversation)?;
                tx.execute(
                    "UPDATE plan_revision_jobs SET status='reviewing',error=NULL WHERE id=?1",
                    [&id],
                )?;
            }
            tx.commit()?;
            return Ok((id, resume));
        }
        require_no_review(&tx, conversation)?;
        let id = crate::id();
        tx.execute("INSERT INTO plan_revision_jobs(id,conversation_id,plan_id,request_key,proposal,status) VALUES(?1,?2,?3,?4,?5,'reviewing')",params![id,conversation,revision.plan,key,payload])?;
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

    pub(in crate::lean) fn interrupt_revision_jobs(&self) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        tx.execute("INSERT INTO messages(conversation_id,author,text,created_ms) SELECT conversation_id,'runtime','Plan revision interrupted. Resubmit the same request_key to resume independent review; no approval is implied.',?1 FROM plan_revision_jobs WHERE status='reviewing'",[now_ms()])?;
        tx.execute("UPDATE plan_revision_jobs SET status='interrupted',error='daemon interrupted revision review' WHERE status='reviewing'",[])?;
        tx.commit()?;
        Ok(())
    }
}

fn require_no_review(tx: &rusqlite::Transaction<'_>, conversation: &str) -> Result<()> {
    let busy:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM plan_revision_jobs WHERE conversation_id=?1 AND status='reviewing')",[conversation],|r|r.get(0))?;
    crate::require(!busy, "another plan revision is still under review")
}

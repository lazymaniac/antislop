use super::*;
#[path = "reviews/assignments.rs"]
mod assignments;

struct LedgerReview<'a> {
    conversation: &'a str,
    run_id: &'a str,
    revision: i64,
    kind: &'a str,
    producer: &'a str,
    verifier: &'a str,
    fingerprint: &'a str,
    approved: bool,
    note: &'a str,
}

fn persist_review(tx: &rusqlite::Transaction<'_>, review: &LedgerReview<'_>) -> Result<()> {
    validate_review(tx, review)?;
    insert_review(tx, review)?;
    event(
        tx,
        review.conversation,
        "ledger_review",
        &serde_json::json!({"run":review.run_id,"revision":review.revision,"kind":review.kind,"approved":review.approved}),
    )
}

fn validate_review(tx: &rusqlite::Transaction<'_>, review: &LedgerReview<'_>) -> Result<()> {
    validate_review_session(tx, review)?;
    validate_review_coverage(tx, review)
}

fn validate_review_session(
    tx: &rusqlite::Transaction<'_>,
    review: &LedgerReview<'_>,
) -> Result<()> {
    let current = run(tx, review.conversation, review.run_id)?.spec_revision;
    require(
        current == review.revision,
        "review refers to an old spec revision",
    )?;
    require(
        matches!(review.kind, "extraction" | "audit" | "options"),
        "unknown ledger review kind",
    )?;
    require(!review.note.trim().is_empty(), "review needs an assessment")?;
    independent(tx, review.producer, review.verifier)?;
    require(
        review_turns_bound(tx, review)?,
        "review sessions are not bound to this run revision and phase",
    )
}

fn review_turns_bound(tx: &rusqlite::Transaction<'_>, review: &LedgerReview<'_>) -> Result<bool> {
    Ok(tx.query_row("SELECT EXISTS(SELECT 1 FROM ledger_turns p JOIN ledger_turns v ON p.run_id=v.run_id WHERE p.turn_id=?1 AND v.turn_id=?2 AND p.run_id=?3 AND v.spec_revision=?4 AND p.phase=?5 AND v.phase=?5)", params![review.producer,review.verifier,review.run_id,review.revision,review.kind], |row| row.get(0))?)
}

fn validate_review_coverage(
    tx: &rusqlite::Transaction<'_>,
    review: &LedgerReview<'_>,
) -> Result<()> {
    let uncovered = review_sources_uncovered(tx, review)?;
    require(
        !uncovered,
        "source set changed during review; repeat against all sources",
    )?;
    let (producer_conversation, producer_role) = review_producer(tx, review.producer)?;
    let verifier_role: String = tx.query_row(
        "SELECT role FROM turns WHERE id=?1",
        [review.verifier],
        |row| row.get(0),
    )?;
    require(
        producer_conversation == review.conversation
            && producer_role == format!("requirements-{}-analyst", review.kind)
            && verifier_role == format!("requirements-{}-verifier", review.kind),
        "review author is not authorized for this run and phase",
    )
}

fn review_sources_uncovered(
    tx: &rusqlite::Transaction<'_>,
    review: &LedgerReview<'_>,
) -> Result<bool> {
    Ok(tx.query_row("SELECT EXISTS(SELECT 1 FROM ledger_sources s WHERE s.run_id=?1 AND (NOT EXISTS(SELECT 1 FROM ledger_source_coverage c WHERE c.turn_id=?2 AND c.source_id=s.id) OR NOT EXISTS(SELECT 1 FROM ledger_source_coverage c WHERE c.turn_id=?3 AND c.source_id=s.id)))", params![review.run_id,review.producer,review.verifier], |row| row.get(0))?)
}

fn review_producer(tx: &rusqlite::Transaction<'_>, producer: &str) -> Result<(String, String)> {
    Ok(tx.query_row(
        "SELECT conversation_id,role FROM turns WHERE id=?1",
        [producer],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?)
}

fn insert_review(tx: &rusqlite::Transaction<'_>, review: &LedgerReview<'_>) -> Result<()> {
    tx.execute("INSERT INTO ledger_reviews(run_id,spec_revision,kind,producer_turn,verifier_turn,fingerprint,approved,note,created_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)", params![review.run_id,review.revision,review.kind,review.producer,review.verifier,review.fingerprint,review.approved,review.note,now_ms()])?;
    Ok(())
}

impl Store {
    pub(crate) fn rejected_requirements_feedback(
        &self,
        conversation: &str,
        run_id: &str,
    ) -> Result<Option<serde_json::Value>> {
        let current = self.run(conversation, run_id)?;
        let row: Option<(String, String)> = self.db().query_row(
            "SELECT p.id,m.text FROM ledger_rejections r JOIN ledger_proposals p ON p.id=r.proposal_id JOIN messages m ON m.seq=r.comment_message WHERE p.run_id=?1 AND m.conversation_id=?2 AND m.author='user' AND json_extract(p.request,'$.expected_spec_revision')=?3 ORDER BY r.created_ms DESC,r.rowid DESC LIMIT 1",
            params![run_id, conversation, current.spec_revision],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?;
        row.map(|(proposal, comment)| {
            let comment = self.decode_content(&comment)?;
            Ok(serde_json::json!({"proposal":proposal,"decision":"rejected","comment":comment,"approval_required":true}))
        }).transpose()
    }

    /// Local review data for the analyst, never orchestration routing messages.
    pub(crate) fn requirements_review_findings(
        &self,
        conversation: &str,
        run_id: &str,
    ) -> Result<String> {
        let current = self.run(conversation, run_id)?;
        let note: Option<String> = self.db().query_row(
            "SELECT note FROM ledger_reviews WHERE run_id=?1 AND spec_revision=?2 AND kind='extraction' ORDER BY created_ms DESC,rowid DESC LIMIT 1",
            params![run_id, current.spec_revision], |row| row.get(0),
        ).optional()?;
        note.map(|note| {
            let findings = crate::lean::turn_result::TurnResult::review_verdict(&note)
                .map(|verdict| verdict.findings)
                .unwrap_or_default();
            serde_json::to_string(&findings).map_err(Into::into)
        })
        .unwrap_or_else(|| Ok("[]".into()))
    }

    pub fn task_generation(&self, task: &str) -> Result<i64> {
        Ok(self
            .db()
            .query_row(
                "SELECT generation FROM task_inputs WHERE task_id=?1",
                [task],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0))
    }
    pub fn assert_task_inputs(&self, task: &crate::lean::store::Task) -> Result<()> {
        require(
            self.task_generation(&task.id)? == task.input_generation,
            "stale task generation; result retained in turn history",
        )?;
        let db = self.db();
        let bound: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM task_inputs i JOIN task_ledger_inputs l ON l.task_id=i.task_id WHERE i.task_id=?1)", [&task.id], |r|r.get(0))?;
        require(bound, "integrity error: task has no requirement inputs")?;
        let stale:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM task_ledger_inputs l JOIN task_inputs i ON i.task_id=l.task_id WHERE l.task_id=?1 AND l.version<>(SELECT max(v.version) FROM ledger_versions v WHERE v.run_id=i.run_id AND v.entry_id=l.entry_id))",[&task.id],|r|r.get(0))?;
        require(!stale, "task ledger inputs changed; replan before resuming")
    }
    #[allow(clippy::too_many_arguments)] // Session and input identities are validated together.
    pub fn record_ledger_review(
        &self,
        conversation: &str,
        run_id: &str,
        revision: i64,
        kind: &str,
        producer: &str,
        verifier: &str,
        fingerprint: &str,
        approved: bool,
        note: &str,
    ) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let record = LedgerReview {
            conversation,
            run_id,
            revision,
            kind,
            producer,
            verifier,
            fingerprint,
            approved,
            note,
        };
        persist_review(&tx, &record)?;
        tx.commit().map_err(Into::into)
    }
    /// Records the analyst's extraction as accepted for the current revision.
    /// The producer turn stands in for the verifier: it is bound to every source.
    pub fn accept_extraction(
        &self,
        conversation: &str,
        run_id: &str,
        revision: i64,
        producer: &str,
    ) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        validate_extraction_revision(&tx, conversation, run_id, revision)?;
        record_extraction_acceptance(&tx, conversation, run_id, revision, producer)?;
        tx.commit().map_err(Into::into)
    }
    pub fn extraction_current(&self, conversation: &str, run_id: &str) -> Result<bool> {
        let db = self.db();
        let r = run(&db, conversation, run_id)?;
        Ok(db.query_row("SELECT approved FROM ledger_reviews WHERE run_id=?1 AND spec_revision=?2 AND kind='extraction' AND NOT EXISTS(SELECT 1 FROM ledger_sources s WHERE s.run_id=?1 AND NOT EXISTS(SELECT 1 FROM ledger_source_coverage c WHERE c.turn_id=ledger_reviews.verifier_turn AND c.source_id=s.id)) ORDER BY created_ms DESC,rowid DESC LIMIT 1",params![run_id,r.spec_revision],|r|r.get(0)).optional()?.unwrap_or(false))
    }
}

fn validate_extraction_revision(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    run_id: &str,
    revision: i64,
) -> Result<()> {
    require(
        run(tx, conversation, run_id)?.spec_revision == revision,
        "review refers to an old spec revision",
    )
}

fn record_extraction_acceptance(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    run_id: &str,
    revision: i64,
    producer: &str,
) -> Result<()> {
    let record = LedgerReview {
        conversation,
        run_id,
        revision,
        kind: "extraction",
        producer,
        verifier: producer,
        fingerprint: "",
        approved: true,
        note: "Accepted without a separate source-coverage review.",
    };
    insert_review(tx, &record)?;
    event(
        tx,
        conversation,
        "ledger_review",
        &serde_json::json!({"run":run_id,"revision":revision,"kind":"extraction","approved":true}),
    )
}

use super::super::parse_block;
use super::SolutionChoice;
use crate::{
    Error, Result,
    lean::store::{Store, now_ms},
    require,
};
use rusqlite::{OptionalExtension, params};
use serde_json::json;

impl Store {
    pub fn recover_options_comparisons(&self) -> Result<()> {
        let pending = self.db().prepare(
            "SELECT r.conversation_id,r.id,v.producer_turn,v.verifier_turn,v.approved,v.note FROM planning_runs p JOIN runs r ON r.id=p.run_id JOIN session_workflow w ON w.conversation_id=r.conversation_id JOIN ledger_reviews v ON v.run_id=r.id AND v.spec_revision=r.spec_revision AND v.kind='options' WHERE w.phase='planning' AND NOT EXISTS(SELECT 1 FROM solution_choices c WHERE c.run_id=r.id AND c.spec_revision=r.spec_revision) AND v.rowid=(SELECT max(recent.rowid) FROM ledger_reviews recent WHERE recent.run_id=r.id AND recent.spec_revision=r.spec_revision AND recent.kind='options') AND NOT EXISTS(SELECT 1 FROM ledger_sources s WHERE s.run_id=r.id AND (NOT EXISTS(SELECT 1 FROM ledger_source_coverage x WHERE x.turn_id=v.producer_turn AND x.source_id=s.id) OR NOT EXISTS(SELECT 1 FROM ledger_source_coverage x WHERE x.turn_id=v.verifier_turn AND x.source_id=s.id)))"
        )?.query_map([],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,bool>(4)?,row.get::<_,String>(5)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
        for (conversation, run_id, producer, verifier, approved, note) in pending {
            self.recover_options_comparison(
                &conversation,
                &run_id,
                &producer,
                &verifier,
                approved,
                note,
            )?;
        }
        Ok(())
    }

    fn recover_options_comparison(
        &self,
        conversation: &str,
        run_id: &str,
        producer: &str,
        verifier: &str,
        approved: bool,
        note: String,
    ) -> Result<()> {
        let Some(mut choice) = self.restored_choice(conversation, run_id, producer)? else {
            return Ok(());
        };
        if choice.validate(&self.run(conversation, run_id)?).is_err() {
            return Ok(());
        }
        choice.review_findings = (!approved).then_some(note).into_iter().collect();
        self.save_solution_choice(conversation, &choice, producer, verifier)
    }

    fn restored_choice(
        &self,
        conversation: &str,
        run_id: &str,
        producer: &str,
    ) -> Result<Option<SolutionChoice>> {
        if !self.extraction_current(conversation, run_id)? {
            return Ok(None);
        }
        let Some(encoded) = self.agent_final_payload(conversation, producer)? else {
            return Ok(None);
        };
        let payload = match self.hydrate_event_payload("agent_final", &encoded) {
            Ok(payload) => payload,
            Err(error) => {
                return self
                    .report_unrestorable_comparison(conversation, error)
                    .map(|()| None);
            }
        };
        Ok(parse_block::<SolutionChoice>(payload["text"].as_str().unwrap_or(""), "options").ok())
    }

    fn agent_final_payload(&self, conversation: &str, producer: &str) -> Result<Option<String>> {
        Ok(self.db().query_row(
            "SELECT payload FROM events WHERE conversation_id=?1 AND turn_id=?2 AND kind='agent_final' ORDER BY seq DESC LIMIT 1",
            params![conversation, producer],
            |row| row.get(0),
        ).optional()?)
    }

    fn report_unrestorable_comparison(&self, conversation: &str, error: Error) -> Result<()> {
        self.append_event(
            conversation,
            None,
            None,
            "notice",
            &json!({"text":format!("Could not restore the options comparison: {error}. Send a message to compare solutions again.")}),
        )?;
        Ok(())
    }

    pub fn solution_choice(
        &self,
        conversation: &str,
        run_id: &str,
    ) -> Result<Option<SolutionChoice>> {
        let run = self.run(conversation, run_id)?;
        let encoded: Option<String> = self.db().query_row(
            "SELECT c.decision FROM solution_choices c WHERE c.run_id=?1 AND c.spec_revision=?2 AND EXISTS(SELECT 1 FROM ledger_reviews v WHERE v.run_id=c.run_id AND v.spec_revision=c.spec_revision AND v.kind='options' AND v.producer_turn=c.producer_turn AND v.verifier_turn=c.verifier_turn) AND NOT EXISTS(SELECT 1 FROM ledger_sources s WHERE s.run_id=c.run_id AND (NOT EXISTS(SELECT 1 FROM ledger_source_coverage x WHERE x.turn_id=c.producer_turn AND x.source_id=s.id) OR NOT EXISTS(SELECT 1 FROM ledger_source_coverage x WHERE x.turn_id=c.verifier_turn AND x.source_id=s.id)))",
            params![run_id, run.spec_revision], |row| row.get(0)
        ).optional()?;
        encoded
            .map(|text| serde_json::from_str(&text).map_err(Error::from))
            .transpose()
    }

    fn save_solution_choice(
        &self,
        conversation: &str,
        choice: &SolutionChoice,
        producer: &str,
        verifier: &str,
    ) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let revision = Self::choice_revision(&tx, conversation, choice)?;
        Self::require_review(&tx, choice, revision, producer, verifier)?;
        let inserted = Self::upsert_solution_choice(&tx, choice, revision, producer, verifier)?;
        if inserted == 1 {
            Self::publish_solution_choice(&tx, conversation, choice, revision)?;
        }
        tx.commit()?;
        Ok(())
    }

    fn choice_revision(
        tx: &rusqlite::Transaction<'_>,
        conversation: &str,
        choice: &SolutionChoice,
    ) -> Result<i64> {
        let revision: i64 = tx.query_row(
            "SELECT spec_revision FROM runs WHERE id=?1 AND conversation_id=?2",
            params![choice.run_id, conversation],
            |row| row.get(0),
        )?;
        require(
            revision == choice.spec_revision,
            "requirements changed during options comparison; repeat it",
        )?;
        Ok(revision)
    }

    fn require_review(
        tx: &rusqlite::Transaction<'_>,
        choice: &SolutionChoice,
        revision: i64,
        producer: &str,
        verifier: &str,
    ) -> Result<()> {
        let reviewed: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM ledger_reviews WHERE run_id=?1 AND spec_revision=?2 AND kind='options' AND producer_turn=?3 AND verifier_turn=?4)",
            params![choice.run_id, revision, producer, verifier],
            |row| row.get(0),
        )?;
        require(
            reviewed,
            "solution comparison requires an independent review",
        )
    }

    fn upsert_solution_choice(
        tx: &rusqlite::Transaction<'_>,
        choice: &SolutionChoice,
        revision: i64,
        producer: &str,
        verifier: &str,
    ) -> Result<usize> {
        let encoded = serde_json::to_string(choice)?;
        Ok(tx.execute(
            "INSERT INTO solution_choices VALUES(?1,?2,?3,?4,?5) ON CONFLICT(run_id,spec_revision) DO UPDATE SET producer_turn=excluded.producer_turn,verifier_turn=excluded.verifier_turn,decision=excluded.decision",
            params![choice.run_id, revision, producer, verifier, encoded],
        )?)
    }

    fn publish_solution_choice(
        tx: &rusqlite::Transaction<'_>,
        conversation: &str,
        choice: &SolutionChoice,
        revision: i64,
    ) -> Result<()> {
        let text = planner_message(choice, revision)?;
        tx.execute(
            "INSERT INTO messages(conversation_id,author,text,created_ms) VALUES(?1,'options-planner',?2,?3)",
            params![conversation, text, now_ms()],
        )?;
        let seq = tx.last_insert_rowid();
        let events = solution_choice_events(choice, revision, seq, text);
        for (kind, payload) in events {
            tx.execute(
                "INSERT INTO events(conversation_id,kind,payload,created_ms) VALUES(?1,?2,?3,?4)",
                params![conversation, kind, payload.to_string(), now_ms()],
            )?;
        }
        Ok(())
    }
}

fn planner_message(choice: &SolutionChoice, revision: i64) -> Result<String> {
    Ok(format!(
        "Create a task plan from the verified ledger for run {} and the reviewed implementation options below. Recommendation: {} for requirements revision {}. Resolve every review finding in the actual plan, including replacing an unsupported recommendation with a measurement-first decision task when needed. Do not assume a rejected recommendation is approved. Describe decision criteria and dependencies rather than promising unmeasured results. Requirements approval is not implementation approval.\n{}",
        choice.run_id,
        choice.selected,
        revision,
        serde_json::to_string(choice)?,
    ))
}

fn solution_choice_events(
    choice: &SolutionChoice,
    revision: i64,
    seq: i64,
    text: String,
) -> [(&'static str, serde_json::Value); 3] {
    [
        (
            "message",
            json!({"seq":seq,"author":"options-planner","text":text}),
        ),
        (
            "options_selected",
            json!({"selected":choice.selected,"rationale":choice.rationale,"alternatives":choice.alternatives,"criteria":choice.criteria,"run":choice.run_id,"revision":revision,"review_findings":choice.review_findings}),
        ),
        (
            "planning_progress",
            json!({"text":"Options compared. Preparing one plan with the review findings."}),
        ),
    ]
}

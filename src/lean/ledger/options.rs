use super::{Run, parse_block};
use crate::{
    Error, Result,
    lean::{
        Core,
        store::{Store, now_ms},
        tasks,
    },
    require,
};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Alternative {
    pub id: String,
    pub approach: String,
    pub evaluations: Vec<String>,
    pub risks: String,
    pub validation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SolutionChoice {
    pub run_id: String,
    pub spec_revision: i64,
    pub criteria: Vec<String>,
    pub alternatives: Vec<Alternative>,
    pub selected: String,
    #[serde(deserialize_with = "deserialize_rationale")]
    pub rationale: String,
    #[serde(default)]
    pub review_findings: Vec<String>,
}
fn deserialize_rationale<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<String, D::Error> {
    fn text(value: serde_json::Value) -> Option<String> {
        match value {
            serde_json::Value::String(text) => Some(text),
            serde_json::Value::Array(values) => Some(
                values
                    .into_iter()
                    .map(text)
                    .collect::<Option<Vec<_>>>()?
                    .join("\n"),
            ),
            serde_json::Value::Object(fields) => Some(
                fields
                    .into_iter()
                    .map(|(key, value)| Some(format!("{key}: {}", text(value)?)))
                    .collect::<Option<Vec<_>>>()?
                    .join("\n"),
            ),
            _ => None,
        }
    }
    let value = serde_json::Value::deserialize(deserializer)?;
    text(value).ok_or_else(|| {
        serde::de::Error::custom("rationale must contain text or named text sections")
    })
}

impl SolutionChoice {
    fn validate(&self, run: &Run) -> Result<()> {
        require(
            self.run_id == run.id && self.spec_revision == run.spec_revision,
            "options refer to a different requirements revision",
        )?;
        require(
            (2..=4).contains(&self.alternatives.len()),
            "compare two to four real implementation alternatives",
        )?;
        require(
            !self.criteria.is_empty() && self.criteria.iter().all(|s| !s.trim().is_empty()),
            "options need explicit shared selection criteria",
        )?;
        require(
            !self.rationale.trim().is_empty(),
            "solution choice needs an evidence-based rationale",
        )?;
        let mut ids = HashSet::new();
        for option in &self.alternatives {
            require(
                !option.id.trim().is_empty() && ids.insert(&option.id),
                "alternative IDs must be nonempty and unique",
            )?;
            require(
                [&option.approach, &option.risks, &option.validation]
                    .iter()
                    .all(|s| !s.trim().is_empty()),
                "each alternative needs an approach, risks and validation",
            )?;
            require(
                option.evaluations.len() == self.criteria.len()
                    && option.evaluations.iter().all(|s| !s.trim().is_empty()),
                "evaluate every alternative against every shared criterion",
            )?;
        }
        require(
            ids.contains(&self.selected),
            "selected solution must be one of the compared alternatives",
        )
    }
}

impl Store {
    pub fn recover_options_comparisons(&self) -> Result<()> {
        let pending = self.db().prepare(
            "SELECT r.conversation_id,r.id,v.producer_turn,v.verifier_turn,v.approved,v.note FROM planning_runs p JOIN runs r ON r.id=p.run_id JOIN session_workflow w ON w.conversation_id=r.conversation_id JOIN ledger_reviews v ON v.run_id=r.id AND v.spec_revision=r.spec_revision AND v.kind='options' WHERE w.phase='planning' AND NOT EXISTS(SELECT 1 FROM solution_choices c WHERE c.run_id=r.id AND c.spec_revision=r.spec_revision) AND v.rowid=(SELECT max(recent.rowid) FROM ledger_reviews recent WHERE recent.run_id=r.id AND recent.spec_revision=r.spec_revision AND recent.kind='options') AND NOT EXISTS(SELECT 1 FROM ledger_sources s WHERE s.run_id=r.id AND (NOT EXISTS(SELECT 1 FROM ledger_source_coverage x WHERE x.turn_id=v.producer_turn AND x.source_id=s.id) OR NOT EXISTS(SELECT 1 FROM ledger_source_coverage x WHERE x.turn_id=v.verifier_turn AND x.source_id=s.id)))"
        )?.query_map([],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,bool>(4)?,row.get::<_,String>(5)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
        for (conversation, run_id, producer, verifier, approved, note) in pending {
            if !self.extraction_current(&conversation, &run_id)? {
                continue;
            }
            let encoded: Option<String> = self.db().query_row(
                "SELECT payload FROM events WHERE conversation_id=?1 AND turn_id=?2 AND kind='agent_final' ORDER BY seq DESC LIMIT 1",params![conversation,producer],|row|row.get(0)
            ).optional()?;
            let Some(encoded) = encoded else {
                continue;
            };
            let payload = match self.hydrate_event_payload("agent_final", &encoded) {
                Ok(payload) => payload,
                Err(error) => {
                    self.append_event(&conversation,None,None,"notice",&json!({"text":format!("Could not restore the options comparison: {error}. Send a message to compare solutions again.")}))?;
                    continue;
                }
            };
            let Ok(mut choice) =
                parse_block::<SolutionChoice>(payload["text"].as_str().unwrap_or(""), "options")
            else {
                continue;
            };
            if choice.validate(&self.run(&conversation, &run_id)?).is_err() {
                continue;
            }
            choice.review_findings.clear();
            if !approved {
                choice.review_findings.push(note);
            }
            self.save_solution_choice(&conversation, &choice, &producer, &verifier)?;
        }
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
        let revision: i64 = tx.query_row(
            "SELECT spec_revision FROM runs WHERE id=?1 AND conversation_id=?2",
            params![choice.run_id, conversation],
            |row| row.get(0),
        )?;
        require(
            revision == choice.spec_revision,
            "requirements changed during options comparison; repeat it",
        )?;
        let reviewed: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM ledger_reviews WHERE run_id=?1 AND spec_revision=?2 AND kind='options' AND producer_turn=?3 AND verifier_turn=?4)", params![choice.run_id,revision,producer,verifier],|row|row.get(0))?;
        require(
            reviewed,
            "solution comparison requires an independent review",
        )?;
        let encoded = serde_json::to_string(choice)?;
        let inserted = tx.execute(
            "INSERT INTO solution_choices VALUES(?1,?2,?3,?4,?5) ON CONFLICT(run_id,spec_revision) DO UPDATE SET producer_turn=excluded.producer_turn,verifier_turn=excluded.verifier_turn,decision=excluded.decision",
            params![choice.run_id, revision, producer, verifier, encoded],
        )?;
        if inserted == 1 {
            let text = format!(
                "Create a task plan from the verified ledger for run {} and the reviewed implementation options below. Recommendation: {} for requirements revision {}. Resolve every review finding in the actual plan, including replacing an unsupported recommendation with a measurement-first decision task when needed. Do not assume a rejected recommendation is approved. Describe decision criteria and dependencies rather than promising unmeasured results. Requirements approval is not implementation approval.\n{}",
                choice.run_id, choice.selected, revision, encoded
            );
            tx.execute("INSERT INTO messages(conversation_id,author,text,created_ms) VALUES(?1,'options-planner',?2,?3)",params![conversation,text,now_ms()])?;
            let seq = tx.last_insert_rowid();
            for (kind, payload) in [
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
            ] {
                tx.execute("INSERT INTO events(conversation_id,kind,payload,created_ms) VALUES(?1,?2,?3,?4)",params![conversation,kind,payload.to_string(),now_ms()])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}

impl Core {
    pub(in crate::lean) fn ensure_options(
        self: &Arc<Self>,
        conversation: &str,
        run: &Run,
        cancel: &AtomicBool,
    ) -> Result<bool> {
        require(
            self.store.extraction_current(conversation, &run.id)?,
            "verify requirements before comparing implementation options",
        )?;
        if self.store.solution_choice(conversation, &run.id)?.is_some() {
            return Ok(false);
        }
        let sources = self.full_ledger_sources(conversation, &run.id)?;
        let mut feedback = String::new();
        let (mut choice, producer) = loop {
            if cancel.load(Ordering::SeqCst) {
                return Err(Error::Invalid("options comparison cancelled".into()));
            }
            let prompt = format!(
                "Compare implementation options before task planning. Inspect the actual project read-only. Compare 2-4 genuinely different feasible approaches under the SAME explicit criteria, including requirement coverage, complexity, risk, validation and cost. Respect user-prescribed methods. State evidence and unknowns honestly; no implementation or fabricated measurements. This is input to a plan, not a completed implementation. When the best approach depends on future measurements, recommend a diagnostic-first strategy with concrete decision criteria; do not claim a measured winner. Propose tests for later execution rather than requiring their results now. Suggested numeric tolerances are proposals, not new user obligations. Return a ```options JSON block with run_id, spec_revision, criteria (nonempty string array), alternatives (2-4 objects: id, approach, evaluations [one assessment per criterion in the same order], risks, validation), selected (alternative id), rationale (text or named text sections explaining evidence, tradeoffs and what would reopen the choice). Run {} revision {}. Full sources:\n{sources}\nFormat correction:\n{feedback}",
                run.id, run.spec_revision
            );
            let (answer, producer) =
                self.requirements_turn(conversation, &run.id, "options", false, prompt, cancel)?;
            let validated =
                parse_block::<SolutionChoice>(&answer, "options").and_then(|mut choice| {
                    choice.review_findings.clear();
                    choice.validate(run)?;
                    Ok(choice)
                });
            match validated {
                Ok(choice) => break (choice, producer),
                Err(error @ (Error::Invalid(_) | Error::Json(_))) if feedback.is_empty() => {
                    feedback = format!(
                        "{error}. Fix the schema without changing the comparison. Previous response:\n{answer}"
                    );
                }
                Err(error) => return Err(error),
            }
        };
        let (answer,verifier)=self.requirements_turn(conversation,&run.id,"options",true,format!("Independently verify implementation options before planning. Read the actual project and full sources. Check that the alternatives are real, cover the same requirements and criteria, and the selected solution follows from evidence rather than preference. Reject violated obligations, unsupported claims, fake alternatives and missing validation. User-prescribed methods remain binding. This review gives planning feedback, not implementation certification. Accept a sound measurement-first strategy when data needed to choose an optimization does not exist yet. Identify what the complete plan must resolve; do not require implementation results before a plan can exist. Return a ```verdict JSON block with approved (boolean) and findings (objects with issue). Proposed comparison:\n{}\nFull sources:\n{sources}",serde_json::to_string(&choice)?),cancel)?;
        let verdict = tasks::parse_verdict(&answer);
        self.store.record_ledger_review(
            conversation,
            &run.id,
            run.spec_revision,
            "options",
            &producer,
            &verifier,
            "",
            verdict.approved,
            &answer,
        )?;
        if !verdict.approved {
            choice.review_findings = verdict
                .findings
                .iter()
                .map(|finding| tasks::findings_text(std::slice::from_ref(finding)))
                .collect();
            if choice.review_findings.is_empty() {
                choice.review_findings.push("The options reviewer did not approve the recommendation. Establish the missing evidence before selecting the implementation approach.".into());
            }
        }
        let _integration = self.integrating.lock().expect("integration mutex");
        require(
            !cancel.load(Ordering::SeqCst),
            "options comparison cancelled",
        )?;
        self.store
            .save_solution_choice(conversation, &choice, &producer, &verifier)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choice_cannot_select_an_uncompared_approach_or_use_incomparable_criteria() {
        let run = Run {
            id: "run".into(),
            conversation_id: "conversation".into(),
            goal: "goal".into(),
            spec_revision: 2,
            state: "active".into(),
            start_message: 1,
        };
        let alternative = |id: &str| Alternative {
            id: id.into(),
            approach: format!("Use {id}"),
            evaluations: vec!["Meets the requirement".into()],
            risks: "Risk is addressed by checks".into(),
            validation: "Run the acceptance checks".into(),
        };
        let mut choice = SolutionChoice {
            run_id: run.id.clone(),
            spec_revision: 2,
            criteria: vec!["Requirement coverage".into()],
            alternatives: vec![alternative("a"), alternative("b")],
            selected: "a".into(),
            rationale: "A meets the requirements with less complexity".into(),
            review_findings: vec![],
        };
        choice.validate(&run).unwrap();
        choice.selected = "uncompared".into();
        assert!(choice.validate(&run).is_err());
        choice.selected = "a".into();
        choice.alternatives[1].evaluations.clear();
        assert!(choice.validate(&run).is_err());
        choice.alternatives[1]
            .evaluations
            .push("Covers scope".into());
        choice.spec_revision = 1;
        assert!(choice.validate(&run).is_err());
    }
}

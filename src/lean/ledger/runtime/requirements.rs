use super::*;

#[path = "requirements/extraction.rs"]
mod extraction;
#[path = "requirements/turn.rs"]
mod turn;

impl Core {
    pub(in crate::lean) fn ensure_requirements(
        self: &Arc<Self>,
        conversation: &str,
        cancel: &AtomicBool,
    ) -> Result<Run> {
        let (run, extraction) = self.prepare_requirements(conversation)?;
        let Some((sources, new_source)) = extraction else {
            return Ok(run);
        };
        let producer =
            self.extract_requirements(conversation, &run, &sources, new_source, cancel)?;
        self.accept_requirements(conversation, &run, &producer)
    }

    fn prepare_requirements(&self, conversation: &str) -> Result<(Run, Option<(String, bool)>)> {
        let user = self.store.messages_by(conversation, "user", 0)?;
        let r = match self.store.planning_run(conversation)? {
            Some(r) => r,
            None => self.store.create_run(
                conversation,
                &tasks::bound(
                    &user
                        .first()
                        .ok_or_else(|| Error::Invalid("run needs a user request".into()))?
                        .text,
                    1000,
                ),
            )?,
        };
        self.store.select_planning_run(conversation, &r.id)?;
        require(
            !self.store.has_pending_ledger_proposal(&r.id)?,
            "requirements proposal awaits user approval",
        )?;
        let new_source = self.register_missing_sources(conversation, &r, &user)?;
        if self.store.extraction_current(conversation, &r.id)? && !new_source {
            return Ok((r, None));
        }
        let sources = self.full_ledger_sources(conversation, &r.id)?;
        Ok((r, Some((sources, new_source))))
    }

    pub(in crate::lean::ledger::runtime) fn register_missing_sources(
        &self,
        conversation: &str,
        run: &Run,
        user: &[crate::lean::store::Message],
    ) -> Result<bool> {
        let known = self.store.ledger_sources(conversation, &run.id)?;
        let decisions = self.store.execution_decision_ids(conversation)?;
        let mut added = false;
        for message in user
            .iter()
            .filter(|message| message.seq >= run.start_message)
            .filter(|message| {
                !decisions.iter().any(|id| {
                    message
                        .text
                        .starts_with(&format!("Interview answer ({id})\n"))
                })
            })
        {
            if !known
                .iter()
                .any(|source| source.message == Some(message.seq))
            {
                self.store
                    .register_message_source(conversation, &run.id, message.seq)?;
                added = true;
            }
        }
        Ok(added)
    }

    fn extract_requirements(
        self: &Arc<Self>,
        conversation: &str,
        run: &Run,
        sources: &str,
        new_source: bool,
        cancel: &AtomicBool,
    ) -> Result<String> {
        if let Some(producer) = self
            .store
            .approved_requirements_producer(conversation, &run.id, run.spec_revision)?
            .filter(|_| !new_source)
        {
            return Ok(producer);
        }
        let existing = self.store.ledger_entries(conversation, &run.id, None)?;
        let feedback = self.requirements_feedback(conversation, run)?;
        let prompt = extraction::extraction_prompt(&feedback, sources);
        let (change, producer) =
            self.analyze_requirements(conversation, run, &existing, &prompt, cancel)?;
        self.apply_extracted_requirements(conversation, run, &existing, &change, &producer)?;
        Ok(producer)
    }

    fn requirements_feedback(&self, conversation: &str, run: &Run) -> Result<String> {
        let findings: serde_json::Value = serde_json::from_str(
            &self
                .store
                .requirements_review_findings(conversation, &run.id)?,
        )?;
        Ok(serde_json::to_string(&json!({
            "review_findings": findings,
            "user_rejection": self.store.rejected_requirements_feedback(conversation, &run.id)?,
        }))?)
    }

    fn analyze_requirements(
        self: &Arc<Self>,
        conversation: &str,
        run: &Run,
        existing: &[LedgerEntry],
        prompt: &str,
        cancel: &AtomicBool,
    ) -> Result<(LedgerChange, String)> {
        extraction::ensure_not_cancelled(cancel)?;
        let (answer, producer) = self.requirements_turn(
            conversation,
            &run.id,
            "extraction",
            false,
            prompt.to_owned(),
            cancel,
        )?;
        // submit_ledger checked the extraction in the turn; checked again
        // here in case the run moved on since.
        let crate::lean::turn_result::TurnResult::Ledger { extraction } =
            crate::lean::turn_result::TurnResult::decode(&answer)
        else {
            return Err(Error::Invalid(MISSING_LEDGER.into()));
        };
        let change = self.validate_extraction(conversation, run, existing, extraction)?;
        Ok((change, producer))
    }

    pub(in crate::lean::ledger::runtime) fn validate_extraction(
        &self,
        conversation: &str,
        run: &Run,
        existing: &[LedgerEntry],
        mut extraction: Extraction,
    ) -> Result<LedgerChange> {
        require(
            extraction.run_id == run.id && extraction.expected_spec_revision == run.spec_revision,
            "the requirements changed during extraction; extract again",
        )?;
        extraction
            .entries
            .retain(|entry| !existing.iter().any(|old| old.spec == *entry));
        require(
            !extraction.entries.is_empty() || !existing.is_empty(),
            "requirements extraction is empty",
        )?;
        let change = extraction::extracted_change(conversation, run, extraction.entries);
        if !change.entries.is_empty() {
            self.store.preview_ledger_change(&change, &self.state)?;
        }
        Ok(change)
    }

    fn apply_extracted_requirements(
        &self,
        conversation: &str,
        run: &Run,
        existing: &[LedgerEntry],
        change: &LedgerChange,
        producer: &str,
    ) -> Result<()> {
        if change.entries.is_empty() {
            return Ok(());
        }
        if self.store.has_rejected_ledger_proposal(&run.id)?
            || change
                .entries
                .iter()
                .any(|entry| existing.iter().any(|old| old.spec.id == entry.id))
        {
            let (proposal, diff) =
                self.store
                    .propose_agent_ledger_change(change, &self.state, producer)?;
            self.store.append_event(conversation, None, None, "ledger_approval_required",
                &json!({"proposal":proposal,"run":run.id,"revision":run.spec_revision,"affected_tasks":diff.affected_tasks}))?;
            return Err(Error::Invalid(
                "requirements proposal awaits user approval".into(),
            ));
        }
        let _integration = self.integrating.lock().expect("integration mutex");
        self.store
            .apply_analyst_ledger_change(change, &self.state, producer)?;
        Ok(())
    }

    /// The analyst's ledger is accepted without a second model: the separate
    /// source-coverage review rejected correct ledgers over wording and cost
    /// hours per request. Task and plan reviews still check coverage later.
    fn accept_requirements(&self, conversation: &str, run: &Run, producer: &str) -> Result<Run> {
        let current = self.store.run(conversation, &run.id)?;
        self.require_extracted_entries(conversation, run)?;
        self.store
            .accept_extraction(conversation, &run.id, current.spec_revision, producer)?;
        Ok(current)
    }

    fn require_extracted_entries(&self, conversation: &str, run: &Run) -> Result<()> {
        require(
            !self
                .store
                .ledger_entries(conversation, &run.id, None)?
                .is_empty(),
            "requirements extraction is empty",
        )
    }
}

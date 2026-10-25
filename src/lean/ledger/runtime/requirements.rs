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
        self.verify_requirements(conversation, &run, &sources, &producer, cancel)
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

    fn register_missing_sources(
        &self,
        conversation: &str,
        run: &Run,
        user: &[crate::lean::store::Message],
    ) -> Result<bool> {
        let known = self.store.ledger_sources(conversation, &run.id)?;
        let mut added = false;
        for message in user
            .iter()
            .filter(|message| message.seq >= run.start_message)
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
        let prompt = extraction::extraction_prompt(run, &feedback, sources);
        let (change, producer) =
            self.analyze_requirements(conversation, run, &existing, &prompt, cancel)?;
        self.apply_extracted_requirements(conversation, run, &existing, &change, &producer)?;
        Ok(producer)
    }

    fn requirements_feedback(&self, conversation: &str, run: &Run) -> Result<String> {
        Ok(self
            .store
            .messages_after(conversation, run.start_message)?
            .into_iter()
            .rev()
            .find(|message| {
                message.author == "requirements-reviewer"
                    || message.author == "requirements-approval"
            })
            .map(|message| message.text.clone())
            .unwrap_or_default())
    }

    fn analyze_requirements(
        self: &Arc<Self>,
        conversation: &str,
        run: &Run,
        existing: &[LedgerEntry],
        prompt: &str,
        cancel: &AtomicBool,
    ) -> Result<(LedgerChange, String)> {
        let mut feedback = String::new();
        loop {
            extraction::ensure_not_cancelled(cancel)?;
            let (answer, producer) = self.requirements_turn(
                conversation,
                &run.id,
                "extraction",
                false,
                format!("{prompt}{feedback}"),
                cancel,
            )?;
            match self.validate_extraction(conversation, run, existing, &answer) {
                Ok(change) => return Ok((change, producer)),
                Err(error @ (Error::Invalid(_) | Error::Json(_))) if feedback.is_empty() => {
                    self.record_extraction_correction(conversation, &error)?;
                    feedback = extraction::correction_feedback(&error, &answer);
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn validate_extraction(
        &self,
        conversation: &str,
        run: &Run,
        existing: &[LedgerEntry],
        answer: &str,
    ) -> Result<LedgerChange> {
        let mut extraction: Extraction = parse_block(answer, "ledger")?;
        require(
            extraction.run_id == run.id && extraction.expected_spec_revision == run.spec_revision,
            "analyst returned wrong run or revision",
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

    fn record_extraction_correction(&self, conversation: &str, error: &Error) -> Result<()> {
        self.store.append_event(
            conversation,
            None,
            None,
            "planning_progress",
            &json!({"text": format!("Correcting requirements extraction: {error}")}),
        )?;
        Ok(())
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

    fn verify_requirements(
        self: &Arc<Self>,
        conversation: &str,
        run: &Run,
        sources: &str,
        producer: &str,
        cancel: &AtomicBool,
    ) -> Result<Run> {
        let current = self.store.run(conversation, &run.id)?;
        self.require_extracted_entries(conversation, run)?;
        let (answer,verifier)=self.requirements_turn(conversation,&run.id,"extraction",true,format!(
            "Independently compare the requirement entries to every full source and the interview context. This is extraction before task planning: tasks and execution evidence may legitimately be absent. Selected interview options include their descriptions; use that context before declaring ambiguity. For planning-only requests, completion criteria describe acceptance tests for later implementation, not a demand to implement now. Do not add obligations unsupported by user sources or project instructions. Find omitted obligations, lost qualifiers, ambiguous interpretation, missing plan steps, prescribed methods and missing composite integration criteria. Do not accept analyst assertions as evidence. Read full document artifacts. Return a ```verdict JSON block {{\"approved\":true|false,\"findings\":[{{\"issue\":\"...\"}}]}}. Full sources:\n{sources}"),cancel)?;
        let verdict = tasks::parse_verdict(&answer);
        self.store.record_ledger_review(
            conversation,
            &run.id,
            current.spec_revision,
            "extraction",
            producer,
            &verifier,
            "",
            verdict.approved,
            &answer,
        )?;
        self.finish_requirements_review(conversation, &current, &verdict, cancel)?;
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

    fn finish_requirements_review(
        &self,
        conversation: &str,
        current: &Run,
        verdict: &tasks::Verdict,
        cancel: &AtomicBool,
    ) -> Result<()> {
        if !verdict.approved && !cancel.load(Ordering::SeqCst) {
            self.store.add_message(
                conversation,
                "requirements-reviewer",
                &extraction::requirements_review_message(current, verdict),
            )?;
        }
        require(
            verdict.approved,
            &format!(
                "requirements source coverage rejected: {}",
                tasks::findings_text(&verdict.findings)
            ),
        )
    }
}

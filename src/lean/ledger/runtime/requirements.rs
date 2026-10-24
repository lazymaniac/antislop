use super::*;

impl Core {
    pub(super) fn requirements_turn(
        self: &Arc<Self>,
        conversation: &str,
        run_id: &str,
        phase: &str,
        verifier: bool,
        prompt: String,
        cancel: &AtomicBool,
    ) -> Result<(String, String)> {
        let progress = match (phase, verifier) {
            ("extraction", false) => {
                "Preparing requirements from the interview and source documents."
            }
            ("extraction", true) => "Checking requirements independently before planning.",
            ("options", false) => "Comparing implementation alternatives.",
            ("options", true) => "Checking the solution comparison independently.",
            (_, false) => "Auditing the integrated implementation against requirements.",
            (_, true) => "Verifying the implementation audit independently.",
        };
        self.store.append_event(
            conversation,
            None,
            None,
            "planning_progress",
            &json!({"text":progress}),
        )?;
        let role = format!(
            "requirements-{phase}-{}",
            if verifier { "verifier" } else { "analyst" }
        );
        let setup = TurnSetup {
            session_policy: crate::lean::SessionPolicy::Role,
            conversation,
            plan_id: None,
            role: role.clone(),
            profile: self.config_snapshot().resolve(
                if phase == "options" {
                    WorkKind::Options
                } else {
                    WorkKind::Requirements
                },
                if verifier {
                    RoleKind::Verifier
                } else {
                    RoleKind::Executor
                },
            ),
            cwd: PathBuf::from(self.store.conversation(conversation)?.project),
            add_dirs: vec![],
            access: agent::Access::ReadOnly,
            upto_message: 0,
        };
        let source_ids = self
            .store
            .ledger_sources(conversation, run_id)?
            .into_iter()
            .map(|s| s.id)
            .collect::<Vec<_>>();
        let revision = self.store.run(conversation, run_id)?.spec_revision;
        let context = self.ledger_context(conversation, run_id)?;
        let answer = self
            .turn(setup, cancel, |_| format!("{prompt}\n{context}"))?
            .ok_or_else(|| {
                Error::Invalid(format!(
                    "{role} failed: {}",
                    self.store
                        .last_turn(conversation, &role)
                        .ok()
                        .flatten()
                        .and_then(|t| t.error)
                        .unwrap_or_else(|| "no response".into())
                ))
            })?;
        let turn = self
            .store
            .last_turn(conversation, &role)?
            .ok_or_else(|| Error::Invalid("requirements turn missing".into()))?;
        self.store.bind_ledger_turn(
            conversation,
            run_id,
            &turn.id,
            revision,
            phase,
            &source_ids,
        )?;
        Ok((answer, turn.id))
    }
    pub(in crate::lean) fn ensure_requirements(
        self: &Arc<Self>,
        conversation: &str,
        cancel: &AtomicBool,
    ) -> Result<Run> {
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
        let mut new_source = false;
        let known = self.store.ledger_sources(conversation, &r.id)?;
        for message in user.iter().filter(|m| m.seq >= r.start_message) {
            if !known.iter().any(|s| s.message == Some(message.seq)) {
                self.store
                    .register_message_source(conversation, &r.id, message.seq)?;
                new_source = true;
            }
        }
        if self.store.extraction_current(conversation, &r.id)? && !new_source {
            return Ok(r);
        }
        let sources = self.full_ledger_sources(conversation, &r.id)?;
        let producer = if let Some(producer) = self
            .store
            .approved_requirements_producer(conversation, &r.id, r.spec_revision)?
            .filter(|_| !new_source)
        {
            producer
        } else {
            let existing = self.store.ledger_entries(conversation, &r.id, None)?;
            let review_feedback =
                self.store
                    .messages_by(conversation, "requirements-reviewer", r.start_message)?;
            let review_feedback = review_feedback
                .last()
                .map(|message| message.text.as_str())
                .unwrap_or("");
            let prompt = format!(
                "Act as requirements analyst before planning. Read ALL sources, including middle sections. Preserve qualifiers, exceptions, negative obligations, original byte ranges, parents and dependencies. parent references another requirement or is null. All references must exist in the combined existing and proposed ledger. Split acceptance criteria with globally unique stable IDs; composite parents need mandatory integration criteria. Preserve every mandatory outcome, constraint, method and step prescribed by the user as a requirement with source and acceptance criteria. Suggestions and examples are not automatically obligations. Agent-selected technical steps remain tasks serving existing requirements; do not invent requirements merely to justify tasks. task_ids are proposed planner IDs. Return a ```ledger JSON block with {{\"run_id\":\"{}\",\"expected_spec_revision\":{},\"entries\":[EntrySpec...]}}. EntrySpec fields: id, source (source_id,start,end byte range on grapheme boundaries), interpretation, qualifiers, parent, depends_on, mandatory, implementation_owners, verification_owners, criteria (id,text,mandatory,integration), task_ids, position, disposition (active). Include only additions or proposed semantic changes. Existing obligations must retain stable IDs. Never silently supersede or weaken obligations. For a planning-only request, distinguish acceptance tests the plan must specify from results checked during later execution. Do not invent implementation tasks that have not been planned. Previous independent review feedback (correct omissions; semantic changes still require user approval): {review_feedback}. Sources:\n{sources}",
                r.id, r.spec_revision
            );
            let mut feedback = String::new();
            let (change, producer) = loop {
                if cancel.load(Ordering::SeqCst) {
                    return Err(Error::Invalid("requirements extraction cancelled".into()));
                }
                let (answer, producer) = self.requirements_turn(
                    conversation,
                    &r.id,
                    "extraction",
                    false,
                    format!("{prompt}{feedback}"),
                    cancel,
                )?;
                let validated = (|| -> Result<LedgerChange> {
                    let mut extraction: Extraction = parse_block(&answer, "ledger")?;
                    require(
                        extraction.run_id == r.id
                            && extraction.expected_spec_revision == r.spec_revision,
                        "analyst returned wrong run or revision",
                    )?;
                    extraction
                        .entries
                        .retain(|entry| !existing.iter().any(|old| old.spec == *entry));
                    require(
                        !extraction.entries.is_empty() || !existing.is_empty(),
                        "requirements extraction is empty",
                    )?;
                    let change = LedgerChange {
                        conversation: conversation.into(),
                        run_id: r.id.clone(),
                        expected_spec_revision: r.spec_revision,
                        operation_id: crate::id(),
                        reason: "source extraction".into(),
                        entries: extraction.entries,
                    };
                    if !change.entries.is_empty() {
                        self.store.preview_ledger_change(&change, &self.state)?;
                    }
                    Ok(change)
                })();
                match validated {
                    Ok(change) => break (change, producer),
                    Err(error @ (Error::Invalid(_) | Error::Json(_))) if feedback.is_empty() => {
                        self.store.append_event(
                        conversation,
                        None,
                        None,
                        "planning_progress",
                        &json!({"text": format!("Correcting requirements extraction: {error}")}),
                    )?;
                        feedback = format!(
                            "\nThe previous extraction was rejected: {error}. Correct the complete ledger JSON without dropping obligations or weakening criteria. Previous response:\n{answer}"
                        );
                    }
                    Err(error) => return Err(error),
                }
            };
            if !change.entries.is_empty() {
                if change
                    .entries
                    .iter()
                    .any(|e| existing.iter().any(|old| old.spec.id == e.id))
                {
                    let (proposal, diff) =
                        self.store
                            .propose_agent_ledger_change(&change, &self.state, &producer)?;
                    self.store.append_event(conversation,None,None,"ledger_approval_required",&json!({"proposal":proposal,"run":r.id,"revision":r.spec_revision,"affected_tasks":diff.affected_tasks}))?;
                    return Err(Error::Invalid("requirements analyst proposed a semantic scope change; open /requirements and press p to review the exact diff before planning".into()));
                }
                let _integration = self.integrating.lock().expect("integration mutex");
                self.store
                    .apply_analyst_ledger_change(&change, &self.state, &producer)?;
            }
            producer
        };
        let current = self.store.run(conversation, &r.id)?;
        require(
            !self
                .store
                .ledger_entries(conversation, &r.id, None)?
                .is_empty(),
            "requirements extraction is empty",
        )?;
        let (answer,verifier)=self.requirements_turn(conversation,&r.id,"extraction",true,format!(
            "Independently compare the requirement entries to every full source and the interview context. This is extraction before task planning: tasks and execution evidence may legitimately be absent. Selected interview options include their descriptions; use that context before declaring ambiguity. For planning-only requests, completion criteria describe acceptance tests for later implementation, not a demand to implement now. Do not add obligations unsupported by user sources or project instructions. Find omitted obligations, lost qualifiers, ambiguous interpretation, missing plan steps, prescribed methods and missing composite integration criteria. Do not accept analyst assertions as evidence. Read full document artifacts. Return a ```verdict JSON block {{\"approved\":true|false,\"findings\":[{{\"issue\":\"...\"}}]}}. Full sources:\n{sources}"),cancel)?;
        let verdict = tasks::parse_verdict(&answer);
        self.store.record_ledger_review(
            conversation,
            &r.id,
            current.spec_revision,
            "extraction",
            &producer,
            &verifier,
            "",
            verdict.approved,
            &answer,
        )?;
        if !verdict.approved
            && !cancel.load(Ordering::SeqCst)
            && self
                .store
                .count_since_requirements_reset(conversation, "requirements-reviewer")?
                == 0
        {
            self.store.add_message(conversation, "requirements-reviewer", &format!(
                "Independent requirements review rejected run {} revision {}. Revisit the extraction using this feedback and the full interview context, then regenerate the plan. Do not weaken obligations or bypass scope-change approval. Findings: {}",
                r.id, current.spec_revision, tasks::findings_text(&verdict.findings)
            ))?;
        }
        require(
            verdict.approved,
            &format!(
                "requirements source coverage rejected: {}",
                tasks::findings_text(&verdict.findings)
            ),
        )?;
        Ok(current)
    }
}

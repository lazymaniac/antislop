use super::*;
use crate::{
    Error,
    config::{RoleKind, WorkKind},
    lean::{
        Core, TurnSetup, agent, git,
        tasks::{self, TaskSpec},
    },
};
use serde_json::json;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

impl Core {
    pub fn ledger_fingerprint(&self, conversation: &str) -> Result<String> {
        let project = PathBuf::from(self.store.conversation(conversation)?.project);
        git::integrated_fingerprint(&project)
    }
    pub fn ledger_query(&self, mut query: LedgerQuery, kind: LedgerKind) -> Result<LedgerPage> {
        if query.run_id.is_empty() {
            query.run_id = self
                .store
                .latest_run(&query.conversation)?
                .ok_or_else(|| {
                    Error::Invalid(
                        "no run; requirements are extracted when a plan is proposed".into(),
                    )
                })?
                .id;
        }
        let fp = self
            .ledger_fingerprint(&query.conversation)
            .unwrap_or_default();
        self.store.list_ledger(&query, kind, &fp)
    }
    pub fn apply_user_ledger_change(
        &self,
        proposal_id: &str,
        diff_hash: &str,
        change: &LedgerChange,
    ) -> Result<ChangeResult> {
        // The same mutex protects both scope publication and the last check before Git integration.
        let _integration = self.integrating.lock().expect("integration mutex");
        self.store
            .check_ledger_proposal(proposal_id, diff_hash, change)?;
        let result = match self.store.applied_ledger_change(change)? {
            Some(saved) => saved,
            None => {
                let current = self.store.preview_ledger_change(change, &self.state)?;
                require(
                    super::diff_hash(&current)? == diff_hash,
                    "ledger impact changed; preview the current diff before applying",
                )?;
                self.store.apply_ledger_change(change, &self.state, true)?
            }
        };
        let flags = self.task_cancels.lock().expect("task cancel mutex");
        for task in &result.affected_tasks {
            if let Some((_, flag)) = flags.get(task) {
                flag.store(true, Ordering::SeqCst);
            }
        }
        drop(flags);
        for id in &result.affected_tasks {
            if let Some(task) = self.store.task_by_id(id)? {
                self.announce_task(
                    &change.conversation,
                    &task,
                    Some("scope changed; previous result requires reassessment"),
                );
            }
        }
        Ok(result)
    }
    pub(in crate::lean) fn ledger_context(
        &self,
        conversation: &str,
        run_id: &str,
    ) -> Result<String> {
        Ok(format!(
            "\nRun and ledger inputs (stable identities; keep their versions):\n{}\nSources: {}\nEntries: {}\nNever weaken an obligation. Propose changes for user approval. Worker approval does not verify requirements.\n",
            serde_json::to_string(&self.store.run(conversation, run_id)?)?,
            serde_json::to_string(&self.store.ledger_sources(conversation, run_id)?)?,
            serde_json::to_string(&self.store.ledger_entries(conversation, run_id, None)?.iter().map(|e|json!({"id":e.spec.id,"version":e.version,"spec_revision":e.spec_revision,"spec":e.spec})).collect::<Vec<_>>())?
        ))
    }
    fn requirements_turn(
        self: &Arc<Self>,
        conversation: &str,
        run_id: &str,
        phase: &str,
        verifier: bool,
        prompt: String,
        cancel: &AtomicBool,
    ) -> Result<(String, String)> {
        let role = format!(
            "requirements-{phase}-{}",
            if verifier { "verifier" } else { "analyst" }
        );
        let setup = TurnSetup {
            conversation,
            plan_id: None,
            role: role.clone(),
            profile: self.config_snapshot().resolve(
                WorkKind::Requirements,
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
    fn full_ledger_sources(&self, conversation: &str, run_id: &str) -> Result<String> {
        let mut out = String::new();
        for source in self.store.ledger_sources(conversation, run_id)? {
            if source.message.is_some() {
                out.push_str(&format!(
                    "\nFull user source {}:\n{}\n",
                    source.id,
                    self.store
                        .ledger_source_text(conversation, run_id, &source.id, &self.state)?
                ));
            } else {
                out.push_str(&format!("\nRead the full immutable document {} at {}. Hash {}. Do not rely on excerpts.\n",source.id,self.state.join(source.locator.unwrap_or_default()).display(),source.hash));
            }
        }
        Ok(out)
    }
    /// Extraction is separate from task planning and has a separate verifier session.
    pub(in crate::lean) fn ensure_requirements(
        self: &Arc<Self>,
        conversation: &str,
        cancel: &AtomicBool,
    ) -> Result<Run> {
        let user = self.store.messages_by(conversation, "user", 0)?;
        let r = match self.store.latest_run(conversation)? {
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
        if let Some(task) = self.store.latest_plan(conversation)?.first()
            && self.store.plan_run(&task.plan_id)?.is_none()
            && let Some(document) = self.store.plan_document(conversation, &task.plan_id)?
        {
            self.store.register_document_source(conversation,&r.id,&format!("Legacy agent plan (input only; not verification or authority to weaken user requirements):\n{document}"),&self.state)?;
        }
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
        let existing = self.store.ledger_entries(conversation, &r.id, None)?;
        let (answer,producer)=self.requirements_turn(conversation,&r.id,"extraction",false,format!(
            "Act as requirements analyst before planning. Read ALL sources, including middle sections. Preserve qualifiers, exceptions, negative obligations, original byte ranges, parents and dependencies. Split acceptance criteria with globally unique stable IDs; composite parents need mandatory integration criteria. Preserve every supplied plan step. A prescribed implementation method is also a separate requirement. Technical steps need task_ids, technical_goal, check and criteria. task_ids are proposed planner IDs. Return a ```ledger JSON block with {{\"run_id\":\"{}\",\"expected_spec_revision\":{},\"entries\":[EntrySpec...]}}. EntrySpec fields: id, kind (requirement|plan_item), source (source_id,start,end byte range on grapheme boundaries), interpretation, qualifiers, parent, depends_on, mandatory, implementation_owners, verification_owners, criteria (id,text,mandatory,integration), requirement_ids, task_ids, position, technical_goal, check, disposition (active). Include only additions or proposed semantic changes. Existing obligations must retain stable IDs. Never silently supersede or weaken obligations. Sources:\n{sources}",r.id,r.spec_revision),cancel)?;
        let extraction: Extraction = parse_block(&answer, "ledger")?;
        require(
            extraction.run_id == r.id && extraction.expected_spec_revision == r.spec_revision,
            "analyst returned wrong run or revision",
        )?;
        if !extraction.entries.is_empty() {
            let change = LedgerChange {
                conversation: conversation.into(),
                run_id: r.id.clone(),
                expected_spec_revision: r.spec_revision,
                operation_id: crate::id(),
                reason: "source extraction".into(),
                entries: extraction.entries,
            };
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
        let current = self.store.run(conversation, &r.id)?;
        require(
            !self
                .store
                .ledger_entries(conversation, &r.id, None)?
                .is_empty(),
            "requirements extraction is empty",
        )?;
        let (answer,verifier)=self.requirements_turn(conversation,&r.id,"extraction",true,format!(
            "Independently compare both ledgers to every full source. Find omitted obligations, lost qualifiers, ambiguous interpretation, missing plan steps, prescribed methods and missing composite integration criteria. Do not accept analyst assertions as evidence. Read full document artifacts. Return a ```verdict JSON block {{\"approved\":true|false,\"findings\":[{{\"issue\":\"...\"}}]}}. Full sources:\n{sources}"),cancel)?;
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
        require(
            verdict.approved,
            &format!(
                "requirements source coverage rejected: {}",
                tasks::findings_text(&verdict.findings)
            ),
        )?;
        Ok(current)
    }
    pub(in crate::lean) fn validate_ledger_plan(
        &self,
        conversation: &str,
        plan: &str,
        run_id: &str,
    ) -> Result<()> {
        require(
            self.store.extraction_current(conversation, run_id)?,
            "requirements source coverage needs independent verification",
        )?;
        let tasks = self.store.plan(plan)?;
        if tasks
            .iter()
            .all(|t| self.store.task_has_ledger_inputs(&t.id).unwrap_or(false))
        {
            for t in &tasks {
                self.store.assert_task_inputs(t)?;
            }
            Ok(())
        } else {
            self.store.bind_task_ledger(conversation, run_id, plan)
        }
    }
    /// Auditors inspect the primary integrated tree. Worktree approval is never imported.
    pub(in crate::lean) fn compliance_audit(
        self: &Arc<Self>,
        conversation: &str,
        plan: &str,
        cancel: &AtomicBool,
    ) -> Result<Vec<String>> {
        let Some(run_id) = self.store.plan_run(plan)? else {
            return Ok(vec!["legacy plan has no verified requirement ledger; extract and replan before claiming success".into()]);
        };
        let revision = self.store.run(conversation, &run_id)?.spec_revision;
        let fingerprint = self.ledger_fingerprint(conversation)?;
        let sources = self.full_ledger_sources(conversation, &run_id)?;
        let tasks = self.store.run_tasks(conversation, &run_id)?;
        let (answer,producer)=self.requirements_turn(conversation,&run_id,"audit",false,format!(
            "Audit the integrated project against every full source and both ledgers. Search for obligations omitted in extraction. Inspect actual files and run relevant read-only checks. Do not change files. Integrated fingerprint: {fingerprint}. Task inputs and reports: {}. Return ```compliance JSON {{\"approved\":true|false,\"findings\":[...],\"evidence\":[EvidenceProposal...]}}. Each evidence proposal: entry_id,entry_version,criterion_id,task_id (UUID),generation,attempt (round),producer_turn (leave empty; runtime supplies),result (concrete checks and observed results, never just done),artifact (optional),integrated_fingerprint. Include technical step execution and composite integration criteria. Evidence must concern this integrated tree. Full sources:\n{sources}",serde_json::to_string(&tasks)?),cancel)?;
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Audit {
            approved: bool,
            findings: Vec<serde_json::Value>,
            evidence: Vec<EvidenceProposal>,
        }
        let mut proposed: Audit = parse_block(&answer, "compliance")?;
        for e in &mut proposed.evidence {
            e.producer_turn = producer.clone();
            require(
                e.integrated_fingerprint == fingerprint,
                "auditor inspected a different integrated result",
            )?;
        }
        let (answer,verifier)=self.requirements_turn(conversation,&run_id,"audit",true,format!(
            "Verify this audit independently against ALL full sources, both ledgers and actual integrated files. Find omitted obligations. Independently reproduce/check every evidence result. Reject empty, stale, failed or worktree-only evidence. Read-only access. Integrated fingerprint: {fingerprint}. Analyst findings: {}. Proposed evidence: {}. Return ```compliance_verdict JSON {{\"approved\":true|false,\"note\":\"coverage and gaps\",\"assessments\":[{{\"evidence\":EvidenceProposal,\"approved\":true|false,\"note\":\"independent observation\"}}]}}. Preserve exact proposal fields; runtime owns session identity. Full sources:\n{sources}",serde_json::to_string(&proposed.findings)?,serde_json::to_string(&proposed.evidence)?),cancel)?;
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Review {
            approved: bool,
            note: String,
            assessments: Vec<Assessment>,
        }
        let reviewed: Review = parse_block(&answer, "compliance_verdict")?;
        let _integration = self.integrating.lock().expect("integration mutex");
        require(
            self.ledger_fingerprint(conversation)? == fingerprint,
            "integrated files changed during compliance audit",
        )?;
        require(
            self.store.run(conversation, &run_id)?.spec_revision == revision,
            "scope changed during compliance audit",
        )?;
        for assessment in &reviewed.assessments {
            require(
                proposed.evidence.contains(&assessment.evidence),
                "verifier assessed evidence not proposed by this audit",
            )?;
            self.store
                .add_ledger_evidence(conversation, &run_id, &verifier, assessment)?;
        }
        self.store.record_ledger_review(
            conversation,
            &run_id,
            revision,
            "audit",
            &producer,
            &verifier,
            &fingerprint,
            proposed.approved && reviewed.approved,
            &reviewed.note,
        )?;
        let gaps = self
            .store
            .requirements_gaps(conversation, &run_id, &fingerprint)?;
        self.store
            .set_run_completion(conversation, &run_id, revision, gaps.is_empty())?;
        self.store.append_event(
            conversation,
            None,
            Some(plan),
            "requirements_complete",
            &json!({"run":run_id,"spec_revision":revision,"approved":gaps.is_empty(),"gaps":gaps}),
        )?;
        Ok(gaps)
    }
    pub(in crate::lean) fn task_ledger_context(
        &self,
        conversation: &str,
        task: &crate::lean::store::Task,
    ) -> Result<String> {
        let Some(run_id) = self.store.plan_run(&task.plan_id)? else {
            return Ok(
                "\nLegacy task: its approval cannot establish requirements completion.\n".into(),
            );
        };
        let spec: TaskSpec = serde_json::from_value(task.spec.clone())?;
        let entries: Vec<_> = self
            .store
            .ledger_entries(conversation, &run_id, None)?
            .into_iter()
            .filter(|e| {
                spec.requirement_ids.contains(&e.spec.id) || spec.plan_item_ids.contains(&e.spec.id)
            })
            .collect();
        Ok(format!(
            "\nCurrent run {run_id}, task generation {}, ledger inputs: {}\nEvidence gaps: all mandatory criteria require independent verification on the integrated project. Source artifacts: {}\n",
            task.input_generation,
            serde_json::to_string(&entries.iter().map(|e|json!({"id":e.spec.id,"version":e.version,"spec_revision":e.spec_revision,"spec":e.spec})).collect::<Vec<_>>())?,
            serde_json::to_string(&self.store.ledger_sources(conversation, &run_id)?)?
        ))
    }
}

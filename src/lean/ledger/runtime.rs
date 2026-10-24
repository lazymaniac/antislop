use super::*;

#[path = "runtime/requirements.rs"]
mod requirements;
use crate::{
    Error,
    config::{RoleKind, WorkKind},
    lean::{Core, TurnSetup, agent, git, tasks},
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
    pub fn ledger_query(&self, mut query: LedgerQuery) -> Result<LedgerPage> {
        if query.run_id.is_empty() {
            query.run_id = self
                .store
                .planning_run(&query.conversation)?
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
        self.store.list_ledger(&query, &fp)
    }
    pub fn apply_user_ledger_change(
        self: &Arc<Self>,
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
        drop(_integration);
        if self
            .store
            .has_pending_requirements_approval(&change.conversation)?
        {
            self.wake(&change.conversation);
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
    pub(super) fn full_ledger_sources(&self, conversation: &str, run_id: &str) -> Result<String> {
        let mut out = String::new();
        let messages = self.store.messages_after(conversation, 0)?;
        for source in self.store.ledger_sources(conversation, run_id)? {
            self.append_ledger_source(&mut out, &messages, conversation, run_id, &source)?;
        }
        self.append_interview_summary(&mut out, conversation)?;
        Ok(out)
    }

    fn append_ledger_source(
        &self,
        out: &mut String,
        messages: &[crate::lean::store::Message],
        conversation: &str,
        run_id: &str,
        source: &Source,
    ) -> Result<()> {
        if let Some(seq) = source.message {
            self.append_interview_question(out, messages, seq, &source.id);
            let text =
                self.store
                    .ledger_source_text(conversation, run_id, &source.id, &self.state)?;
            out.push_str(&format!("\nFull user source {}:\n{text}\n", source.id));
        } else {
            let path = self
                .state
                .join(source.locator.as_deref().unwrap_or_default());
            out.push_str(&format!(
                "\nRead the full immutable document {} at {}. Hash {}. Do not rely on excerpts.\n",
                source.id,
                path.display(),
                source.hash
            ));
        }
        Ok(())
    }

    fn append_interview_question(
        &self,
        out: &mut String,
        messages: &[crate::lean::store::Message],
        seq: u64,
        source_id: &str,
    ) {
        let Some(index) = messages.iter().position(|message| message.seq == seq) else {
            return;
        };
        if !messages[index].text.starts_with("Interview answer (") {
            return;
        }
        if let Some(question) = messages[..index]
            .iter()
            .rev()
            .find(|message| message.author == "orchestrator")
        {
            out.push_str(&format!(
                "\nInterview question context for source {source_id} (interpret the selected answer using its option description; this is context, not a separate user obligation; byte ranges refer only to the user source):\n{}\n",
                question.text
            ));
        }
    }

    fn append_interview_summary(&self, out: &mut String, conversation: &str) -> Result<()> {
        if let Some(workflow) = self.store.workflow(conversation)?
            && !workflow.summary.is_empty()
        {
            out.push_str(&format!(
                "\nInterview summary (derived context; verify against user sources and selected option descriptions, never treat additions as user authorization):\n{}\n",
                workflow.summary
            ));
        }
        Ok(())
    }
    /// Extraction is separate from task planning and has a separate verifier session.
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
        require(!tasks.is_empty(), "integrity error: plan has no tasks")?;
        for task in &tasks {
            require(
                self.store.task_has_ledger_inputs(&task.id)?,
                "integrity error: task has no requirement inputs",
            )?;
            self.store.assert_task_inputs(task)?;
        }
        Ok(())
    }
    /// Auditors inspect the primary integrated tree. Worktree approval is never imported.
    pub(in crate::lean) fn compliance_audit(
        self: &Arc<Self>,
        conversation: &str,
        plan: &str,
        cancel: &AtomicBool,
    ) -> Result<Vec<String>> {
        let Some(run_id) = self.store.plan_run(plan)? else {
            return Ok(vec!["integrity error: plan has no requirements run".into()]);
        };
        let revision = self.store.run(conversation, &run_id)?.spec_revision;
        let fingerprint = self.ledger_fingerprint(conversation)?;
        let sources = self.full_ledger_sources(conversation, &run_id)?;
        let tasks = self.store.run_tasks(conversation, &run_id)?;
        let (answer,producer)=self.requirements_turn(conversation,&run_id,"audit",false,format!(
            "Audit the integrated project against every full source and the requirements and their task evidence. Search for obligations omitted in extraction. Inspect actual files and run relevant read-only checks. Do not change files. Integrated fingerprint: {fingerprint}. Task inputs and reports: {}. Return ```compliance JSON {{\"approved\":true|false,\"findings\":[...],\"evidence\":[EvidenceProposal...]}}. Each evidence proposal: entry_id,entry_version,criterion_id,task_id (UUID),generation,attempt (round),producer_turn (leave empty; runtime supplies),result (concrete checks and observed results, never just done),artifact (optional),integrated_fingerprint. Include user-prescribed methods and steps and composite integration criteria. Evidence must concern this integrated tree. Full sources:\n{sources}",serde_json::to_string(&tasks)?),cancel)?;
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
            "Verify this audit independently against ALL full sources, the requirements and their task evidence and actual integrated files. Find omitted obligations. Independently reproduce/check every evidence result. Reject empty, stale, failed or worktree-only evidence. Read-only access. Integrated fingerprint: {fingerprint}. Analyst findings: {}. Proposed evidence: {}. Return ```compliance_verdict JSON {{\"approved\":true|false,\"note\":\"coverage and gaps\",\"assessments\":[{{\"evidence\":EvidenceProposal,\"approved\":true|false,\"note\":\"independent observation\"}}]}}. Preserve exact proposal fields; runtime owns session identity. Full sources:\n{sources}",serde_json::to_string(&proposed.findings)?,serde_json::to_string(&proposed.evidence)?),cancel)?;
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
}

#[cfg(test)]
#[path = "runtime/tests.rs"]
mod tests;

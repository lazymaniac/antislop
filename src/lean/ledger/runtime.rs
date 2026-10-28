use super::*;

#[path = "runtime/compliance.rs"]
mod compliance;
#[path = "runtime/ledger_change.rs"]
mod ledger_change;
#[path = "runtime/requirements.rs"]
mod requirements;
#[path = "runtime/submissions.rs"]
mod submissions;
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
pub use submissions::{AuditAssessment, AuditEvidence};
pub(crate) use submissions::{audit_approves, audited_tasks, evidence_id, resolve_evidence};

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
        let integration = self.integrating.lock().expect("integration mutex");
        let result = self.apply_checked_ledger_change(proposal_id, diff_hash, change)?;
        self.announce_affected_ledger_tasks(change, &result)?;
        drop(integration);
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
        let workflow = self.store.workflow(conversation)?;
        if !workflow.summary.is_empty() {
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
}

#[cfg(test)]
#[path = "runtime/tests.rs"]
mod tests;

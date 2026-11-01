use super::requirements::RequirementsTurnScope;
use super::*;

#[path = "compliance/audit_conversion.rs"]
mod audit_conversion;
use audit_conversion::{audit_proposal, bind_assessments, decode_audit, decode_audit_review};

/// An audit with the identities the runtime bound its evidence to.
struct Audit {
    approved: bool,
    findings: Vec<serde_json::Value>,
    evidence: Vec<EvidenceProposal>,
}

struct Review {
    approved: bool,
    note: String,
    assessments: Vec<Assessment>,
}

struct AuditScope {
    plan_id: String,
    run_id: String,
    revision: i64,
    fingerprint: String,
    sources: String,
    tasks: Vec<crate::lean::store::Task>,
}

struct AuditTurns {
    producer: String,
    verifier: String,
}

impl Core {
    /// Auditors inspect the primary integrated tree. Worktree approval is never imported.
    pub(in crate::lean) fn compliance_audit(
        self: &Arc<Self>,
        conversation: &str,
        plan: &str,
        cancel: &AtomicBool,
    ) -> Result<Vec<String>> {
        let Some(scope) = self.audit_scope(conversation, plan)? else {
            return Ok(vec!["integrity error: plan has no requirements run".into()]);
        };
        let (proposal, producer) = self.produce_audit(conversation, &scope, cancel)?;
        let (review, verifier) = self.verify_audit(conversation, &scope, &proposal, cancel)?;
        let turns = AuditTurns { producer, verifier };
        self.finish_audit(conversation, plan, &scope, &proposal, &review, &turns)
    }

    fn audit_scope(&self, conversation: &str, plan: &str) -> Result<Option<AuditScope>> {
        let Some(run_id) = self.store.plan_run(plan)? else {
            return Ok(None);
        };
        Ok(Some(AuditScope {
            plan_id: plan.to_owned(),
            revision: self.store.run(conversation, &run_id)?.spec_revision,
            fingerprint: self.ledger_fingerprint(conversation)?,
            sources: self.full_ledger_sources(conversation, &run_id)?,
            tasks: self.store.run_tasks(conversation, &run_id)?,
            run_id,
        }))
    }

    fn produce_audit(
        self: &Arc<Self>,
        conversation: &str,
        scope: &AuditScope,
        cancel: &AtomicBool,
    ) -> Result<(Audit, String)> {
        let prompt = audit_prompt(&audit_tasks(&self.store, &scope.tasks)?, &scope.sources)?;
        let (answer, producer) = self.requirements_turn(
            RequirementsTurnScope {
                conversation,
                run_id: &scope.run_id,
                phase: "audit",
                plan_id: Some(&scope.plan_id),
            },
            false,
            prompt,
            cancel,
        )?;
        let (findings, evidence) = decode_audit(&answer)?;
        let entries = self
            .store
            .ledger_entries(conversation, &scope.run_id, None)?;
        let proposal = audit_proposal(findings, &evidence, &entries, scope, &producer)?;
        Ok((proposal, producer))
    }

    fn verify_audit(
        self: &Arc<Self>,
        conversation: &str,
        scope: &AuditScope,
        proposal: &Audit,
        cancel: &AtomicBool,
    ) -> Result<(Review, String)> {
        let prompt = review_prompt(scope, proposal, &audit_tasks(&self.store, &scope.tasks)?)?;
        let (answer, verifier) = self.requirements_turn(
            RequirementsTurnScope {
                conversation,
                run_id: &scope.run_id,
                phase: "audit",
                plan_id: Some(&scope.plan_id),
            },
            true,
            prompt,
            cancel,
        )?;
        let (note, assessments) = decode_audit_review(&answer)?;
        let assessments = bind_assessments(assessments, &proposal.evidence)?;
        let review = Review {
            approved: assessments.len() == proposal.evidence.len()
                && assessments.iter().all(|assessment| assessment.approved),
            note,
            assessments,
        };
        Ok((review, verifier))
    }

    fn finish_audit(
        &self,
        conversation: &str,
        plan: &str,
        scope: &AuditScope,
        proposal: &Audit,
        review: &Review,
        turns: &AuditTurns,
    ) -> Result<Vec<String>> {
        let _integration = self.integrating.lock().expect("integration mutex");
        self.check_audit_scope(conversation, scope)?;
        self.save_audit_evidence(conversation, scope, proposal, review, &turns.verifier)?;
        self.save_audit_review(
            conversation,
            scope,
            proposal,
            &turns.producer,
            review,
            &turns.verifier,
        )?;
        self.complete_audit(conversation, plan, scope)
    }

    fn check_audit_scope(&self, conversation: &str, scope: &AuditScope) -> Result<()> {
        require(
            self.ledger_fingerprint(conversation)? == scope.fingerprint,
            "integrated files changed during compliance audit",
        )?;
        require(
            self.store.run(conversation, &scope.run_id)?.spec_revision == scope.revision,
            "scope changed during compliance audit",
        )
    }

    fn save_audit_evidence(
        &self,
        conversation: &str,
        scope: &AuditScope,
        proposal: &Audit,
        review: &Review,
        verifier: &str,
    ) -> Result<()> {
        for assessment in &review.assessments {
            require(
                proposal.evidence.contains(&assessment.evidence),
                "verifier assessed evidence not proposed by this audit",
            )?;
            self.store
                .add_ledger_evidence(conversation, &scope.run_id, verifier, assessment)?;
        }
        Ok(())
    }

    fn save_audit_review(
        &self,
        conversation: &str,
        scope: &AuditScope,
        proposal: &Audit,
        producer: &str,
        review: &Review,
        verifier: &str,
    ) -> Result<()> {
        self.store.record_ledger_review(
            conversation,
            &scope.run_id,
            scope.revision,
            "audit",
            producer,
            verifier,
            &scope.fingerprint,
            proposal.approved && review.approved,
            &review.note,
        )
    }

    fn complete_audit(
        &self,
        conversation: &str,
        plan: &str,
        scope: &AuditScope,
    ) -> Result<Vec<String>> {
        let gaps = self
            .store
            .requirements_gaps(conversation, &scope.run_id, &scope.fingerprint)?;
        self.store.set_run_completion(
            conversation,
            &scope.run_id,
            scope.revision,
            gaps.is_empty(),
        )?;
        self.store.append_event(
            conversation,
            None,
            Some(plan),
            "requirements_complete",
            &serde_json::json!({"run":scope.run_id,"spec_revision":scope.revision,"approved":gaps.is_empty(),"gaps":gaps}),
        )?;
        Ok(gaps)
    }
}

fn audit_prompt(tasks: &[serde_json::Value], sources: &str) -> Result<String> {
    Ok(format!(
        "Audit the integrated project against every full source and the requirements and their task evidence. Search for obligations omitted in extraction. Inspect actual files and run relevant read-only checks. Do not change files. Tasks of the plan: {}. Summaries are abridged. Read each relevant approved task's full result with read_dependency_result(dependency=result_id), following its cursor to the end. A text-only analysis is delivered in that result; do not require a repository file for it. Verify code and file artifacts in the integrated tree; a worker report alone does not prove code was integrated or checks passed.\n\nSubmit the audit with submit_audit: findings for each problem (severity, issue, where; critical, high and medium reject), and evidence for each acceptance criterion you checked (criterion_id, the task_id that delivered it, and result: the concrete checks you made and what you observed, never just done). Include user-prescribed methods and steps and composite integration criteria. Evidence must concern this integrated tree; the runtime records which tree you audited. Full sources:\n{}",
        serde_json::to_string(tasks)?,
        sources,
    ))
}

/// What the auditor needs of each task to name it in evidence, and what it did.
fn audit_tasks(
    store: &crate::lean::store::Store,
    tasks: &[crate::lean::store::Task],
) -> Result<Vec<serde_json::Value>> {
    audited_tasks(tasks)
        .map(|task| {
            let summary = if task.status == crate::lean::tasks::status::APPROVED {
                Some(store.task_summary(task)?)
            } else {
                None
            };
            Ok(serde_json::json!({
                "task_id": task.spec["id"],
                "result_id": (task.status == crate::lean::tasks::status::APPROVED).then_some(&task.id),
                "result_access": "read_dependency_result",
                "change_kind": task.spec["change_kind"],
                "outputs": task.spec["outputs"],
                "title": task.spec["title"],
                "goal": task.spec["goal"],
                "covers": task.spec["criterion_ids"],
                "status": task.status,
                "summary": summary,
            }))
        })
        .collect()
}

fn review_prompt(
    scope: &AuditScope,
    proposal: &Audit,
    tasks: &[serde_json::Value],
) -> Result<String> {
    let evidence = proposal
        .evidence
        .iter()
        .enumerate()
        .map(|(index, evidence)| {
            serde_json::json!({
                "id": evidence_id(index),
                "criterion_id": evidence.criterion_id,
                "task_id": evidence.task_id,
                "result": evidence.result,
                "artifact": evidence.artifact,
            })
        })
        .collect::<Vec<_>>();
    Ok(format!(
        "Verify this audit independently against ALL full sources, the requirements and their task evidence and actual integrated files. Find omitted obligations. Independently reproduce or check every evidence result. Reject empty, stale, failed or worktree-only evidence. Read-only access. Task results (summaries are abridged): {}. Independently read relevant full results with read_dependency_result(dependency=result_id), following every page. Text-only deliverables live in these results; they need not exist as files. Reports do not replace verification of integrated code or file artifacts. Analyst findings: {}.\nProposed evidence (assess each by its id):\n{}\n\nSubmit your review with submit_audit_review: a note on coverage and gaps, and one assessment per proposed evidence id (approved, and a note of what you observed). Full sources:\n{}",
        serde_json::to_string(tasks)?,
        serde_json::to_string(&proposal.findings)?,
        serde_json::to_string(&evidence)?,
        scope.sources,
    ))
}

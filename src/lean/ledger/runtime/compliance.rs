use super::*;

use crate::lean::turn_result::TurnResult;

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
        let prompt = audit_prompt(scope)?;
        let (answer, producer) =
            self.requirements_turn(conversation, &scope.run_id, "audit", false, prompt, cancel)?;
        let TurnResult::Audit { findings, evidence } = TurnResult::decode(&answer) else {
            return Err(Error::Invalid(
                "the requirements auditor ended its turn without submit_audit".into(),
            ));
        };
        let entries = self
            .store
            .ledger_entries(conversation, &scope.run_id, None)?;
        let evidence = evidence
            .iter()
            .map(|item| {
                let mut proposal = resolve_evidence(item, &entries, &scope.tasks)?;
                proposal.producer_turn = producer.clone();
                proposal.integrated_fingerprint = scope.fingerprint.clone();
                Ok(proposal)
            })
            .collect::<Result<Vec<_>>>()?;
        let proposal = Audit {
            approved: audit_approves(&findings),
            findings,
            evidence,
        };
        Ok((proposal, producer))
    }

    fn verify_audit(
        self: &Arc<Self>,
        conversation: &str,
        scope: &AuditScope,
        proposal: &Audit,
        cancel: &AtomicBool,
    ) -> Result<(Review, String)> {
        let prompt = review_prompt(scope, proposal)?;
        let (answer, verifier) =
            self.requirements_turn(conversation, &scope.run_id, "audit", true, prompt, cancel)?;
        let TurnResult::AuditReview { note, assessments } = TurnResult::decode(&answer) else {
            return Err(Error::Invalid(
                "the audit verifier ended its turn without submit_audit_review".into(),
            ));
        };
        let assessments = assessments
            .into_iter()
            .map(|assessment| {
                let evidence = (0..proposal.evidence.len())
                    .find(|index| evidence_id(*index) == assessment.evidence)
                    .map(|index| proposal.evidence[index].clone())
                    .ok_or_else(|| {
                        Error::Invalid(format!(
                            "the audit verifier assessed {}, which the audit did not propose",
                            assessment.evidence
                        ))
                    })?;
                Ok(Assessment {
                    evidence,
                    approved: assessment.approved,
                    note: assessment.note,
                })
            })
            .collect::<Result<Vec<_>>>()?;
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

fn audit_prompt(scope: &AuditScope) -> Result<String> {
    Ok(format!(
        "Audit the integrated project against every full source and the requirements and their task evidence. Search for obligations omitted in extraction. Inspect actual files and run relevant read-only checks. Do not change files. Tasks of the plan: {}.\n\nSubmit the audit with submit_audit: findings for each problem (severity, issue, where; critical, high and medium reject), and evidence for each acceptance criterion you checked (criterion_id, the task_id that delivered it, and result: the concrete checks you made and what you observed, never just done). Include user-prescribed methods and steps and composite integration criteria. Evidence must concern this integrated tree; the runtime records which tree you audited. Full sources:\n{}",
        serde_json::to_string(&audit_tasks(&scope.tasks))?,
        scope.sources,
    ))
}

/// What the auditor needs of each task to name it in evidence.
fn audit_tasks(tasks: &[crate::lean::store::Task]) -> Vec<serde_json::Value> {
    audited_tasks(tasks)
        .map(|task| {
            serde_json::json!({
                "task_id": task.spec["id"],
                "title": task.spec["title"],
                "goal": task.spec["goal"],
                "covers": task.spec["criterion_ids"],
                "status": task.status,
            })
        })
        .collect()
}

fn review_prompt(scope: &AuditScope, proposal: &Audit) -> Result<String> {
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
        "Verify this audit independently against ALL full sources, the requirements and their task evidence and actual integrated files. Find omitted obligations. Independently reproduce or check every evidence result. Reject empty, stale, failed or worktree-only evidence. Read-only access. Analyst findings: {}.\nProposed evidence (assess each by its id):\n{}\n\nSubmit your review with submit_audit_review: a note on coverage and gaps, and one assessment per proposed evidence id (approved, and a note of what you observed). Full sources:\n{}",
        serde_json::to_string(&proposal.findings)?,
        serde_json::to_string(&evidence)?,
        scope.sources,
    ))
}

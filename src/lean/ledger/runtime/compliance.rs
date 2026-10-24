use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Audit {
    approved: bool,
    findings: Vec<serde_json::Value>,
    evidence: Vec<EvidenceProposal>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
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
        let proposal = parse_audit(&answer, &producer, &scope.fingerprint)?;
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
        Ok((parse_block(&answer, "compliance_verdict")?, verifier))
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
        "Audit the integrated project against every full source and the requirements and their task evidence. Search for obligations omitted in extraction. Inspect actual files and run relevant read-only checks. Do not change files. Integrated fingerprint: {}. Task inputs and reports: {}. Return ```compliance JSON {{\"approved\":true|false,\"findings\":[...],\"evidence\":[EvidenceProposal...]}}. Each evidence proposal: entry_id,entry_version,criterion_id,task_id (UUID),generation,attempt (round),producer_turn (leave empty; runtime supplies),result (concrete checks and observed results, never just done),artifact (optional),integrated_fingerprint. Include user-prescribed methods and steps and composite integration criteria. Evidence must concern this integrated tree. Full sources:\n{}",
        scope.fingerprint,
        serde_json::to_string(&scope.tasks)?,
        scope.sources,
    ))
}

fn parse_audit(answer: &str, producer: &str, fingerprint: &str) -> Result<Audit> {
    let mut proposal: Audit = parse_block(answer, "compliance")?;
    for evidence in &mut proposal.evidence {
        evidence.producer_turn = producer.to_owned();
        require(
            evidence.integrated_fingerprint == fingerprint,
            "auditor inspected a different integrated result",
        )?;
    }
    Ok(proposal)
}

fn review_prompt(scope: &AuditScope, proposal: &Audit) -> Result<String> {
    Ok(format!(
        "Verify this audit independently against ALL full sources, the requirements and their task evidence and actual integrated files. Find omitted obligations. Independently reproduce/check every evidence result. Reject empty, stale, failed or worktree-only evidence. Read-only access. Integrated fingerprint: {}. Analyst findings: {}. Proposed evidence: {}. Return ```compliance_verdict JSON {{\"approved\":true|false,\"note\":\"coverage and gaps\",\"assessments\":[{{\"evidence\":EvidenceProposal,\"approved\":true|false,\"note\":\"independent observation\"}}]}}. Preserve exact proposal fields; runtime owns session identity. Full sources:\n{}",
        scope.fingerprint,
        serde_json::to_string(&proposal.findings)?,
        serde_json::to_string(&proposal.evidence)?,
        scope.sources,
    ))
}

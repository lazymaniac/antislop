use super::*;
use crate::lean::turn_result::TurnResult;

pub(super) fn decode_audit(answer: &str) -> Result<(Vec<serde_json::Value>, Vec<AuditEvidence>)> {
    match TurnResult::decode(answer) {
        TurnResult::Audit { findings, evidence } => Ok((findings, evidence)),
        _ => Err(Error::Invalid(
            "the requirements auditor ended its turn without submit_audit".into(),
        )),
    }
}

pub(super) fn decode_audit_review(answer: &str) -> Result<(String, Vec<AuditAssessment>)> {
    match TurnResult::decode(answer) {
        TurnResult::AuditReview { note, assessments } => Ok((note, assessments)),
        _ => Err(Error::Invalid(
            "the audit verifier ended its turn without submit_audit_review".into(),
        )),
    }
}

pub(super) fn audit_proposal(
    findings: Vec<serde_json::Value>,
    submitted: &[AuditEvidence],
    entries: &[LedgerEntry],
    scope: &AuditScope,
    producer: &str,
) -> Result<Audit> {
    let evidence = submitted
        .iter()
        .map(|item| bind_audit_evidence(item, entries, &scope.tasks, &scope.fingerprint, producer))
        .collect::<Result<Vec<_>>>()?;
    Ok(Audit {
        approved: audit_approves(&findings),
        findings,
        evidence,
    })
}

pub(super) fn bind_audit_evidence(
    item: &AuditEvidence,
    entries: &[LedgerEntry],
    tasks: &[crate::lean::store::Task],
    fingerprint: &str,
    producer: &str,
) -> Result<EvidenceProposal> {
    let mut proposal = resolve_evidence(item, entries, tasks)?;
    proposal.producer_turn = producer.to_owned();
    proposal.integrated_fingerprint = fingerprint.to_owned();
    Ok(proposal)
}

pub(super) fn bind_assessments(
    submitted: Vec<AuditAssessment>,
    evidence: &[EvidenceProposal],
) -> Result<Vec<Assessment>> {
    submitted
        .into_iter()
        .map(|assessment| bind_assessment(assessment, evidence))
        .collect()
}

pub(super) fn bind_assessment(
    assessment: AuditAssessment,
    evidence: &[EvidenceProposal],
) -> Result<Assessment> {
    let proposal = evidence
        .iter()
        .enumerate()
        .find(|(index, _)| evidence_id(*index) == assessment.evidence)
        .map(|(_, evidence)| evidence.clone())
        .ok_or_else(|| {
            Error::Invalid(format!(
                "the audit verifier assessed {}, which the audit did not propose",
                assessment.evidence
            ))
        })?;
    Ok(Assessment {
        evidence: proposal,
        approved: assessment.approved,
        note: assessment.note,
    })
}

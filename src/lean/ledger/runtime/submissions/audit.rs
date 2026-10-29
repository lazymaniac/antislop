use super::*;
use serde::{Deserialize, Serialize};
/// One piece of evidence the audit analyst proposes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditEvidence {
    pub criterion_id: String,
    /// The task's id in the plan, or its runtime id.
    pub task_id: String,
    pub result: String,
    #[serde(default)]
    pub artifact: Option<String>,
}

/// The audit verifier's assessment of one proposed piece of evidence, by
/// the id the runtime gave it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditAssessment {
    pub evidence: String,
    pub approved: bool,
    pub note: String,
}

/// Severities that keep an audit from approving.
const BLOCKING: [&str; 3] = ["critical", "high", "medium"];

/// Whether an audit with these findings approves.
pub(crate) fn audit_approves(findings: &[serde_json::Value]) -> bool {
    !findings
        .iter()
        .any(|finding| BLOCKING.contains(&finding["severity"].as_str().unwrap_or("high")))
}

/// The runtime id of the `index`th proposed evidence.
pub(crate) fn evidence_id(index: usize) -> String {
    format!("E{}", index + 1)
}

impl Core {
    /// The findings and evidence of `submit_audit`; each piece of evidence
    /// must name a criterion of the run and one of its tasks.
    pub(crate) fn submitted_audit(
        &self,
        conversation: &str,
        args: &serde_json::Value,
    ) -> Result<(Vec<serde_json::Value>, Vec<AuditEvidence>)> {
        let findings = plain_findings(&args["findings"])?;
        let evidence: Vec<AuditEvidence> = match &args["evidence"] {
            serde_json::Value::Null => Vec::new(),
            value => serde_json::from_value(value.clone())
                .map_err(|error| Error::Invalid(format!("evidence: {error}")))?,
        };
        if let Some(run) = self.store.planning_run(conversation)? {
            let entries = self.store.ledger_entries(conversation, &run.id, None)?;
            let tasks = self.store.run_tasks(conversation, &run.id)?;
            for (index, item) in evidence.iter().enumerate() {
                resolve_evidence(item, &entries, &tasks)
                    .map_err(|error| Error::Invalid(format!("evidence[{index}]: {error}")))?;
            }
        }
        Ok((findings, evidence))
    }

    /// The assessments of `submit_audit_review`: each names one of the
    /// latest audit's evidence ids, and every proposal is assessed.
    pub(crate) fn submitted_audit_review(
        &self,
        conversation: &str,
        args: &serde_json::Value,
    ) -> Result<(String, Vec<AuditAssessment>)> {
        let note = args["note"]
            .as_str()
            .filter(|note| !note.trim().is_empty())
            .ok_or_else(|| Error::Invalid("note: required".into()))?
            .to_owned();
        let assessments: Vec<AuditAssessment> = match &args["assessments"] {
            serde_json::Value::Null => Vec::new(),
            value => serde_json::from_value(value.clone())
                .map_err(|error| Error::Invalid(format!("assessments: {error}")))?,
        };
        let ids = self.proposed_evidence_ids(conversation)?;
        validate_assessments(&ids, &assessments)?;
        Ok((note, assessments))
    }

    fn proposed_evidence_ids(&self, conversation: &str) -> Result<Vec<String>> {
        let proposed = match self
            .store
            .latest_role_result(conversation, "requirements-audit-analyst")?
        {
            Some(crate::lean::turn_result::TurnResult::Audit { evidence, .. }) => evidence.len(),
            _ => 0,
        };
        Ok((0..proposed).map(evidence_id).collect())
    }
}

/// The findings of an audit as the tools take them: severity, issue, where.
pub(crate) fn plain_findings(value: &serde_json::Value) -> Result<Vec<serde_json::Value>> {
    #[derive(Deserialize, Serialize)]
    #[serde(deny_unknown_fields)]
    struct Finding {
        severity: String,
        issue: String,
        #[serde(rename = "where")]
        location: String,
    }
    let findings: Vec<Finding> = match value {
        serde_json::Value::Null => Vec::new(),
        value => serde_json::from_value(value.clone())
            .map_err(|error| Error::Invalid(format!("findings: {error}")))?,
    };
    findings
        .into_iter()
        .map(|finding| {
            require(
                matches!(
                    finding.severity.as_str(),
                    "critical" | "high" | "medium" | "low"
                ),
                "findings.severity: critical, high, medium or low",
            )?;
            Ok(serde_json::to_value(finding)?)
        })
        .collect()
}

/// The run's tasks an audit may cite, oldest plan first: plans the plan
/// verifier sent back never ran.
pub(crate) fn audited_tasks(
    tasks: &[crate::lean::store::Task],
) -> impl DoubleEndedIterator<Item = &crate::lean::store::Task> {
    tasks
        .iter()
        .filter(|task| task.status != crate::lean::tasks::status::REVISED)
}

/// The evidence proposal `item` stands for, with the identities the runtime
/// owns filled in except the producing turn and the integrated fingerprint.
pub(crate) fn resolve_evidence(
    item: &AuditEvidence,
    entries: &[LedgerEntry],
    tasks: &[crate::lean::store::Task],
) -> Result<EvidenceProposal> {
    let entry = evidence_entry(entries, &item.criterion_id)?;
    let task = evidence_task(tasks, &item.task_id)?;
    require(
        !item.result.trim().is_empty(),
        "result: name the checks you made and what you observed",
    )?;
    Ok(EvidenceProposal {
        entry_id: entry.spec.id.clone(),
        entry_version: entry.version,
        criterion_id: item.criterion_id.clone(),
        task_id: task.id.clone(),
        generation: task.input_generation,
        attempt: task.round,
        producer_turn: String::new(),
        result: item.result.clone(),
        artifact: item.artifact.clone(),
        integrated_fingerprint: String::new(),
    })
}

fn evidence_entry<'a>(entries: &'a [LedgerEntry], criterion_id: &str) -> Result<&'a LedgerEntry> {
    entries
        .iter()
        .find(|entry| criterion_belongs(&entry.spec, criterion_id))
        .ok_or_else(|| {
            Error::Invalid(format!(
                "criterion_id {criterion_id} is not an acceptance criterion of these requirements"
            ))
        })
}

fn evidence_task<'a>(
    tasks: &'a [crate::lean::store::Task],
    task_id: &str,
) -> Result<&'a crate::lean::store::Task> {
    // A plan id names the task in the latest plan of the run.
    audited_tasks(tasks)
        .rev()
        .find(|task| task_matches(task, task_id))
        .ok_or_else(|| Error::Invalid(format!("task_id {task_id} is not a task of this plan")))
}

fn criterion_belongs(entry: &EntrySpec, criterion_id: &str) -> bool {
    entry
        .criteria
        .iter()
        .any(|criterion| criterion.id == criterion_id)
}

fn task_matches(task: &crate::lean::store::Task, task_id: &str) -> bool {
    task.id == task_id || task.spec["id"] == task_id
}

fn validate_assessments(ids: &[String], assessments: &[AuditAssessment]) -> Result<()> {
    let mut seen = std::collections::BTreeSet::new();
    for assessment in assessments {
        validate_assessment(ids, &mut seen, assessment)?;
    }
    let missing: Vec<_> = ids
        .iter()
        .filter(|id| !seen.contains(id.as_str()))
        .collect();
    require(
        missing.is_empty(),
        &format!(
            "assessments: assess every proposed evidence; missing {}",
            missing
                .iter()
                .map(|id| id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    )
}

fn validate_assessment(
    ids: &[String],
    seen: &mut std::collections::BTreeSet<String>,
    assessment: &AuditAssessment,
) -> Result<()> {
    require(
        ids.contains(&assessment.evidence),
        &format!(
            "assessments: {} is not a proposed evidence id; use {}",
            assessment.evidence,
            evidence_ids(ids)
        ),
    )?;
    require(
        seen.insert(assessment.evidence.clone()),
        &format!("assessments: {} is assessed twice", assessment.evidence),
    )?;
    require(
        !assessment.note.trim().is_empty(),
        &format!(
            "assessments ({}): note what you checked",
            assessment.evidence
        ),
    )
}

fn evidence_ids(ids: &[String]) -> String {
    if ids.is_empty() {
        "none, the audit proposed no evidence".into()
    } else {
        ids.join(", ")
    }
}

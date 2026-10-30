use super::tool_results::SubmittedPlan;
use super::types::{Selection, TurnResult};
use crate::lean::tasks::{TaskContract, Verdict};
use crate::{Result, require};

impl TurnResult {
    /// How the gateway hands a result over: a plain report as its bare
    /// text, every other result as JSON.
    pub fn encode(&self) -> Result<String> {
        Ok(match self {
            Self::Report {
                text,
                addressed: None,
            } => text.clone(),
            other => serde_json::to_string(other)?,
        })
    }

    /// What a role without a task contract ended its turn with; the inverse
    /// of [`Self::encode`].
    pub fn decode(text: &str) -> Self {
        serde_json::from_str::<Self>(text)
            .ok()
            .filter(|result| {
                !matches!(
                    result,
                    Self::Report {
                        addressed: None,
                        ..
                    }
                )
            })
            .unwrap_or_else(|| Self::Report {
                text: text.to_owned(),
                addressed: None,
            })
    }

    /// The words of a planning answer and the plan it submitted, if any: its
    /// tasks and the approved tasks it keeps.
    pub fn plan_answer(text: &str) -> (String, Option<SubmittedPlan>) {
        match Self::decode(text) {
            Self::Plan {
                document,
                tasks,
                keep,
                ..
            } => (document, Some((tasks, keep))),
            other => (other.text(), None),
        }
    }

    /// The verdict of a plan, audit or code review; `None` when the reviewer
    /// ended its turn without one.
    pub fn review_verdict(text: &str) -> Option<Verdict> {
        match Self::decode(text) {
            Self::Review { verdict, .. } => Some(verdict),
            _ => None,
        }
    }

    /// The comments this answer says it settled.
    pub fn addressed(&self) -> Option<&[u64]> {
        match self {
            Self::Report { addressed, .. } | Self::Plan { addressed, .. } => addressed.as_deref(),
            _ => None,
        }
    }

    pub fn validate_task(&self, spec: &TaskContract, verifier: bool) -> Result<()> {
        require(
            serde_json::to_vec(self)?.len() <= 256 * 1024,
            "result exceeds 256 KiB; keep the report compact and reference task artifacts",
        )?;
        self.validate_task_shape(spec, verifier)
    }

    fn validate_task_shape(&self, spec: &TaskContract, verifier: bool) -> Result<()> {
        match self {
            Self::Completed {
                summary,
                report,
                selection,
            } if !verifier => validate_completion(summary, report, selection.as_ref(), spec),
            Self::Review { report, verdict } if verifier => validate_review(report, verdict, spec),
            Self::NeedsInput {
                request,
                checkpoint,
            } => validate_waiting(request, checkpoint),
            _ => invalid_task_result(verifier),
        }
    }

    /// The words of this result as the conversation shows them.
    pub fn text(&self) -> String {
        match self {
            Self::Report { text, .. }
            | Self::Plan { document: text, .. }
            | Self::Requirements { markdown: text } => text.clone(),
            Self::Completed { report, .. } | Self::Review { report, .. } => report.clone(),
            Self::NeedsInput { checkpoint, .. } => checkpoint.clone(),
            Self::Question { question } => question.transcript(),
            Self::Ledger { extraction } => {
                format!("Extracted {} requirement(s).", extraction.entries.len())
            }
            Self::Audit { findings, evidence } => format!(
                "Audit: {} finding(s), {} piece(s) of evidence.",
                findings.len(),
                evidence.len()
            ),
            Self::AuditReview { note, .. } => note.clone(),
        }
    }
}

fn validate_completion(
    summary: &str,
    report: &str,
    selection: Option<&Selection>,
    spec: &TaskContract,
) -> Result<()> {
    require(
        !summary.trim().is_empty() && !report.trim().is_empty(),
        "summary and report: required",
    )?;
    match (spec.kind == crate::config::WorkKind::Options, selection) {
        (true, Some(selection)) => selection.validate(),
        (true, None) => Err(crate::Error::Invalid(
            "an options task ends with select, naming the alternatives and the chosen one".into(),
        )),
        (false, Some(_)) => Err(crate::Error::Invalid(
            "select is only for options tasks; end this task with complete".into(),
        )),
        (false, None) => Ok(()),
    }
}

fn validate_review(report: &str, verdict: &Verdict, spec: &TaskContract) -> Result<()> {
    require(!report.trim().is_empty(), "report: required")?;
    let obligations = crate::lean::tasks::obligations(spec);
    validate_coverage(verdict, &obligations)?;
    validate_findings(verdict, &obligations)?;
    validate_complete_coverage(verdict, &obligations)?;
    validate_verdict_consistency(verdict)
}

fn validate_coverage(verdict: &Verdict, obligations: &[(String, String)]) -> Result<()> {
    require(
        verdict
            .coverage
            .keys()
            .all(|id| obligations.iter().any(|(assigned, _)| assigned == id)),
        "checks: use only the check ids this prompt lists",
    )
}

fn validate_findings(verdict: &Verdict, obligations: &[(String, String)]) -> Result<()> {
    let mut ids = std::collections::BTreeSet::new();
    for finding in &verdict.findings {
        validate_finding(finding, obligations, &mut ids)?;
    }
    Ok(())
}

fn validate_finding(
    finding: &serde_json::Value,
    obligations: &[(String, String)],
    ids: &mut std::collections::BTreeSet<String>,
) -> Result<()> {
    let id = required_finding_id(finding)?;
    validate_finding_fields(finding)?;
    require(ids.insert(id.to_owned()), "findings: duplicate id")?;
    require(
        obligations
            .iter()
            .any(|(check, _)| finding["check"] == *check),
        &format!(
            "findings.check: {} is not one of the check ids this prompt lists; name the check each finding belongs to",
            finding["check"]
        ),
    )
}

fn required_finding_id(finding: &serde_json::Value) -> Result<&str> {
    finding["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| crate::Error::Invalid("findings: stable id required".into()))
}

fn validate_finding_fields(finding: &serde_json::Value) -> Result<()> {
    require(
        matches!(
            finding["severity"].as_str(),
            Some("critical" | "high" | "medium" | "low")
        ) && finding["issue"]
            .as_str()
            .is_some_and(|text| !text.trim().is_empty())
            && finding["where"]
                .as_str()
                .is_some_and(|text| !text.trim().is_empty()),
        "findings: severity, issue and where are required",
    )
}

fn validate_waiting(request: &str, checkpoint: &str) -> Result<()> {
    require(
        !request.trim().is_empty() && !checkpoint.trim().is_empty(),
        "ask a question with ask before waiting for its answer",
    )
}

fn invalid_task_result(verifier: bool) -> Result<()> {
    let message = if verifier {
        "a verifier ends its turn with review or wait_for_answer"
    } else {
        "a worker ends its turn with complete (select for an options task) or wait_for_answer"
    };
    Err(crate::Error::Invalid(message.into()))
}

fn validate_complete_coverage(verdict: &Verdict, obligations: &[(String, String)]) -> Result<()> {
    let missing = crate::lean::tasks::unaccounted(verdict, obligations);
    require(
        missing.is_empty(),
        &format!(
            "checks: give a status and note for every listed check; missing: {}",
            missing
                .iter()
                .map(|(id, _)| id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    )
}

fn validate_verdict_consistency(verdict: &Verdict) -> Result<()> {
    require(
        !verdict.approved
            || !verdict.findings.iter().any(|finding| {
                matches!(
                    finding["severity"].as_str(),
                    Some("high" | "medium" | "critical")
                )
            }),
        "verdict: approval contradicts blocking findings",
    )?;
    require(
        !verdict.approved
            || verdict
                .coverage
                .values()
                .all(|assessment| matches!(assessment["status"].as_str(), Some("ok" | "n/a"))),
        "verdict: approval contradicts unsettled checks",
    )
}

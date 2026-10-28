//! What the requirements agents submit through their MCP tools, and what the
//! runtime derives from it: source ranges from quotes, the run and revision,
//! and the identities evidence is bound to. Agents give content and judgement
//! only.
use super::*;
use serde::{Deserialize, Serialize};

/// A requirement as the extraction analyst submits it.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SubmittedEntry {
    id: String,
    /// The user's words the requirement comes from, copied exactly.
    quote: String,
    /// The source the quote is in, when it is not the first that has it.
    #[serde(default)]
    source: Option<String>,
    interpretation: String,
    #[serde(default)]
    qualifiers: Vec<String>,
    #[serde(default)]
    parent: Option<String>,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default = "yes")]
    mandatory: bool,
    #[serde(default)]
    criteria: Vec<Criterion>,
    #[serde(default)]
    task_ids: Vec<String>,
}

fn yes() -> bool {
    true
}

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
    /// The extraction `submit_ledger` makes for the conversation's current
    /// run, checked as the runtime will apply it.
    pub(crate) fn submitted_extraction(
        &self,
        conversation: &str,
        args: &serde_json::Value,
    ) -> Result<Extraction> {
        let run = self
            .store
            .planning_run(conversation)?
            .ok_or_else(|| Error::Invalid("there is no requirements run to extract".into()))?;
        let submitted: Vec<SubmittedEntry> = serde_json::from_value(args["entries"].clone())
            .map_err(|error| Error::Invalid(format!("entries: {error}")))?;
        let sources = self.source_texts(conversation, &run.id)?;
        let existing = self.store.ledger_entries(conversation, &run.id, None)?;
        let entries = submitted
            .into_iter()
            .enumerate()
            .map(|(index, entry)| {
                let position = existing
                    .iter()
                    .find(|old| old.spec.id == entry.id)
                    .map_or((existing.len() + index) as u32, |old| old.spec.position);
                Ok(EntrySpec {
                    source: quoted_span(
                        &sources,
                        &entry.id,
                        &entry.quote,
                        entry.source.as_deref(),
                    )?,
                    id: entry.id,
                    interpretation: entry.interpretation,
                    qualifiers: entry.qualifiers,
                    parent: entry.parent,
                    depends_on: entry.depends_on,
                    mandatory: entry.mandatory,
                    implementation_owners: Vec::new(),
                    verification_owners: Vec::new(),
                    criteria: entry.criteria,
                    task_ids: entry.task_ids,
                    position,
                    disposition: Disposition::Active,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let extraction = Extraction {
            run_id: run.id.clone(),
            expected_spec_revision: run.spec_revision,
            entries,
        };
        self.validate_extraction(conversation, &run, &existing, extraction.clone())?;
        Ok(extraction)
    }

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
        let proposed = match self
            .store
            .latest_role_result(conversation, "requirements-audit-analyst")?
        {
            Some(crate::lean::turn_result::TurnResult::Audit { evidence, .. }) => evidence.len(),
            _ => 0,
        };
        let ids: Vec<String> = (0..proposed).map(evidence_id).collect();
        let mut seen = std::collections::BTreeSet::new();
        for assessment in &assessments {
            require(
                ids.contains(&assessment.evidence),
                &format!(
                    "assessments: {} is not a proposed evidence id; use {}",
                    assessment.evidence,
                    if ids.is_empty() {
                        "none, the audit proposed no evidence".to_owned()
                    } else {
                        ids.join(", ")
                    }
                ),
            )?;
            require(
                seen.insert(assessment.evidence.as_str()),
                &format!("assessments: {} is assessed twice", assessment.evidence),
            )?;
            require(
                !assessment.note.trim().is_empty(),
                &format!(
                    "assessments ({}): note what you checked",
                    assessment.evidence
                ),
            )?;
        }
        let missing: Vec<&str> = ids
            .iter()
            .filter(|id| !seen.contains(id.as_str()))
            .map(String::as_str)
            .collect();
        require(
            missing.is_empty(),
            &format!(
                "assessments: assess every proposed evidence; missing {}",
                missing.join(", ")
            ),
        )?;
        Ok((note, assessments))
    }

    fn source_texts(&self, conversation: &str, run_id: &str) -> Result<Vec<(String, String)>> {
        self.store
            .ledger_sources(conversation, run_id)?
            .into_iter()
            .map(|source| {
                let text =
                    self.store
                        .ledger_source_text(conversation, run_id, &source.id, &self.state)?;
                Ok((source.id, text))
            })
            .collect()
    }
}

/// Where in the sources `quote` stands: in the named source, or else the
/// first that has it (a later message may repeat the user's words, as an
/// approval does); there it must occur once, so the range is unambiguous.
fn quoted_span(
    sources: &[(String, String)],
    entry: &str,
    quote: &str,
    source: Option<&str>,
) -> Result<SourceSpan> {
    let quote = quote.trim();
    require(
        !quote.is_empty(),
        &format!("{entry}: quote the user's words the requirement comes from"),
    )?;
    let named = |id: &String| source.is_none_or(|source| source == id);
    if let Some(source) = source {
        require(
            sources.iter().any(|(id, _)| id == source),
            &format!(
                "{entry}: source {source} is not one of {}",
                sources
                    .iter()
                    .map(|(id, _)| id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )?;
    }
    let Some((id, text)) = sources
        .iter()
        .find(|(id, text)| named(id) && text.contains(quote))
    else {
        return Err(Error::Invalid(format!(
            "{entry}: the quote does not occur in {}; copy the user's words exactly",
            source.unwrap_or("any source")
        )));
    };
    // Count overlapping occurrences too: `aaa` occurs twice in `aaaa`.
    let found: Vec<usize> = text
        .char_indices()
        .map(|(start, _)| start)
        .filter(|&start| text[start..].starts_with(quote))
        .collect();
    require(
        found.len() == 1,
        &format!(
            "{entry}: the quote occurs {} times in source {id}; quote a longer passage so it names one place",
            found.len()
        ),
    )?;
    let span = SourceSpan {
        source_id: id.clone(),
        start: found[0],
        end: found[0] + quote.len(),
    };
    span.text(text).map_err(|_| {
        Error::Invalid(format!(
            "{entry}: the quote cuts through a character; quote whole words"
        ))
    })?;
    Ok(span)
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

/// The evidence proposal `item` stands for, with the identities the runtime
/// owns filled in except the producing turn and the integrated fingerprint.
pub(crate) fn resolve_evidence(
    item: &AuditEvidence,
    entries: &[LedgerEntry],
    tasks: &[crate::lean::store::Task],
) -> Result<EvidenceProposal> {
    let entry = entries
        .iter()
        .find(|entry| {
            entry
                .spec
                .criteria
                .iter()
                .any(|criterion| criterion.id == item.criterion_id)
        })
        .ok_or_else(|| {
            Error::Invalid(format!(
                "criterion_id {} is not an acceptance criterion of these requirements",
                item.criterion_id
            ))
        })?;
    let task = tasks
        .iter()
        .find(|task| task.id == item.task_id || task.spec["id"] == item.task_id.as_str())
        .ok_or_else(|| {
            Error::Invalid(format!(
                "task_id {} is not a task of this plan",
                item.task_id
            ))
        })?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quote_names_one_place_in_the_sources() {
        let sources = vec![
            ("s1".to_owned(), "Add a 👩‍💻 badge. Keep the API.".to_owned()),
            ("s2".to_owned(), "Keep the API stable.".to_owned()),
        ];
        let span = quoted_span(&sources, "r1", " Add a 👩‍💻 badge.", None).unwrap();
        assert_eq!((span.source_id.as_str(), span.start), ("s1", 0));
        assert_eq!(span.text(&sources[0].1).unwrap(), "Add a 👩‍💻 badge.");
        // A later source repeating the words does not make them ambiguous.
        let first = quoted_span(&sources, "r2", "Keep the API", None).unwrap();
        assert_eq!((first.source_id.as_str(), first.start), ("s1", 25));
        let named = quoted_span(&sources, "r2", "Keep the API", Some("s2")).unwrap();
        assert_eq!((named.source_id.as_str(), named.start), ("s2", 0));
        let twice = quoted_span(&sources, "r3", "e", None).unwrap_err();
        assert!(twice.to_string().contains("times in source s1"), "{twice}");
        let missing = quoted_span(&sources, "r4", "Remove the API", None).unwrap_err();
        assert!(missing.to_string().contains("does not occur"), "{missing}");
        assert!(quoted_span(&sources, "r5", "Keep", Some("s9")).is_err());
        assert!(quoted_span(&sources, "r6", " ", None).is_err());
    }

    #[test]
    fn overlapping_quotes_are_ambiguous() {
        for (text, quote) in [("aaaa", "aaa"), ("ążążąż", "ążąż")] {
            let sources = vec![("s1".to_owned(), text.to_owned())];
            let error = quoted_span(&sources, "r1", quote, None).unwrap_err();
            assert!(
                error.to_string().contains("2 times in source s1"),
                "{error}"
            );
        }
    }

    #[test]
    fn an_audit_approves_unless_a_finding_blocks() {
        let low = plain_findings(&serde_json::json!([
            {"severity": "low", "issue": "naming", "where": "README"}
        ]))
        .unwrap();
        assert!(audit_approves(&low));
        let high = plain_findings(&serde_json::json!([
            {"severity": "high", "issue": "missing", "where": "R1"}
        ]))
        .unwrap();
        assert!(!audit_approves(&high));
        assert!(plain_findings(&serde_json::json!([{"issue": "x"}])).is_err());
    }
}

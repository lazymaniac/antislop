//! Typed task results. Prose is evidence, never a control protocol.
use super::tasks::{TaskContract, Verdict};
use crate::{Result, require};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum TurnResult {
    Report {
        text: String,
    },
    Completed {
        summary: String,
        report: String,
        outputs: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        selection: Option<Selection>,
    },
    Review {
        report: String,
        verdict: Verdict,
    },
    NeedsInput {
        request: String,
        checkpoint: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub criteria: Vec<String>,
    pub alternatives: Vec<Alternative>,
    pub selected: String,
    pub rationale: String,
    pub reconsider_when: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Alternative {
    pub id: String,
    pub approach: String,
    pub evaluations: Vec<String>,
    pub eligible: bool,
    pub tradeoffs: String,
}

impl Selection {
    pub fn validate(&self) -> Result<()> {
        require(
            (2..=4).contains(&self.alternatives.len()),
            "selection.alternatives: compare 2–4 variants",
        )?;
        require(
            !self.criteria.is_empty() && self.criteria.iter().all(|c| !c.trim().is_empty()),
            "selection.criteria: shared criteria are required",
        )?;
        let mut ids = std::collections::BTreeSet::new();
        for alternative in &self.alternatives {
            require(
                !alternative.id.trim().is_empty() && ids.insert(&alternative.id),
                "selection.alternatives: duplicate id",
            )?;
            require(
                !alternative.approach.trim().is_empty(),
                "selection.alternatives.approach: required",
            )?;
            require(
                alternative.evaluations.len() == self.criteria.len()
                    && alternative.evaluations.iter().all(|e| !e.trim().is_empty()),
                &format!(
                    "selection.alternatives.evaluations: alternative {:?} has {} evaluations for {} criteria; give exactly one non-empty evaluation per criterion, in the order of selection.criteria",
                    alternative.id,
                    alternative.evaluations.len(),
                    self.criteria.len()
                ),
            )?;
        }
        require(
            self.alternatives
                .iter()
                .any(|a| a.id == self.selected && a.eligible),
            "selection.selected: select an eligible alternative",
        )?;
        require(
            !self.rationale.trim().is_empty() && !self.reconsider_when.trim().is_empty(),
            "selection: rationale and reconsider_when are required",
        )
    }
}

impl TurnResult {
    pub fn validate_task(&self, spec: &TaskContract, verifier: bool) -> Result<()> {
        require(
            serde_json::to_vec(self)?.len() <= 256 * 1024,
            "result exceeds 256 KiB; keep the report compact and reference task artifacts",
        )?;
        match (self, verifier) {
            (
                Self::Completed {
                    summary,
                    report,
                    outputs,
                    selection,
                },
                false,
            ) => {
                require(
                    !summary.trim().is_empty() && !report.trim().is_empty(),
                    "summary and report: required",
                )?;
                let missing: Vec<_> = spec
                    .outputs
                    .iter()
                    .filter(|key| !outputs.contains_key(*key))
                    .collect();
                let unexpected: Vec<_> = outputs
                    .keys()
                    .filter(|key| !spec.outputs.contains(key))
                    .collect();
                require(
                    missing.is_empty() && unexpected.is_empty(),
                    &format!(
                        "outputs: must match the task's declared outputs {:?}; missing {missing:?}, not declared {unexpected:?}",
                        spec.outputs
                    ),
                )?;
                if spec.kind == crate::config::WorkKind::Options {
                    selection
                        .as_ref()
                        .ok_or_else(|| {
                            crate::Error::Invalid("selection: required for options".into())
                        })?
                        .validate()?;
                }
                Ok(())
            }
            (Self::Review { report, verdict }, true) => {
                require(!report.trim().is_empty(), "review.report: required")?;
                let obligations = super::tasks::obligations(spec);
                require(
                    verdict
                        .coverage
                        .keys()
                        .all(|id| obligations.iter().any(|(assigned, _)| assigned == id)),
                    "verdict.coverage contains checks outside the local contract",
                )?;
                let mut finding_ids = std::collections::BTreeSet::new();
                for finding in &verdict.findings {
                    let id = finding["id"]
                        .as_str()
                        .filter(|id| !id.is_empty())
                        .ok_or_else(|| {
                            crate::Error::Invalid("verdict.findings.id: stable id required".into())
                        })?;
                    require(
                        matches!(
                            finding["severity"].as_str(),
                            Some("critical" | "high" | "medium" | "low")
                        ) && finding["issue"]
                            .as_str()
                            .is_some_and(|s| !s.trim().is_empty())
                            && finding["where"]
                                .as_str()
                                .is_some_and(|s| !s.trim().is_empty()),
                        "verdict.findings: severity, issue and where are required",
                    )?;
                    require(finding_ids.insert(id), "verdict.findings: duplicate id")?;
                    require(
                        obligations.iter().any(|(id, _)| finding["check"] == *id),
                        "verdict.findings.check: must reference an assigned check",
                    )?;
                }
                require(
                    super::tasks::unaccounted(verdict, &obligations).is_empty(),
                    "verdict.coverage: assess each assigned criterion and risk",
                )?;
                require(
                    !verdict.approved
                        || !verdict.findings.iter().any(|f| {
                            matches!(f["severity"].as_str(), Some("high" | "medium" | "critical"))
                        }),
                    "verdict: approval contradicts blocking findings",
                )?;
                require(
                    !verdict.approved
                        || verdict.coverage.values().all(|assessment| {
                            matches!(assessment["status"].as_str(), Some("ok" | "n/a"))
                        }),
                    "verdict: approval contradicts unsettled checks",
                )
            }
            (
                Self::NeedsInput {
                    request,
                    checkpoint,
                },
                _,
            ) => require(
                !request.trim().is_empty() && !checkpoint.trim().is_empty(),
                "request and checkpoint: required",
            ),
            _ => Err(crate::Error::Invalid(
                "result.type: use completed, review or needs_input for this task role".into(),
            )),
        }
    }

    pub fn text(&self) -> &str {
        match self {
            Self::Report { text } => text,
            Self::Completed { report, .. } | Self::Review { report, .. } => report,
            Self::NeedsInput { checkpoint, .. } => checkpoint,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn task() -> TaskContract {
        serde_json::from_value(json!({"id":"local","title":"API","goal":"Define API","kind":"implementation","outputs":["api"],"verifier":{"focus":"Check API","risk_ids":["R021"]}})).unwrap()
    }

    #[test]
    fn completion_requires_the_declared_outputs_and_correct_role() {
        let result: TurnResult = serde_json::from_value(json!({"type":"completed","summary":"API","report":"Defined","outputs":{"api":"Result<T>"}})).unwrap();
        result.validate_task(&task(), false).unwrap();
        assert!(result.validate_task(&task(), true).is_err());
        let result: TurnResult = serde_json::from_value(json!({"type":"completed","summary":"API","report":"Defined","outputs":{"extra":"data"}})).unwrap();
        assert!(result.validate_task(&task(), false).is_err());
        assert!(serde_json::from_value::<TurnResult>(json!({"text":"done"})).is_err());
    }

    #[test]
    fn review_must_account_for_each_selected_local_risk() {
        let result: TurnResult = serde_json::from_value(
            json!({"type":"review","report":"Reviewed","verdict":{"approved":true,"findings":[]}}),
        )
        .unwrap();
        assert!(result.validate_task(&task(), true).is_err());
    }

    #[test]
    fn ineligible_and_duplicate_variants_cannot_be_selected() {
        let mut choice = Selection {
            criteria: vec!["Correctness".into()],
            alternatives: vec![
                Alternative {
                    id: "A".into(),
                    approach: "Reuse".into(),
                    evaluations: vec!["Passes".into()],
                    eligible: true,
                    tradeoffs: "Less flexibility".into(),
                },
                Alternative {
                    id: "B".into(),
                    approach: "Replace".into(),
                    evaluations: vec!["Fails".into()],
                    eligible: false,
                    tradeoffs: "Changes contracts".into(),
                },
            ],
            selected: "A".into(),
            rationale: "Preserves contracts".into(),
            reconsider_when: "Contracts change".into(),
        };
        choice.validate().unwrap();
        choice.selected = "B".into();
        assert!(choice.validate().is_err());
        choice.alternatives[1].id = "A".into();
        assert!(choice.validate().is_err());
    }
}

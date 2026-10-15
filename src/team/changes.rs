//! Reviewed change classification and before/after proof obligations for software tasks.
use super::{CatalogRole, TaskRecord, checks::CheckPhase};
use crate::{Result, domain::*, hash, quality::TaskFamily, require};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const PLAN_CONTRACT: &str = "Bugfix proof and acceptance coverage are distinct. Each bugfix lists the assigned criteria that describe that original defect; BOTH referenced before_execution and before_verification checks must cover EVERY such criterion. The after check requires expected_exit_code 0. A pure bugfix task must account for every assigned criterion either through a bugfix or through non_defect_criteria: a map from an assigned criterion ID to a specific rationale explaining why it does not describe an original defect (for example an approval or delivery obligation). The sets must be disjoint and no criterion may be dropped. Do not classify a defect criterion as non-defect to avoid reproducing it. Non-defect classification never establishes fulfillment: all ordinary task, requirement, runner and quality checks still apply. TASK_CHANGE and NON_DEFECT_PLAN reviewers independently assess the rationale against original requirements. The task verifier must return non_defect_assessments for exactly those keys, with its own assessment and current evidence_ids already included in classification_evidence_ids; CHANGE:non_defect checks must pass with that evidence. Do not claim a test command proves policy approvals or other behavior outside its observations. Missing reproduction still requires a performed investigation and justified alternative proof.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Feature,
    Bugfix,
    Refactor,
    Maintenance,
    Mixed,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangeContract {
    pub kind: ChangeKind,
    pub rationale: String,
    pub source_refs: BTreeSet<String>,
    pub bugfixes: Vec<Bugfix>,
    /// Assigned acceptance obligations that do not describe an original defect.
    /// This classification never waives their ordinary completion checks.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub non_defect_criteria: BTreeMap<String, String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Bugfix {
    pub id: String,
    pub defect: String,
    pub criterion_ids: BTreeSet<String>,
    pub oracle: String,
    pub before: BeforeProof,
    pub after_check_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BeforeProof {
    Reproduction {
        check_id: String,
    },
    Alternative {
        investigation_check_id: String,
        unavailable_reason: String,
        method: String,
        source_refs: BTreeSet<String>,
    },
}
impl BeforeProof {
    pub fn check_id(&self) -> &str {
        match self {
            Self::Reproduction { check_id } => check_id,
            Self::Alternative {
                investigation_check_id,
                ..
            } => investigation_check_id,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangeReview {
    pub contract_hash: String,
    pub kind: ChangeKind,
    pub classification_assessment: String,
    pub classification_evidence_ids: BTreeSet<String>,
    pub bugfixes: Vec<BugfixReview>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub non_defect_assessments: BTreeMap<String, NonDefectAssessment>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NonDefectAssessment {
    /// Explain independently why this criterion does not describe an original
    /// defect. This is classification, never proof of criterion fulfillment.
    pub assessment: String,
    /// Current source evidence also cited by classification_evidence_ids.
    pub evidence_ids: BTreeSet<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BugfixReview {
    pub id: String,
    pub before_evidence_ids: BTreeSet<String>,
    pub after_evidence_ids: BTreeSet<String>,
    pub before_assessment: String,
    pub independent_oracle: String,
    pub counterexample: String,
    pub alternative_assessment: Option<String>,
}
fn text(value: &str) -> bool {
    !value.trim().is_empty()
}
pub fn required(task: &TaskRecord, role: CatalogRole, family: TaskFamily) -> bool {
    family == TaskFamily::Software
        && (!task.write_scopes.is_empty()
            || matches!(
                role,
                CatalogRole::Implementer
                    | CatalogRole::Refactorer
                    | CatalogRole::TestImplementer
                    | CatalogRole::Integrator
            ))
}
pub fn contract_hash(contract: &ChangeContract) -> String {
    hash(serde_json::to_vec(contract).expect("change contract serialization"))
}
pub fn validate_plan(
    task: &TaskRecord,
    require_contract: bool,
    role: CatalogRole,
    family: TaskFamily,
) -> Result<()> {
    let Some(contract) = &task.change_contract else {
        return require(
            !require_contract || !required(task, role, family),
            "software change task requires an explicitly reviewed change classification",
        );
    };
    require(
        family == TaskFamily::Software
            && text(&contract.rationale)
            && !contract.source_refs.is_empty()
            && contract.source_refs.is_subset(&task.source_dependencies),
        "change classification lacks assigned source context or rationale",
    )?;
    let has_bugfix = matches!(contract.kind, ChangeKind::Bugfix | ChangeKind::Mixed);
    require(
        has_bugfix != contract.bugfixes.is_empty(),
        "change classification contradicts its bugfix obligations",
    )?;
    let non_defects = contract
        .non_defect_criteria
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    require(
        non_defects.is_subset(&task.criterion_ids)
            && contract.non_defect_criteria.values().all(|r| text(r))
            && (has_bugfix || non_defects.is_empty()),
        "non-defect classification has foreign criteria, missing rationale or no bugfix",
    )?;
    let mut ids = BTreeSet::new();
    let mut criteria = BTreeSet::new();
    for bugfix in &contract.bugfixes {
        require(
            text(&bugfix.id)
                && ids.insert(&bugfix.id)
                && text(&bugfix.defect)
                && text(&bugfix.oracle)
                && !bugfix.criterion_ids.is_empty()
                && bugfix.criterion_ids.is_subset(&task.criterion_ids),
            "bugfix has duplicate/missing identity, criteria, defect or oracle",
        )?;
        let before = task
            .check_plan
            .iter()
            .find(|p| p.id == bugfix.before.check_id())
            .ok_or_else(|| {
                crate::Error::Invalid("bugfix has no approved original-state check".into())
            })?;
        let after = task
            .check_plan
            .iter()
            .find(|p| p.id == bugfix.after_check_id)
            .ok_or_else(|| {
                crate::Error::Invalid("bugfix has no approved post-change check".into())
            })?;
        require(
            before.phase == CheckPhase::BeforeExecution
                && after.phase == CheckPhase::BeforeVerification
                && after.expected_exit_code == 0
                && bugfix.criterion_ids.is_subset(&before.criterion_ids)
                && bugfix.criterion_ids.is_subset(&after.criterion_ids),
            &format!(
                "task {} bugfix {}: before check {} phase {:?} (required BeforeExecution), after check {} phase {:?} exit {} (required BeforeVerification/0); missing before criteria {:?}; missing after criteria {:?}",
                task.id,
                bugfix.id,
                before.id,
                before.phase,
                after.id,
                after.phase,
                after.expected_exit_code,
                bugfix
                    .criterion_ids
                    .difference(&before.criterion_ids)
                    .collect::<Vec<_>>(),
                bugfix
                    .criterion_ids
                    .difference(&after.criterion_ids)
                    .collect::<Vec<_>>()
            ),
        )?;
        match &bugfix.before {
            BeforeProof::Reproduction { .. } => {}
            BeforeProof::Alternative {
                unavailable_reason,
                method,
                source_refs,
                ..
            } => require(
                text(unavailable_reason)
                    && text(method)
                    && !source_refs.is_empty()
                    && source_refs.is_subset(&task.source_dependencies),
                "unavailable reproduction requires a performed investigation, assigned sources and concrete alternative method",
            )?,
        }
        criteria.extend(bugfix.criterion_ids.iter().cloned());
    }
    require(
        criteria.is_disjoint(&non_defects),
        "a criterion cannot be both a defect and classified as non-defect",
    )?;
    if contract.kind == ChangeKind::Bugfix {
        require(
            criteria
                .union(&non_defects)
                .cloned()
                .collect::<BTreeSet<_>>()
                == task.criterion_ids,
            "bugfix classification leaves assigned criteria without before/after obligations or justified non-defect classification",
        )?;
    }
    Ok(())
}

pub(crate) fn validate_review(
    run: &Run,
    task: &TaskRecord,
    artifact: &Artifact,
    review: Option<&ChangeReview>,
    checks: &[CheckResult],
) -> Result<()> {
    let role = run
        .team
        .as_ref()
        .unwrap()
        .agents
        .iter()
        .find(|a| a.id == task.owner_id)
        .unwrap()
        .role;
    validate_plan(task, true, role, run.spec.task_family)?;
    let Some(contract) = &task.change_contract else {
        return require(
            review.is_none(),
            "change review invents a contract absent from the approved task",
        );
    };
    let review = review.ok_or_else(|| {
        crate::Error::Invalid(
            "task review omitted independent change classification/bugfix proof".into(),
        )
    })?;
    require(
        review.contract_hash == contract_hash(contract)
            && review.kind == contract.kind
            && text(&review.classification_assessment)
            && !review.classification_evidence_ids.is_empty(),
        "change review is unbound or omits independent classification evidence",
    )?;
    for id in &review.classification_evidence_ids {
        require(
            run.evidence.iter().any(|e| {
                e.id == *id
                    && e.output_hash == hash(&e.output)
                    && crate::verification::evidence_is_current(run, artifact, e)
                    && (e.success
                        || crate::runtime::team::checks::baseline_expected(run, artifact, e))
            }),
            "change classification cites missing, stale or corrupted evidence",
        )?;
    }
    require(
        review.classification_evidence_ids.iter().any(|id| {
            run.evidence.iter().any(|e| {
                e.id == *id
                    && e.source == "runtime:input-snapshot"
                    && e.success
                    && crate::verification::evidence_is_current(run, artifact, e)
            })
        }),
        "change classification has no actual current source-snapshot observation",
    )?;
    let classification = checks
        .iter()
        .find(|c| c.check_id == "CHANGE:classification")
        .ok_or_else(|| crate::Error::Invalid("review omits change classification check".into()))?;
    require(
        classification.status == CheckStatus::Pass
            && review
                .classification_evidence_ids
                .iter()
                .all(|id| classification.evidence_ids.contains(id)),
        "classification check omits the independently inspected source evidence",
    )?;
    validate_non_defect_assessments(contract, review)?;
    for (criterion, assessment) in &review.non_defect_assessments {
        require(
            checks.iter().any(|c| {
                c.check_id == format!("CHANGE:non_defect:{criterion}")
                    && c.status == CheckStatus::Pass
                    && assessment
                        .evidence_ids
                        .iter()
                        .all(|id| c.evidence_ids.contains(id))
            }),
            "non-defect classification check lacks its independent decision or evidence",
        )?;
    }
    let mut seen = BTreeSet::new();
    for inspected in &review.bugfixes {
        let bugfix = contract
            .bugfixes
            .iter()
            .find(|b| b.id == inspected.id)
            .ok_or_else(|| {
                crate::Error::Invalid("independent review invents a bugfix obligation".into())
            })?;
        require(
            seen.insert(&inspected.id)
                && text(&inspected.before_assessment)
                && text(&inspected.independent_oracle)
                && text(&inspected.counterexample),
            "bugfix review duplicates an obligation or omits independent oracle/counterexample",
        )?;
        let before = crate::runtime::team::checks::planned_receipt(
            run,
            task,
            artifact,
            bugfix.before.check_id(),
        )?;
        let after = crate::runtime::team::checks::planned_receipt(
            run,
            task,
            artifact,
            &bugfix.after_check_id,
        )?;
        require(
            inspected.before_evidence_ids.contains(&before.id)
                && inspected.after_evidence_ids.contains(&after.id)
                && after.success
                && !after.source.starts_with("runner:baseline:"),
            "bugfix review lacks its exact original/post-change invocation receipts",
        )?;
        let check = checks
            .iter()
            .find(|c| c.check_id == format!("BUGFIX:{}", bugfix.id))
            .ok_or_else(|| crate::Error::Invalid("review omits bugfix proof check".into()))?;
        require(
            check.status == CheckStatus::Pass
                && check.evidence_ids.contains(&before.id)
                && check.evidence_ids.contains(&after.id),
            "bugfix checklist result does not cite both designated invocation receipts",
        )?;
        for (ids, baseline) in [
            (&inspected.before_evidence_ids, true),
            (&inspected.after_evidence_ids, false),
        ] {
            for id in ids {
                require(
                    run.evidence.iter().any(|e| {
                        e.id == *id
                            && e.output_hash == hash(&e.output)
                            && crate::verification::evidence_is_current(run, artifact, e)
                            && if baseline {
                                e.success
                                    || crate::runtime::team::checks::baseline_expected(
                                        run, artifact, e,
                                    )
                            } else {
                                e.success && !e.source.starts_with("runner:baseline:")
                            }
                    }),
                    "bugfix proof cites unobserved, stale or failed post-change evidence",
                )?;
            }
        }
        match &bugfix.before {
            BeforeProof::Reproduction { .. } => require(
                inspected.alternative_assessment.is_none(),
                "reproduced bugfix is relabeled as unavailable",
            )?,
            BeforeProof::Alternative { .. } => require(
                inspected
                    .alternative_assessment
                    .as_ref()
                    .is_some_and(|s| text(s)),
                "independent verifier did not assess unavailable reproduction and the alternative proof",
            )?,
        }
    }
    require(
        seen.len() == contract.bugfixes.len(),
        "independent review omits an assigned bugfix proof",
    )
}

fn validate_non_defect_assessments(contract: &ChangeContract, review: &ChangeReview) -> Result<()> {
    require(
        contract
            .non_defect_criteria
            .keys()
            .eq(review.non_defect_assessments.keys()),
        "independent review omits or invents non-defect classifications",
    )?;
    for assessment in review.non_defect_assessments.values() {
        require(
            text(&assessment.assessment)
                && !assessment.evidence_ids.is_empty()
                && assessment
                    .evidence_ids
                    .is_subset(&review.classification_evidence_ids),
            "non-defect classification lacks an independent assessment with current inspected evidence",
        )?;
    }
    Ok(())
}

/// A fresh format review may fill omissions, but cannot erase contradictory proof claims.
pub(crate) fn validate_retry_claims(
    run: &Run,
    task: &TaskRecord,
    artifact: &Artifact,
    value: &serde_json::Value,
) -> Result<()> {
    let Some(claim) = value
        .pointer("/quality/change_review")
        .filter(|v| !v.is_null())
    else {
        return Ok(());
    };
    let contract = task
        .change_contract
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("prior verifier invented a change contract".into()))?;
    require(
        claim.is_object()
            && claim
                .get("contract_hash")
                .is_none_or(|v| v.as_str() == Some(contract_hash(contract).as_str()))
            && claim
                .get("kind")
                .is_none_or(|v| *v == serde_json::to_value(contract.kind).unwrap()),
        "prior verifier change proof claims another contract or classification",
    )?;
    if let Some(proofs) = claim.get("bugfixes") {
        let proofs = proofs.as_array().ok_or_else(|| {
            crate::Error::Invalid("prior bugfix claims are not identifiable".into())
        })?;
        let mut seen = BTreeSet::new();
        for proof in proofs {
            let bugfix = contract
                .bugfixes
                .iter()
                .find(|b| proof["id"].as_str() == Some(&b.id))
                .ok_or_else(|| {
                    crate::Error::Invalid("prior review invented a defect identity".into())
                })?;
            require(
                seen.insert(&bugfix.id),
                "prior review duplicates a defect identity",
            )?;
            for (field, check_id) in [
                ("before_evidence_ids", bugfix.before.check_id()),
                ("after_evidence_ids", bugfix.after_check_id.as_str()),
            ] {
                if let Some(ids) = proof.get(field) {
                    let ids = ids.as_array().ok_or_else(|| {
                        crate::Error::Invalid(
                            "prior bugfix receipt claims are not identifiable".into(),
                        )
                    })?;
                    let actual = crate::runtime::team::checks::planned_receipt(
                        run, task, artifact, check_id,
                    )?;
                    require(
                        ids.is_empty() || ids.iter().any(|id| id.as_str() == Some(&actual.id)),
                        "prior verifier contradicts the designated original/post-change receipt",
                    )?;
                    for id in ids {
                        require(
                            run.evidence.iter().any(|e| {
                                id.as_str() == Some(&e.id)
                                    && e.output_hash == hash(&e.output)
                                    && crate::verification::evidence_is_current(run, artifact, e)
                                    && if field == "after_evidence_ids" {
                                        e.success && !e.source.starts_with("runner:baseline:")
                                    } else {
                                        e.success
                                            || crate::runtime::team::checks::baseline_expected(
                                                run, artifact, e,
                                            )
                                    }
                            }),
                            "prior bugfix proof cites stale, unobserved or failed post-change evidence",
                        )?;
                    }
                }
            }
        }
    }
    Ok(())
}

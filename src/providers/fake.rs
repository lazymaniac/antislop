use super::*;
use crate::{id, require};
use std::sync::atomic::Ordering;

pub struct FakeProvider;
impl Provider for FakeProvider {
    fn probe(&self) -> Result<Capabilities> {
        Ok(Capabilities {
            provider: "fake".into(),
            version: "1".into(),
            structured_output: true,
            streaming: false,
            resume: false,
            steering: false,
            cancellation: true,
            approvals: false,
            native_subagents_disabled: true,
            sandbox: "no tools".into(),
            usage: "unavailable".into(),
        })
    }
    fn execute(
        &self,
        run: &Run,
        cancelled: Arc<AtomicBool>,
        session: &mut dyn FnMut(&str) -> Result<()>,
    ) -> Result<(String, String)> {
        require(!cancelled.load(Ordering::SeqCst), "cancelled")?;
        let session_id = id();
        session(&session_id)?;
        let content = serde_json::json!({"kind":"fake_fixture", "stage":run.stage, "revision":run.spec_revision,
            "objective":run.spec.objective, "attempt":run.attempt, "simulation":"No real model or tools were run."}).to_string();
        Ok((
            session_id,
            serde_json::to_string(&ExecutionOutput {
                quality: None,
                artifact: content,
                edits: vec![],
                context: Some(ContextModel {
                    facts: vec![ContextFact {
                        claim: run.spec.objective.clone(),
                        source_ref: "objective".into(),
                        source_hash: crate::hash(&run.spec.objective),
                    }],
                    unknowns: vec![],
                }),
                solution: Some(SolutionModel {
                    criteria: vec!["Fulfill all requirements".into()],
                    options: vec![
                        SolutionOption {
                            id: "a".into(),
                            mechanism: "minimal change".into(),
                            requirement_ids: run
                                .spec
                                .requirements
                                .iter()
                                .map(|r| r.id.clone())
                                .collect(),
                            evidence: "fixture only".into(),
                        },
                        SolutionOption {
                            id: "b".into(),
                            mechanism: "replace implementation".into(),
                            requirement_ids: run
                                .spec
                                .requirements
                                .iter()
                                .map(|r| r.id.clone())
                                .collect(),
                            evidence: "fixture only".into(),
                        },
                    ],
                    chosen_option: "a".into(),
                    rationale: "fixture comparison".into(),
                    invariants: vec![Invariant {
                        id: "I1".into(),
                        statement: "preserve scope".into(),
                    }],
                    simulations: ["success", "failure", "counterexample"]
                        .iter()
                        .map(|kind| SimulationCase {
                            kind: (*kind).into(),
                            initial_state: "fixture".into(),
                            events: vec!["evaluate".into()],
                            expected_result: "invariant holds".into(),
                            invariant_ids: std::collections::BTreeSet::from(["I1".into()]),
                        })
                        .collect(),
                }),
            })?,
        ))
    }
    fn verify(
        &self,
        run: &Run,
        artifact: &Artifact,
        evidence: &Evidence,
        cancelled: Arc<AtomicBool>,
        session: &mut dyn FnMut(&str) -> Result<()>,
    ) -> Result<Review> {
        require(!cancelled.load(Ordering::SeqCst), "cancelled")?;
        let verifier_session = id();
        session(&verifier_session)?;
        let reject = run.spec.objective.contains("[reject]");
        Ok(Review {
            quality: None,
            artifact_hash: artifact.hash.clone(),
            spec_revision: run.spec_revision,
            checklist_version: checklist_version(),
            executor_session: artifact.session_id.clone(),
            verifier_session,
            checks: run
                .expected_checks()
                .into_iter()
                .map(|check_id| {
                    let mut evidence_ids = vec![evidence.id.clone()];
                    if check_id.starts_with("REVALIDATE:") {
                        let current_hash = crate::hash(
                            serde_json::to_vec(&run.input_snapshot)
                                .expect("snapshot serialization"),
                        );
                        if let Some(snapshot) = run.evidence.iter().rev().find(|receipt| {
                            receipt.source == "runtime:input-snapshot"
                                && receipt.spec_revision == run.spec_revision
                                && receipt.output_hash == current_hash
                                && receipt.success
                        }) {
                            evidence_ids.push(snapshot.id.clone());
                        }
                    }
                    CheckResult {
                        check_id,
                        status: if reject {
                            CheckStatus::Fail
                        } else {
                            CheckStatus::Pass
                        },
                        method: "deterministic fixture inspection".into(),
                        evidence_ids,
                        rationale: "Synthetic fixture result; not a claim of product quality."
                            .into(),
                    }
                })
                .collect(),
            findings: if reject {
                vec!["Seeded defective result".into()]
            } else {
                vec![]
            },
        })
    }
}

use super::*;
use crate::{id, require};
use std::sync::atomic::Ordering;

pub struct FakeProvider;
impl Provider for FakeProvider {
    fn execute_observed(
        &self,
        invocation: &ObservedInvocation<'_>,
        cancelled: Arc<AtomicBool>,
        events: &mut dyn FnMut(&ProviderEvent) -> Result<()>,
    ) -> Result<TurnReceipt> {
        require(
            invocation.resume.is_none(),
            "synthetic provider has no native resume",
        )?;
        events(&ProviderEvent::DeliveryPendingUnknown {
            turn_id: invocation.turn_id.into(),
        })?;
        let (session_id, output) = self.execute(invocation.run, cancelled, &mut |session| {
            events(&ProviderEvent::SessionObserved {
                session_id: session.into(),
            })
        })?;
        events(&ProviderEvent::DeliverySent {
            turn_id: invocation.turn_id.into(),
        })?;
        events(&ProviderEvent::Completed {
            turn_id: invocation.turn_id.into(),
        })?;
        Ok(TurnReceipt {
            turn_id: invocation.turn_id.into(),
            provider: "fake".into(),
            session_id,
            delivered: true,
            completed: true,
            stopped: true,
            output,
            usage: UsageReport::Unavailable,
        })
    }
    fn verify_observed(
        &self,
        invocation: &ObservedInvocation<'_>,
        artifact: &Artifact,
        evidence: &Evidence,
        cancelled: Arc<AtomicBool>,
        events: &mut dyn FnMut(&ProviderEvent) -> Result<()>,
    ) -> Result<TurnReceipt> {
        require(
            invocation.resume.is_none(),
            "synthetic provider has no native resume",
        )?;
        events(&ProviderEvent::DeliveryPendingUnknown {
            turn_id: invocation.turn_id.into(),
        })?;
        let review = self.verify(
            invocation.run,
            artifact,
            evidence,
            cancelled,
            &mut |session| {
                events(&ProviderEvent::SessionObserved {
                    session_id: session.into(),
                })
            },
        )?;
        events(&ProviderEvent::DeliverySent {
            turn_id: invocation.turn_id.into(),
        })?;
        events(&ProviderEvent::Completed {
            turn_id: invocation.turn_id.into(),
        })?;
        let receipt = TurnReceipt {
            turn_id: invocation.turn_id.into(),
            provider: "fake".into(),
            session_id: review.verifier_session.clone(),
            delivered: true,
            completed: true,
            stopped: true,
            output: serde_json::to_string(&review)?,
            usage: UsageReport::Unavailable,
        };
        Ok(receipt)
    }
    fn turn_receipt(
        &self,
        request: &TurnRequest<'_>,
        cancelled: Arc<AtomicBool>,
        events: &mut dyn FnMut(&ProviderEvent) -> Result<()>,
    ) -> Result<TurnReceipt> {
        require(!cancelled.load(Ordering::SeqCst), "cancelled")?;
        events(&ProviderEvent::DeliveryPendingUnknown {
            turn_id: request.turn_id.into(),
        })?;
        let session_id = id();
        events(&ProviderEvent::SessionObserved {
            session_id: session_id.clone(),
        })?;
        events(&ProviderEvent::DeliverySent {
            turn_id: request.turn_id.into(),
        })?;
        events(&ProviderEvent::Completed {
            turn_id: request.turn_id.into(),
        })?;
        let output = if request
            .schema
            .get("properties")
            .and_then(|p| p.get("diff_hash"))
            .is_some()
        {
            let context: serde_json::Value = serde_json::Deserializer::from_str(
                request
                    .prompt
                    .split("Immutable diff review context: ")
                    .nth(1)
                    .ok_or_else(|| crate::Error::Invalid("fake diff context missing".into()))?,
            )
            .into_iter()
            .next()
            .transpose()?
            .ok_or_else(|| crate::Error::Invalid("fake diff context empty".into()))?;
            let rejected = request.run.spec.objective.contains("[reject]");
            serde_json::to_string(&serde_json::json!({
                "diff_hash":context["diff_hash"], "input_hash":context["input_hash"],
                "spec_revision":context["spec_revision"],
                "status":if rejected {"fail"} else {"pass"},
                "method":"Synthetic diff-review transport fixture",
                "rationale":"Synthetic fixture; no real code quality is established",
                "findings":if rejected {vec!["Synthetic defect"]} else {vec![]},
                "agent_messages":[]
            }))?
        } else if request
            .schema
            .get("properties")
            .and_then(|p| p.get("team_plan_json"))
            .is_some()
        {
            let team =
                crate::team::TeamPlan::bootstrap(request.run.spec.task_family, &request.run.spec)?;
            serde_json::to_string(&serde_json::json!({
                "message": "Synthetic orchestration proposal",
                "team_plan_json": serde_json::to_string(&team)?,
                "risk_owners": request.run.risk_coverage.keys().map(|r| {
                    serde_json::json!({
                        "risk_id": r,
                        "task_id": team.tasks.first().map(|t| t.id.clone()).unwrap_or_default(),
                        "rationale": "Synthetic risk assignment"
                    })
                }).collect::<Vec<_>>(),
                "agent_messages": []
            }))?
        } else if request
            .schema
            .get("properties")
            .and_then(|p| p.get("contract_version"))
            .is_some()
        {
            let checks: Vec<_> = (1..=8)
                .map(|i| {
                    serde_json::json!({
                        "check_id": format!("V{i:02}"),
                        "status": "pass",
                        "method": "Synthetic check fixture",
                        "evidence_ids": vec!["synthetic-evidence"],
                        "rationale": "Synthetic passing review"
                    })
                })
                .collect();
            serde_json::to_string(&serde_json::json!({
                "contract_version": "1.0",
                "output_hash": crate::hash("fake-output"),
                "spec_revision": request.run.spec_revision,
                "input_hash": crate::hash("fake-input"),
                "checks": checks,
                "findings": [],
                "agent_messages": []
            }))?
        } else if request
            .schema
            .get("properties")
            .and_then(|p| p.get("criteria"))
            .is_some()
        {
            let criteria_map: std::collections::BTreeMap<String, bool> =
                if let Some(team) = &request.run.team {
                    team.tasks
                        .iter()
                        .flat_map(|t| t.criterion_ids.iter().cloned().map(|c| (c, true)))
                        .collect()
                } else {
                    std::collections::BTreeMap::new()
                };
            let reject = request.run.spec.objective.contains("[reject]");
            serde_json::to_string(&serde_json::json!({
                "accepted": !reject,
                "criteria": criteria_map,
                "findings": if reject { vec!["synthetic rejection".to_string()] } else { vec![] },
                "checks": [],
                "agent_messages": []
            }))?
        } else {
            serde_json::to_string(&serde_json::json!({
                "artifact": format!("Synthetic task artifact for turn {}", request.turn_id),
                "edits": [],
                "quality": null,
                "agent_messages": []
            }))?
        };
        Ok(TurnReceipt {
            turn_id: request.turn_id.into(),
            provider: "fake".into(),
            session_id,
            delivered: true,
            completed: true,
            stopped: true,
            output,
            usage: UsageReport::Unavailable,
        })
    }
    fn parse_review(
        &self,
        _run: &Run,
        _artifact: &Artifact,
        receipt: &TurnReceipt,
    ) -> Result<Review> {
        crate::transport::decode(receipt.output.as_bytes())
    }
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
                agent_messages: vec![],
                quality: Some(crate::quality::synthetic_artifacts(run)),
                artifact: content,
                edits: vec![],
                context: Some(ContextModel {
                    facts: run
                        .source_hashes()
                        .into_iter()
                        .map(|(source_ref, source_hash)| ContextFact {
                            claim: "Synthetic source-identity fixture; no product truth asserted."
                                .into(),
                            source_ref,
                            source_hash,
                        })
                        .collect(),
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
            quality: Some(crate::quality::synthetic_artifacts(run)),
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

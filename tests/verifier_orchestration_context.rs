//! Independent checks of machine-owned workflow facts supplied to models.
use architect::{
    config::RoleKind,
    domain::{Approval, Artifact, Evidence, Run, RunSpec, Stage},
    orchestration,
    providers::{self, ObservedInvocation},
    sessions::{OutputRef, SessionRecord, SessionScope, TurnRecord},
    verification,
};
use std::sync::{Arc, atomic::AtomicBool};

fn context_section(prompt: &str, name: &str) -> serde_json::Value {
    use serde_json::Value;
    fn raw_document(registry: &Value, id: &str) -> String {
        let document = registry["documents"]
            .get(id)
            .or_else(|| {
                registry["documents"]
                    .as_object()
                    .unwrap()
                    .values()
                    .find(|d| d.get("sha256").unwrap_or(&d[0]) == id)
            })
            .unwrap();
        let document = if let Some(tuple) = document.as_array() {
            assert_eq!(tuple.len(), 3);
            if tuple[2].is_string() {
                serde_json::json!({"sha256":tuple[0],"utf8_bytes":tuple[1],"utf8":tuple[2]})
            } else {
                serde_json::json!({"sha256":tuple[0],"utf8_bytes":tuple[1],"parts":tuple[2]})
            }
        } else {
            document.clone()
        };
        let raw = if let Some(raw) = document["utf8"].as_str() {
            raw.to_owned()
        } else {
            document["parts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|part| {
                    if let Some(text) = part.as_str() {
                        text.to_owned()
                    } else {
                        let index = part.as_i64().unwrap();
                        assert_ne!(index, 0);
                        let raw = raw_document(registry, &format!("d{}", index.unsigned_abs()));
                        if index < 0 {
                            serde_json::to_string(&raw).unwrap()
                        } else {
                            raw
                        }
                    }
                })
                .collect()
        };
        assert_eq!(architect::hash(&raw), document["sha256"]);
        raw
    }
    fn resolve(value: &Value, registry: &Value) -> Value {
        match value {
            Value::Object(object) if object.len() == 1 && object.contains_key("json_value_ref") => {
                serde_json::from_str(&raw_document(
                    registry,
                    object["json_value_ref"].as_str().unwrap(),
                ))
                .unwrap()
            }
            Value::Object(object) if object.len() == 1 && object.contains_key("document_ref") => {
                let id = object["document_ref"].as_str().unwrap();
                Value::String(raw_document(registry, id))
            }
            Value::Object(object) => Value::Object(
                object
                    .iter()
                    .map(|(key, value)| (key.clone(), resolve(value, registry)))
                    .collect(),
            ),
            Value::Array(values) => Value::Array(
                values
                    .iter()
                    .map(|value| resolve(value, registry))
                    .collect(),
            ),
            other => other.clone(),
        }
    }
    let raw = prompt
        .split("BEGIN INLINE CONTEXT REGISTRY\n")
        .nth(1)
        .unwrap()
        .split("\nEND INLINE CONTEXT REGISTRY")
        .next()
        .unwrap();
    let registry: Value = serde_json::from_str(raw).unwrap();
    let original = &registry["sections"][name];
    let resolved = resolve(original, &registry);
    if original.get("document_ref").is_some() {
        serde_json::from_str(resolved.as_str().unwrap()).unwrap()
    } else {
        resolved
    }
}

fn fixture() -> (tempfile::TempDir, Run) {
    let root = tempfile::tempdir().unwrap();
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().to_string_lossy().into();
    let mut run = Run::new(architect::id(), spec).unwrap();
    run.attempt = 1;
    run.approvals.push(Approval {
        action: "start".into(),
        scope_hash: run.scope_hash("start"),
        spec_revision: run.spec_revision,
        actor: "user:actual-approval".into(),
    });
    let snapshot = serde_json::to_string(&run.input_snapshot).unwrap();
    run.evidence.push(Evidence {
        id: architect::id(),
        artifact_hash: architect::hash(&snapshot),
        spec_revision: run.spec_revision,
        input_hash: Some(architect::hash(&snapshot)),
        source: "runtime:input-snapshot".into(),
        output_hash: architect::hash(&snapshot),
        output: snapshot,
        success: true,
    });
    (root, run)
}

fn artifact(run: &Run) -> Artifact {
    let content = format!("artifact at {:?}/{}", run.stage, run.attempt);
    Artifact {
        id: architect::id(),
        stage: run.stage,
        attempt: run.attempt,
        spec_revision: run.spec_revision,
        session_id: "actual-executor".into(),
        hash: architect::hash(&content),
        content,
    }
}

#[test]
fn fixed_stage_context_preserves_actual_assignments_approvals_and_pending_status() {
    let (_root, run) = fixture();
    let context = orchestration::context(&run).unwrap();
    assert_eq!(context["run_id"], run.id);
    assert_eq!(context["attempt"], run.attempt);
    assert_eq!(
        context["approvals"],
        serde_json::to_value(&run.approvals).unwrap()
    );
    let risks = context["risk_coverage"].as_array().unwrap();
    assert_eq!(risks.len(), run.risk_coverage.len());
    assert_eq!(risks.len(), 240);
    let mut actual_ids = std::collections::BTreeSet::new();
    for risk in risks {
        let risk_id = risk["risk_id"].as_str().unwrap();
        assert!(actual_ids.insert(risk_id));
        let actual = &run.risk_coverage[risk_id];
        assert_eq!(risk["stage"], serde_json::to_value(actual.stage).unwrap());
        assert_eq!(risk["executor_role"], actual.executor_role);
        assert_eq!(risk["verifier_role"], actual.verifier_role);
        assert_eq!(risk["check_id"], actual.check_id);
        assert_eq!(risk["status"], "blocked");
    }
    let stages = context["stage_assignments"].as_array().unwrap();
    assert_eq!(stages.len(), 6);
    for (index, assignment) in stages.iter().enumerate() {
        let stage: Stage = serde_json::from_value(assignment["stage"].clone()).unwrap();
        let expected: std::collections::BTreeSet<_> = architect::quality::catalog()
            .into_iter()
            .filter(|check| {
                check.stages.contains(&stage)
                    && check
                        .family
                        .is_none_or(|family| family == run.spec.task_family)
            })
            .map(|check| check.role)
            .collect();
        let actual: std::collections::BTreeSet<String> =
            serde_json::from_value(assignment["competencies"].clone()).unwrap();
        assert_eq!(actual, expected);
        assert_ne!(assignment["executor_role"], assignment["verifier_role"]);
        assert_eq!(
            assignment["depends_on_stage"],
            if index == 0 {
                serde_json::Value::Null
            } else {
                stages[index - 1]["stage"].clone()
            }
        );
    }
    assert_eq!(context["sessions"], serde_json::json!([]));
    assert_eq!(context["turns"], serde_json::json!([]));
    assert!(
        context["limitations"]
            .to_string()
            .contains("not execution of the standalone TeamPlan")
    );
    assert!(
        context["independence_contract"]
            .as_str()
            .unwrap()
            .contains("not yet known")
    );
}

#[test]
fn observed_native_metadata_is_real_and_slim_context_does_not_copy_raw_output() {
    let (_root, mut run) = fixture();
    let provider = providers::build("fake").unwrap();
    let view = run.clone();
    let invocation = ObservedInvocation {
        turn_id: "actual-turn",
        run: &view,
        resume: None,
        context_packet: None,
    };
    let prepared = provider.prepare_execute(&invocation).unwrap();
    let scope = SessionScope::for_run(
        &run,
        "actual-executor".into(),
        None,
        RoleKind::Executor,
        "fake".into(),
        "1".into(),
    )
    .unwrap();
    run.session_ledger
        .reserve(
            SessionRecord::new("actual-executor".into(), scope).unwrap(),
            TurnRecord::new(
                "actual-turn".into(),
                "actual-executor".into(),
                prepared.prompt_hash(),
                prepared.schema_hash().unwrap(),
                None,
            )
            .unwrap(),
            &mut |_, _| Ok(()),
        )
        .unwrap();
    let mut receipt = provider
        .execute_observed(
            &invocation,
            Arc::new(AtomicBool::new(false)),
            &mut |event| {
                run.session_ledger
                    .record_provider_event("actual-turn", event, &mut |_, _| Ok(()))
            },
        )
        .unwrap();
    receipt.output = "PRIVATE RAW OUTPUT MUST NOT BE DUPLICATED IN CONTEXT".into();
    run.session_ledger
        .complete(
            "actual-turn",
            &receipt,
            OutputRef {
                document: "turn:actual-turn".into(),
                hash: architect::hash(&receipt.output),
            },
            &mut |_, _| Ok(()),
        )
        .unwrap();
    let context = orchestration::context(&run).unwrap();
    assert_eq!(
        context["sessions"][0]["native_session_id"],
        receipt.session_id
    );
    assert_eq!(context["sessions"][0]["native_observed"], true);
    assert_eq!(context["sessions"][0]["state"], "completed");
    assert_eq!(
        context["turns"][0]["output_ref"]["hash"],
        architect::hash(&receipt.output)
    );
    assert!(!context.to_string().contains(&receipt.output));
    let execution = provider
        .prepare_execute(&ObservedInvocation {
            turn_id: "future-turn",
            run: &run,
            resume: None,
            context_packet: None,
        })
        .unwrap();
    assert_eq!(
        context_section(&execution.prompt, "Machine-owned orchestration context"),
        context
    );
    let artifact = artifact(&run);
    let evidence = verification::observe(&artifact);
    let verification = provider
        .prepare_verify(
            &ObservedInvocation {
                turn_id: "future-verifier",
                run: &run,
                resume: None,
                context_packet: None,
            },
            &artifact,
            &evidence,
        )
        .unwrap();
    assert_eq!(
        context_section(&verification.prompt, "Machine-owned orchestration context"),
        context
    );
    assert!(
        verification.prompt.contains("future allocations")
            || verification.prompt.contains("future sessions")
            || verification.prompt.contains("pending assessments")
    );
}

#[test]
fn prior_stage_orchestration_receipt_is_not_current_for_a_new_artifact() {
    let (_root, mut run) = fixture();
    let old_artifact = artifact(&run);
    let old = orchestration::observe(&run, &old_artifact.hash).unwrap();
    assert!(verification::evidence_is_current(&run, &old_artifact, &old));
    run.stage = Stage::Planning;
    run.attempt += 1;
    let new_artifact = artifact(&run);
    assert!(
        !verification::evidence_is_current(&run, &new_artifact, &old),
        "Unchanged files do not make old approval/session/stage metadata current"
    );
}

#[test]
fn subordinate_refresh_distinguishes_invocation_stage_from_actual_primary_approval() {
    let (_root, mut primary) = fixture();
    primary.stage = Stage::Execution;
    primary.approvals.clear();
    primary.approvals.push(Approval {
        action: "start".into(),
        scope_hash: primary.scope_hash("start"),
        spec_revision: primary.spec_revision,
        actor: "user:actual-execution-approval".into(),
    });
    let mut scoped = primary.clone();
    scoped.stage = Stage::Discovery;
    let artifact = artifact(&scoped);
    let receipt = orchestration::observe(&primary, &artifact.hash).unwrap();
    let observed: serde_json::Value = serde_json::from_str(&receipt.output).unwrap();
    assert_eq!(observed["invocation_stage"], "execution");
    assert!(verification::evidence_is_current(
        &scoped, &artifact, &receipt
    ));
    scoped.evidence.push(receipt);
    let invocation = orchestration::context(&scoped).unwrap();
    assert_eq!(invocation["invocation_stage"], "discovery");
    assert_eq!(invocation["approvals"], observed["approvals"]);
    assert_ne!(scoped.scope_hash("start"), primary.approvals[0].scope_hash);
    let policy = invocation["policy"]["source_refresh"].as_str().unwrap();
    assert!(
        policy.contains("subordinate")
            && policy.contains("cannot edit files")
            && policy.contains("do not claim separate stage approvals")
    );
    let provider = providers::build("fake").unwrap();
    let prepared = provider
        .prepare_verify(
            &ObservedInvocation {
                turn_id: "refresh-verifier",
                run: &scoped,
                resume: None,
                context_packet: None,
            },
            &artifact,
            &verification::observe(&artifact),
        )
        .unwrap();
    assert!(
        context_section(&prepared.prompt, "Current receipts")
            .as_array()
            .unwrap()
            .iter()
            .any(|receipt| receipt["output"] == serde_json::to_string(&observed).unwrap())
    );
    assert_eq!(
        context_section(&prepared.prompt, "Machine-owned orchestration context"),
        invocation
    );
}

#[test]
fn verifier_can_inspect_executor_preturn_snapshot_without_using_it_as_current_evidence() {
    let (_root, mut run) = fixture();
    let preturn = orchestration::observe(
        &run,
        &architect::hash(serde_json::to_vec(&run.input_snapshot).unwrap()),
    )
    .unwrap();
    let preturn_context: serde_json::Value = serde_json::from_str(&preturn.output).unwrap();
    assert_eq!(preturn_context["sessions"], serde_json::json!([]));
    run.evidence.push(preturn.clone());
    let provider = providers::build("fake").unwrap();
    let executor = provider
        .prepare_execute(&ObservedInvocation {
            turn_id: "executor-turn",
            run: &run,
            resume: None,
            context_packet: None,
        })
        .unwrap();
    assert!(
        context_section(&executor.prompt, "Runner evidence")
            .as_array()
            .unwrap()
            .contains(&serde_json::to_value(&preturn).unwrap())
    );
    let artifact = artifact(&run);
    let postturn = orchestration::observe(&run, &artifact.hash).unwrap();
    run.evidence.push(postturn.clone());
    assert!(!verification::evidence_is_current(
        &run, &artifact, &preturn
    ));
    assert!(verification::evidence_is_current(
        &run, &artifact, &postturn
    ));
    let verifier = provider
        .prepare_verify(
            &ObservedInvocation {
                turn_id: "verifier-turn",
                run: &run,
                resume: None,
                context_packet: None,
            },
            &artifact,
            &verification::observe(&artifact),
        )
        .unwrap();
    let ids = verifier.schema["properties"]["checks"]["items"]["properties"]["evidence_ids"]["items"]["enum"].as_array().unwrap();
    assert!(ids.contains(&serde_json::json!(postturn.id)));
    assert!(!ids.contains(&serde_json::json!(preturn.id)));
    assert!(
        context_section(&verifier.prompt, "Historical orchestration snapshots")
            .as_array()
            .unwrap()
            .contains(&serde_json::to_value(&preturn).unwrap()),
        "The historical snapshot supplied to the executor must remain inspectable; its missing future session metadata is not a false executor statement"
    );
    assert!(verifier.prompt.to_lowercase().contains("historical"));
}

#[test]
fn historical_orchestration_narrative_excludes_different_contracts_or_corrupt_output() {
    for field in [
        "run_id",
        "attempt",
        "spec_revision",
        "checklist_version",
        "corrupt",
    ] {
        let (_root, mut run) = fixture();
        let mut historical =
            orchestration::observe(&run, &architect::hash("preturn snapshot")).unwrap();
        let mut context: serde_json::Value = serde_json::from_str(&historical.output).unwrap();
        match field {
            "run_id" | "checklist_version" => {
                context[field] = serde_json::json!("different-contract")
            }
            "attempt" | "spec_revision" => context[field] = serde_json::json!(999),
            "corrupt" => {}
            _ => unreachable!(),
        }
        historical.output = serde_json::to_string(&context).unwrap();
        historical.output_hash = architect::hash(&historical.output);
        if field == "corrupt" {
            historical.output.push_str("corrupt");
        }
        let excluded_id = historical.id.clone();
        run.evidence.push(historical);
        let artifact = artifact(&run);
        run.evidence
            .push(orchestration::observe(&run, &artifact.hash).unwrap());
        let provider = providers::build("fake").unwrap();
        let prepared = provider
            .prepare_verify(
                &ObservedInvocation {
                    turn_id: "verifier",
                    run: &run,
                    resume: None,
                    context_packet: None,
                },
                &artifact,
                &verification::observe(&artifact),
            )
            .unwrap();
        assert!(
            !prepared.prompt.contains(&excluded_id),
            "Unrelated or corrupt {field} snapshot leaked into historical narrative"
        );
    }
}

#[test]
fn corrupted_orchestration_receipt_cannot_establish_machine_owned_facts() {
    let (_root, run) = fixture();
    let artifact = artifact(&run);
    let mut evidence = orchestration::observe(&run, &artifact.hash).unwrap();
    assert!(verification::evidence_is_current(
        &run, &artifact, &evidence
    ));
    evidence.output.push_str("tampered");
    assert!(!verification::evidence_is_current(
        &run, &artifact, &evidence
    ));
}

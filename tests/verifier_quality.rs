//! Independent seeded counterexamples for evidence freshness.
use architect::{
    domain::*,
    hash, id,
    providers::{Provider, fake::FakeProvider},
    verification,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, atomic::AtomicBool},
};

fn fixture() -> (tempfile::TempDir, Run) {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("source"), "before").unwrap();
    let run = Run::new(
        id(),
        RunSpec {
            team_template: None,
            task_family: architect::quality::TaskFamily::Software,
            config: None,
            objective: "Preserve the source contract".into(),
            workspace: root.path().to_string_lossy().into_owned(),
            provider: "fake".into(),
            model: None,
            requirements: vec![Requirement {
                id: "R1".into(),
                original_text: "Preserve the contract".into(),
                source_ref: "user".into(),
                criteria: BTreeMap::from([("contract".into(), "Contract is preserved".into())]),
            }],
            plan_items: vec![PlanItem {
                id: "P1".into(),
                dependencies: BTreeSet::new(),
                requirement_ids: BTreeSet::from(["R1".into()]),
                description: "Verify the contract".into(),
            }],
            input_files: vec!["source".into()],
            editable_files: vec![],
            check_command: vec![],
        },
    )
    .unwrap();
    (root, run)
}

fn snapshot(run: &mut Run, artifact: &Artifact) {
    let output = serde_json::to_string(&run.input_snapshot).unwrap();
    run.evidence.push(Evidence {
        id: id(),
        artifact_hash: artifact.hash.clone(),
        spec_revision: run.spec_revision,
        input_hash: Some(hash(serde_json::to_vec(&run.input_snapshot).unwrap())),
        source: "runtime:input-snapshot".into(),
        output_hash: hash(&output),
        output,
        success: true,
    });
}

fn artifact(run: &mut Run, stage: Stage) -> (Artifact, Evidence) {
    run.stage = stage;
    run.attempt += 1;
    let (session_id, content) = FakeProvider
        .execute(run, Arc::new(AtomicBool::new(false)), &mut |_| Ok(()))
        .unwrap();
    let mut output: ExecutionOutput = serde_json::from_str(&content).unwrap();
    if stage == Stage::Discovery {
        output.context.as_mut().unwrap().facts.push(ContextFact {
            claim: "The source contains exactly the text before".into(),
            source_ref: "file:source".into(),
            source_hash: run.source_hashes()["file:source"].clone(),
        });
    }
    let content = serde_json::to_string(&output).unwrap();
    let artifact = Artifact {
        id: id(),
        stage,
        attempt: run.attempt,
        spec_revision: run.spec_revision,
        session_id,
        hash: hash(&content),
        content,
    };
    run.artifacts.push(artifact.clone());
    verification::validate_output(run, &output).unwrap();
    verification::register_artifact(run, &artifact, &output).unwrap();
    let evidence = verification::observe(&artifact);
    run.evidence.push(evidence.clone());
    snapshot(run, &artifact);
    (artifact, evidence)
}

fn review(run: &Run, artifact: &Artifact, evidence: &Evidence) -> Review {
    FakeProvider
        .verify(
            run,
            artifact,
            evidence,
            Arc::new(AtomicBool::new(false)),
            &mut |_| Ok(()),
        )
        .unwrap()
}

fn accept(run: &mut Run, artifact: &Artifact, evidence: &Evidence) {
    let review = review(run, artifact, evidence);
    verification::validate_review(run, artifact, &review).unwrap();
    verification::record_review(run, artifact, &review).unwrap();
    run.reviews.push(review);
}

#[test]
fn a_source_snapshot_is_not_a_replacement_for_a_contradicted_fact() {
    let (_root, mut run) = fixture();
    let (discovery, evidence) = artifact(&mut run, Stage::Discovery);
    accept(&mut run, &discovery, &evidence);
    let fact_id = run
        .provenance
        .iter()
        .find(|(_, node)| {
            node.kind == ProvenanceKind::ContextFact
                && serde_json::from_str::<ContextFact>(&node.content)
                    .is_ok_and(|fact| fact.claim == "The source contains exactly the text before")
        })
        .expect("seeded contradicted file fact")
        .0
        .clone();
    let old_fact = run.provenance[&fact_id].content.clone();
    run.input_snapshot.insert("source".into(), "after".into());
    verification::refresh_provenance(&mut run);
    assert!(!verification::provenance_node_current(&run, &fact_id));
    let (execution, evidence) = artifact(&mut run, Stage::Execution);
    let reviewed = review(&run, &execution, &evidence);
    let result = verification::validate_review(&run, &execution, &reviewed);
    if result.is_ok() {
        verification::record_review(&mut run, &execution, &reviewed).unwrap();
        run.reviews.push(reviewed);
        assert!(
            run.provenance[&fact_id].content != old_fact
                || !verification::provenance_node_current(&run, &fact_id),
            "the contradicted claim and embedded old source hash were laundered into current evidence without a replacement"
        );
    }
}

#[test]
fn a_same_artifact_runner_receipt_expires_when_its_source_snapshot_changes() {
    let (_root, mut run) = fixture();
    let (execution, _) = artifact(&mut run, Stage::Execution);
    let output = serde_json::json!({"stdout":"passed","stderr":"","exit_code":0}).to_string();
    let receipt = Evidence {
        id: id(),
        artifact_hash: execution.hash.clone(),
        spec_revision: run.spec_revision,
        input_hash: Some(hash(serde_json::to_vec(&run.input_snapshot).unwrap())),
        source: "runner:[\"check\"]".into(),
        output_hash: hash(&output),
        output,
        success: true,
    };
    run.evidence.push(receipt.clone());
    assert!(verification::evidence_is_current(
        &run, &execution, &receipt
    ));
    run.input_snapshot.insert("source".into(), "after".into());
    verification::refresh_provenance(&mut run);
    snapshot(&mut run, &execution);
    assert!(
        !verification::evidence_is_current(&run, &execution, &receipt),
        "a new snapshot for the same artifact must not renew an earlier runner result"
    );
}

#[test]
fn input_domain_delivery_gate_requires_evidence_for_its_own_mandatory_scenarios() {
    let (_root, mut run) = fixture();
    run.spec.check_command = vec!["contracted-check".into()];
    let (integration, evidence) = artifact(&mut run, Stage::Integration);
    let output = serde_json::json!({"stdout":"fixture-scenario passed","stderr":"","exit_code":0})
        .to_string();
    let runner_id = id();
    run.evidence.push(Evidence {
        id: runner_id.clone(),
        artifact_hash: integration.hash.clone(),
        spec_revision: run.spec_revision,
        input_hash: Some(hash(serde_json::to_vec(&run.input_snapshot).unwrap())),
        source: format!(
            "runner:{}",
            serde_json::to_string(&run.spec.check_command).unwrap()
        ),
        output_hash: hash(&output),
        output,
        success: true,
    });
    let mut reviewed = review(&run, &integration, &evidence);
    for check in &mut reviewed.checks {
        if check.check_id.starts_with("GATE:") {
            check.evidence_ids.push(runner_id.clone());
        }
    }
    let software = reviewed
        .quality
        .as_mut()
        .unwrap()
        .software
        .as_mut()
        .unwrap();
    let case = &mut software.input_domain.inputs[0].categories[0];
    case.applicable = true;
    case.expected_result = "Boundary input is rejected without side effects".into();
    case.scenario_ids = BTreeSet::from(["mandatory-never-executed".into()]);
    assert!(
        software
            .behavior
            .as_ref()
            .unwrap()
            .scenario_evidence
            .iter()
            .all(|scenario| scenario.scenario_id != "mandatory-never-executed")
    );
    assert!(
        verification::validate_review(&run, &integration, &reviewed).is_err(),
        "input_domain_verified:delivery must reject a mandatory domain scenario absent from actual behavior evidence"
    );
}

#[test]
fn planning_quality_report_cannot_choose_a_different_solution_than_its_model() {
    let (_root, mut run) = fixture();
    let (planning, _) = artifact(&mut run, Stage::Planning);
    let mut output: ExecutionOutput = serde_json::from_str(&planning.content).unwrap();
    assert_eq!(output.solution.as_ref().unwrap().chosen_option, "a");
    let planning_report = output.quality.as_mut().unwrap().planning.as_mut().unwrap();
    planning_report.chosen_option = "b".into();
    planning_report.rejected_options = vec![architect::quality::OptionRejection {
        option_id: "a".into(),
        reason: "Reject the option that the model actually selected".into(),
    }];
    assert!(
        verification::validate_output(&run, &output).is_err(),
        "plan_ready cannot use a quality report that contradicts the actual selected solution"
    );
}

#[test]
fn a_previous_refresh_does_not_authorize_rebinding_after_a_second_source_change() {
    let (_root, mut run) = fixture();
    let (discovery, evidence) = artifact(&mut run, Stage::Discovery);
    accept(&mut run, &discovery, &evidence);
    run.input_snapshot.insert("source".into(), "after".into());
    verification::refresh_provenance(&mut run);
    let (execution, evidence) = artifact(&mut run, Stage::Execution);
    let mut scoped = run.clone();
    scoped.stage = Stage::Discovery;
    let (session_id, content) = FakeProvider
        .execute(&scoped, Arc::new(AtomicBool::new(false)), &mut |_| Ok(()))
        .unwrap();
    let output: ExecutionOutput = serde_json::from_str(&content).unwrap();
    let fresh = Artifact {
        id: id(),
        stage: Stage::Discovery,
        attempt: run.attempt,
        spec_revision: run.spec_revision,
        session_id,
        hash: hash(&content),
        content,
    };
    verification::register_refresh(&mut run, fresh.clone(), &output).unwrap();
    let view = verification::refresh_review_scope(&run, &fresh);
    let receipt = view
        .evidence
        .iter()
        .find(|e| e.artifact_hash == fresh.hash && e.source == "runtime:artifact-read")
        .unwrap();
    let reviewed = review(&view, &fresh, receipt);
    verification::accept_refresh_review(&mut run, &fresh, &reviewed).unwrap();
    accept(&mut run, &execution, &evidence);
    let fact_id = run
        .provenance
        .iter()
        .find(|(_, node)| {
            node.refresh
                && node.kind == ProvenanceKind::ContextFact
                && serde_json::from_str::<ContextFact>(&node.content)
                    .is_ok_and(|fact| fact.source_ref == "file:source")
        })
        .unwrap()
        .0
        .clone();
    let old_content = run.provenance[&fact_id].content.clone();
    run.input_snapshot.insert("source".into(), "third".into());
    verification::refresh_provenance(&mut run);
    snapshot(&mut run, &execution);
    assert!(!verification::provenance_node_current(&run, &fact_id));
    let reviewed = review(&run, &execution, &evidence);
    if verification::validate_review(&run, &execution, &reviewed).is_ok() {
        verification::record_review(&mut run, &execution, &reviewed).unwrap();
        run.reviews.push(reviewed);
        assert!(
            run.provenance[&fact_id].content != old_content
                || !verification::provenance_node_current(&run, &fact_id),
            "a once-refreshed typed fact was rebound to a second changed source without a new executor replacement"
        );
    }
}

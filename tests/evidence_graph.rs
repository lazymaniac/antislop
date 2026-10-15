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
            objective: "Preserve the documented input contract".into(),
            workspace: root.path().to_string_lossy().into_owned(),
            provider: "fake".into(),
            model: None,
            requirements: vec![Requirement {
                id: "R1".into(),
                original_text: "Preserve the input contract".into(),
                source_ref: "user".into(),
                criteria: BTreeMap::from([("contract".into(), "Contract remains valid".into())]),
            }],
            plan_items: vec![PlanItem {
                id: "P1".into(),
                dependencies: BTreeSet::new(),
                requirement_ids: BTreeSet::from(["R1".into()]),
                description: "Apply the minimal correction".into(),
            }],
            input_files: vec!["source".into()],
            editable_files: vec![],
            check_command: vec![],
        },
    )
    .unwrap();
    (root, run)
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
            claim: "Source participates in the input contract".into(),
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

fn snapshot(run: &mut Run, artifact: &Artifact) {
    let output = serde_json::to_string(&run.input_snapshot).unwrap();
    run.evidence.push(Evidence {
        input_hash: Some(hash(serde_json::to_vec(&run.input_snapshot).unwrap())),
        id: id(),
        artifact_hash: artifact.hash.clone(),
        spec_revision: run.spec_revision,
        source: "runtime:input-snapshot".into(),
        output_hash: hash(&output),
        output,
        success: true,
    });
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

fn accept(run: &mut Run, artifact: &Artifact, evidence: &Evidence) -> Review {
    let review = review(run, artifact, evidence);
    verification::validate_review(run, artifact, &review).unwrap();
    verification::record_review(run, artifact, &review).unwrap();
    run.reviews.push(review.clone());
    // This is the same validation the runtime performs at explicit acceptance.
    verification::validate_review(run, artifact, &review).unwrap();
    review
}

fn refresh(run: &mut Run, stage: Stage) {
    let mut execution = run.clone();
    execution.stage = stage;
    let (session_id, content) = FakeProvider
        .execute(
            &execution,
            Arc::new(AtomicBool::new(false)),
            &mut |_| Ok(()),
        )
        .unwrap();
    let mut output: ExecutionOutput = serde_json::from_str(&content).unwrap();
    if stage == Stage::Discovery
        && !output
            .context
            .as_ref()
            .unwrap()
            .facts
            .iter()
            .any(|fact| fact.source_ref == "file:source")
    {
        output.context.as_mut().unwrap().facts.push(ContextFact {
            claim: "Synthetic refreshed source contract fixture".into(),
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
    verification::register_refresh(run, artifact.clone(), &output).unwrap();
    let scope = verification::refresh_review_scope(run, &artifact);
    let evidence = scope
        .evidence
        .iter()
        .find(|evidence| {
            evidence.artifact_hash == artifact.hash && evidence.source == "runtime:artifact-read"
        })
        .unwrap();
    let reviewed = review(&scope, &artifact, evidence);
    let mut self_review = reviewed.clone();
    self_review.verifier_session = artifact.session_id.clone();
    assert!(
        verification::accept_refresh_review(&mut run.clone(), &artifact, &self_review).is_err()
    );
    verification::accept_refresh_review(run, &artifact, &reviewed).unwrap();
}

#[test]
fn source_change_invalidates_facts_models_simulations_reviews_and_risks_transitively() {
    let (_root, mut run) = fixture();
    let (discovery, evidence) = artifact(&mut run, Stage::Discovery);
    accept(&mut run, &discovery, &evidence);
    let (planning, evidence) = artifact(&mut run, Stage::Planning);
    accept(&mut run, &planning, &evidence);
    assert!(run.provenance.values().all(|node| node.valid));
    run.input_snapshot.insert("source".into(), "after".into());
    verification::refresh_provenance(&mut run);
    for kind in [
        ProvenanceKind::ContextFact,
        ProvenanceKind::ContextModel,
        ProvenanceKind::SolutionModel,
        ProvenanceKind::Simulation,
        ProvenanceKind::Artifact,
        ProvenanceKind::Review,
        ProvenanceKind::RiskAssessment,
    ] {
        assert!(
            run.provenance
                .values()
                .any(|node| node.kind == kind && !node.valid),
            "{kind:?}"
        );
    }
    // The independent objective fact survives; dependent aggregate conclusions do not.
    assert!(
        run.provenance
            .values()
            .any(|node| node.kind == ProvenanceKind::ContextFact && node.valid)
    );
    assert!(
        run.risk_coverage
            .values()
            .filter(|entry| entry.assessment.is_some())
            .all(|entry| !verification::risk_assessment_is_current(&run, entry))
    );
}

#[test]
fn code_change_needs_complete_current_revalidation_and_can_then_finish_delivery() {
    let (_root, mut run) = fixture();
    for stage in [Stage::Discovery, Stage::Planning] {
        let (artifact, evidence) = artifact(&mut run, stage);
        accept(&mut run, &artifact, &evidence);
    }
    let (execution, evidence) = artifact(&mut run, Stage::Execution);
    run.input_snapshot
        .insert("source".into(), "authorized correction".into());
    verification::refresh_provenance(&mut run);
    snapshot(&mut run, &execution);
    let complete = review(&run, &execution, &evidence);
    let stale_checks = complete
        .checks
        .iter()
        .filter(|check| check.check_id.starts_with("REVALIDATE:"))
        .count();
    assert!(stale_checks > 10);
    let mut missing = complete.clone();
    let omitted = missing
        .checks
        .iter()
        .position(|check| check.check_id.starts_with("REVALIDATE:model:"))
        .unwrap();
    missing.checks.remove(omitted);
    assert!(verification::validate_review(&run, &execution, &missing).is_err());
    let mut old_evidence = complete.clone();
    for check in &mut old_evidence.checks {
        if check.check_id.starts_with("REVALIDATE:") {
            check.evidence_ids = vec![evidence.id.clone()];
        }
    }
    assert!(verification::validate_review(&run, &execution, &old_evidence).is_err());
    let mut refused = complete.clone();
    refused
        .checks
        .iter_mut()
        .find(|check| check.check_id.starts_with("REVALIDATE:simulation:"))
        .unwrap()
        .status = CheckStatus::Fail;
    assert!(verification::validate_review(&run, &execution, &refused).is_err());
    assert!(verification::validate_review(&run, &execution, &complete).is_err());
    refresh(&mut run, Stage::Discovery);
    refresh(&mut run, Stage::Planning);
    accept(&mut run, &execution, &evidence);
    assert!(
        run.provenance
            .values()
            .filter(|node| node.superseded_by.is_none())
            .all(|node| node.valid)
    );
    assert_eq!(run.stage, Stage::Execution);
    assert!(
        run.provenance
            .iter()
            .filter(|(_, node)| node.superseded_by.is_some()
                && matches!(
                    node.kind,
                    ProvenanceKind::ContextFact
                        | ProvenanceKind::SolutionModel
                        | ProvenanceKind::Simulation
                ))
            .all(|(id, _)| !verification::provenance_node_current(&run, id))
    );
    for stage in [Stage::Validation, Stage::Integration, Stage::Delivery] {
        let (artifact, evidence) = artifact(&mut run, stage);
        accept(&mut run, &artifact, &evidence);
    }
    assert!(
        run.risk_coverage
            .values()
            .all(|entry| verification::risk_assessment_is_current(&run, entry))
    );
    assert!(
        run.risk_coverage
            .values()
            .all(|entry| entry.assessment.as_ref().is_some_and(|binding| {
                !binding.evidence_ids.is_empty() && !binding.review_hash.is_empty()
            }))
    );
}

#[test]
fn refresh_rejects_missing_or_stale_fact_sources_and_orphaned_review_bindings() {
    let (_root, mut run) = fixture();
    let (discovery, evidence) = artifact(&mut run, Stage::Discovery);
    accept(&mut run, &discovery, &evidence);
    run.input_snapshot.insert("source".into(), "after".into());
    verification::refresh_provenance(&mut run);
    let (primary, _) = artifact(&mut run, Stage::Execution);
    let mut scope = run.clone();
    scope.stage = Stage::Discovery;
    let (session_id, content) = FakeProvider
        .execute(&scope, Arc::new(AtomicBool::new(false)), &mut |_| Ok(()))
        .unwrap();
    let mut output: ExecutionOutput = serde_json::from_str(&content).unwrap();
    output
        .context
        .as_mut()
        .unwrap()
        .facts
        .retain(|fact| fact.source_ref != "file:source");
    let content = serde_json::to_string(&output).unwrap();
    let missing = Artifact {
        id: id(),
        stage: Stage::Discovery,
        attempt: run.attempt,
        spec_revision: run.spec_revision,
        session_id: session_id.clone(),
        hash: hash(&content),
        content,
    };
    assert!(verification::register_refresh(&mut run.clone(), missing, &output).is_err());
    output.context.as_mut().unwrap().facts.push(ContextFact {
        claim: "Actual refreshed source identity".into(),
        source_ref: "file:source".into(),
        source_hash: hash("before"),
    });
    let content = serde_json::to_string(&output).unwrap();
    let stale = Artifact {
        id: id(),
        stage: Stage::Discovery,
        attempt: run.attempt,
        spec_revision: run.spec_revision,
        session_id,
        hash: hash(&content),
        content,
    };
    assert!(verification::register_refresh(&mut run.clone(), stale, &output).is_err());
    refresh(&mut run, Stage::Discovery);
    let node_id = run
        .provenance
        .iter()
        .find(|(_, node)| node.refresh && node.kind == ProvenanceKind::ContextFact)
        .unwrap()
        .0
        .clone();
    assert!(verification::provenance_node_current(&run, &node_id));
    run.provenance
        .get_mut(&node_id)
        .unwrap()
        .revalidation
        .as_mut()
        .unwrap()
        .review_hash = "orphaned-review".into();
    assert!(!verification::provenance_node_current(&run, &node_id));
    assert_eq!(run.artifacts.last().unwrap().id, primary.id);
}

#[test]
fn stale_artifact_receipt_and_forged_cached_validity_cannot_authorize_review() {
    let (_root, mut run) = fixture();
    let (discovery, old_evidence) = artifact(&mut run, Stage::Discovery);
    accept(&mut run, &discovery, &old_evidence);
    run.input_snapshot
        .insert("source".into(), "different".into());
    verification::refresh_provenance(&mut run);
    let (planning, _) = artifact(&mut run, Stage::Planning);
    for node in run.provenance.values_mut() {
        node.valid = true;
    }
    assert!(!verification::evidence_is_current(
        &run,
        &planning,
        &old_evidence
    ));
    assert!(
        run.expected_checks()
            .iter()
            .any(|id| id.starts_with("REVALIDATE:context:"))
    );
    assert!(
        run.risk_coverage
            .values()
            .filter(|entry| entry.assessment.is_some())
            .all(|entry| !verification::risk_assessment_is_current(&run, entry))
    );
}

#[test]
fn delivery_rejects_corrupted_or_reassigned_assessment_references() {
    let (_root, mut run) = fixture();
    for stage in [
        Stage::Discovery,
        Stage::Planning,
        Stage::Execution,
        Stage::Validation,
        Stage::Integration,
    ] {
        let (artifact, evidence) = artifact(&mut run, stage);
        accept(&mut run, &artifact, &evidence);
    }
    let (delivery, evidence) = artifact(&mut run, Stage::Delivery);
    let review = review(&run, &delivery, &evidence);
    verification::validate_review(&run, &delivery, &review).unwrap();
    for variant in 0..5 {
        let mut broken = run.clone();
        let entry = broken
            .risk_coverage
            .values_mut()
            .find(|entry| entry.stage == Stage::Discovery)
            .unwrap();
        let binding = entry.assessment.as_mut().unwrap();
        match variant {
            0 => binding.review_hash = "unrecorded review".into(),
            1 => binding.evidence_ids = vec!["invented evidence".into()],
            2 => binding
                .source_hashes
                .insert("file:source".into(), "old hash".into())
                .map(|_| ())
                .unwrap_or(()),
            3 => binding.check_id = "RISK:another-risk".into(),
            _ => {
                let other = run
                    .risk_coverage
                    .values()
                    .find(|other| other.stage == Stage::Discovery && other.risk_id != entry.risk_id)
                    .unwrap();
                entry.assessment = other.assessment.clone();
            }
        }
        assert!(verification::validate_review(&broken, &delivery, &review).is_err());
    }
}

#[test]
fn revision_archives_old_nodes_and_requires_new_risk_assessments() {
    let (_root, mut run) = fixture();
    let (discovery, evidence) = artifact(&mut run, Stage::Discovery);
    accept(&mut run, &discovery, &evidence);
    let old_count = run.provenance.len();
    run.spec_revision += 1;
    run.risk_coverage = risk_coverage();
    verification::refresh_provenance(&mut run);
    assert!(run.provenance.len() >= old_count);
    assert!(
        run.provenance
            .values()
            .filter(|node| node.kind != ProvenanceKind::Source)
            .all(|node| !node.valid)
    );
    assert!(
        !run.expected_checks()
            .iter()
            .any(|id| id.starts_with("REVALIDATE:"))
    );
    assert!(
        run.risk_coverage
            .values()
            .all(|entry| entry.assessment.is_none())
    );
}

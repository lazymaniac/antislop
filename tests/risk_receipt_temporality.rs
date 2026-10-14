//! Persisted risk proofs retain historical identity, never current claim credit.
use architect::{
    domain::*,
    hash, id, orchestration,
    providers::{Provider, fake::FakeProvider},
    verification,
};
use serde_json::json;
use std::sync::{Arc, atomic::AtomicBool};
fn artifact(run: &mut Run) -> (Artifact, Evidence) {
    run.attempt += 1;
    let (session_id, content) = FakeProvider
        .execute(run, Arc::new(AtomicBool::new(false)), &mut |_| Ok(()))
        .unwrap();
    let output: ExecutionOutput = serde_json::from_str(&content).unwrap();
    let artifact = Artifact {
        id: id(),
        stage: run.stage,
        attempt: run.attempt,
        spec_revision: run.spec_revision,
        session_id,
        hash: hash(&content),
        content,
    };
    run.artifacts.push(artifact.clone());
    verification::register_artifact(run, &artifact, &output).unwrap();
    let read = verification::observe(&artifact);
    run.evidence.push(read.clone());
    let snapshot = serde_json::to_string(&run.input_snapshot).unwrap();
    run.evidence.push(Evidence {
        id: id(),
        artifact_hash: artifact.hash.clone(),
        spec_revision: run.spec_revision,
        source: "runtime:input-snapshot".into(),
        output_hash: hash(&snapshot),
        input_hash: Some(hash(&snapshot)),
        output: snapshot,
        success: true,
    });
    (artifact, read)
}
fn fixture() -> (
    tempfile::TempDir,
    Run,
    Artifact,
    Evidence,
    Artifact,
    Evidence,
    String,
) {
    let root = tempfile::tempdir().unwrap();
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().to_string_lossy().into();
    let mut run = Run::new(id(), spec).unwrap();
    let (old, old_read) = artifact(&mut run);
    let (fresh, fresh_read) = artifact(&mut run);
    let mut review = FakeProvider
        .verify(
            &run,
            &fresh,
            &fresh_read,
            Arc::new(AtomicBool::new(false)),
            &mut |_| Ok(()),
        )
        .unwrap();
    let check = review
        .checks
        .iter_mut()
        .find(|c| c.check_id.starts_with("RISK:"))
        .unwrap();
    let risk = check.check_id.strip_prefix("RISK:").unwrap().to_owned();
    check.evidence_ids = vec![fresh_read.id.clone(), old_read.id.clone()];
    check.rationale = "The original scope is preserved in the refreshed artifact".into();
    verification::validate_review(&run, &fresh, &review).unwrap();
    verification::record_review(&mut run, &fresh, &review).unwrap();
    run.reviews.push(review);
    run.provenance
        .get_mut(&format!("artifact:{}", old.id))
        .unwrap()
        .superseded_by = Some(format!("artifact:{}", fresh.id));
    verification::refresh_provenance(&mut run);
    (root, run, old, old_read, fresh, fresh_read, risk)
}
#[test]
fn authenticated_current_risk_can_retain_an_exact_historical_identity_comparison() {
    let (_root, mut run, old, old_read, fresh, _fresh_read, risk) = fixture();
    assert!(!verification::provenance_node_current(
        &run,
        &format!("artifact:{}", old.id)
    ));
    assert!(!verification::evidence_is_current(&run, &fresh, &old_read));
    assert!(verification::risk_assessment_is_current(
        &run,
        &run.risk_coverage[&risk]
    ));
    let mut new_review = run.reviews.last().unwrap().clone();
    new_review.verifier_session = id();
    assert!(
        verification::validate_review(&run, &fresh, &new_review).is_err(),
        "a fresh check cannot cite the superseded read as current evidence"
    );
    run.attempt += 1;
    run.stage = Stage::Delivery;
    assert!(verification::risk_assessment_is_current(
        &run,
        &run.risk_coverage[&risk]
    ));
}
#[test]
fn historical_identity_never_repairs_a_missing_or_changed_proof() {
    let (_root, run, old, old_read, _fresh, _fresh_read, risk) = fixture();
    for damage in [
        "source",
        "artifact_missing",
        "artifact_content",
        "artifact_revision",
        "receipt_content",
        "receipt_revision",
        "receipt_failed",
        "review_missing",
        "review_corrupt",
    ] {
        let mut changed = run.clone();
        match damage {
            "source" => {
                changed.spec.objective.push_str(" changed scope");
            }
            "artifact_missing" => changed.artifacts.retain(|a| a.id != old.id),
            "artifact_content" => changed
                .artifacts
                .iter_mut()
                .find(|a| a.id == old.id)
                .unwrap()
                .content
                .push_str("changed"),
            "artifact_revision" => {
                changed
                    .artifacts
                    .iter_mut()
                    .find(|a| a.id == old.id)
                    .unwrap()
                    .spec_revision = 0
            }
            "receipt_content" => changed
                .evidence
                .iter_mut()
                .find(|e| e.id == old_read.id)
                .unwrap()
                .output
                .push_str("changed"),
            "receipt_revision" => {
                changed
                    .evidence
                    .iter_mut()
                    .find(|e| e.id == old_read.id)
                    .unwrap()
                    .spec_revision += 1
            }
            "receipt_failed" => {
                changed
                    .evidence
                    .iter_mut()
                    .find(|e| e.id == old_read.id)
                    .unwrap()
                    .success = false
            }
            "review_missing" => changed.reviews.clear(),
            "review_corrupt" => changed
                .reviews
                .last_mut()
                .unwrap()
                .findings
                .push("unresolved defect".into()),
            _ => unreachable!(),
        }
        assert!(
            !verification::risk_assessment_is_current(&changed, &changed.risk_coverage[&risk]),
            "accepted {damage}"
        );
    }
}
#[test]
fn orchestration_authentication_uses_its_bound_artifact_attempt_without_current_scope_leakage() {
    let (_root, mut run, _old, _old_read, fresh, _fresh_read, _risk) = fixture();
    let receipt = orchestration::observe(&run, &fresh.hash).unwrap();
    run.evidence.push(receipt.clone());
    assert!(verification::evidence_is_current(&run, &fresh, &receipt));
    run.attempt += 1;
    run.stage = Stage::Delivery;
    assert!(
        verification::evidence_is_current(&run, &fresh, &receipt),
        "authenticated prior artifact retains its own temporal scope"
    );
    let mut current = fresh.clone();
    current.attempt = run.attempt;
    current.content = "new artifact".into();
    current.hash = hash(&current.content);
    assert!(!verification::evidence_is_current(&run, &current, &receipt));
    for field in ["attempt", "run_id", "spec_revision", "checklist_version"] {
        let mut wrong = receipt.clone();
        let mut body: serde_json::Value = serde_json::from_str(&wrong.output).unwrap();
        body[field] = match field {
            "attempt" => json!(run.attempt),
            "spec_revision" => json!(run.spec_revision + 1),
            _ => json!("foreign"),
        };
        wrong.output = serde_json::to_string(&body).unwrap();
        wrong.output_hash = hash(&wrong.output);
        assert!(
            !verification::evidence_is_current(&run, &fresh, &wrong),
            "accepted changed {field}"
        );
    }
}

#[test]
#[ignore = "read-only audit of an explicitly supplied saved Delivery snapshot"]
fn saved_delivery_review_when_explicitly_supplied() {
    let path = std::env::var_os("ANTISLOP_DELIVERY_RUN_JSON").expect("explicit saved run required");
    let original = std::fs::read(&path).unwrap();
    let run: Run = architect::transport::decode(&original).unwrap();
    assert_eq!(run.stage, Stage::Delivery);
    let failed: Vec<_> = run
        .risk_coverage
        .values()
        .filter(|entry| {
            entry.stage != Stage::Delivery && !verification::risk_assessment_is_current(&run, entry)
        })
        .map(|entry| entry.risk_id.as_str())
        .collect();
    assert!(failed.is_empty(), "stale prior risks: {failed:?}");
    let review = run.reviews.last().unwrap();
    let expected = run.expected_checks();
    let reported: std::collections::BTreeSet<_> = review
        .checks
        .iter()
        .map(|check| check.check_id.clone())
        .collect();
    assert_eq!(
        reported, expected,
        "saved independent review must still cover the exact full expected set"
    );
    verification::validate_review(&run, run.artifacts.last().unwrap(), review).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), original);
}

#[test]
fn batched_prior_binding_observation_matches_the_exact_public_predicate_without_mutation() {
    let (_root, mut run, _old, _old_read, _fresh, _fresh_read, risk) = fixture();
    run.stage = Stage::Planning;
    run.attempt += 1;
    let before = serde_json::to_vec(&run).unwrap();
    let summary = verification::prior_risk_binding_summary(&run);
    let prior: Vec<_> = run
        .risk_coverage
        .values()
        .filter(|entry| entry.stage == Stage::Discovery)
        .collect();
    let stale: std::collections::BTreeSet<_> = prior
        .iter()
        .filter(|entry| !verification::risk_assessment_is_current(&run, entry))
        .map(|entry| entry.risk_id.clone())
        .collect();
    assert!(stale.is_empty());
    assert_eq!(summary.checked_prior_risk_count, prior.len());
    assert_eq!(summary.current_prior_risk_count, prior.len());
    assert_eq!(summary.stale_or_missing_prior_risk_ids, stale);
    assert!(summary.all_checked_prior_bindings_current);
    assert_eq!(
        summary.observation_scope_hash,
        run.scope_hash("risk-binding-observation")
    );
    assert_eq!(
        summary.input_hash,
        hash(serde_json::to_vec(&run.input_snapshot).unwrap())
    );
    assert_eq!(
        summary.source_hashes_hash,
        hash(serde_json::to_vec(&run.source_hashes()).unwrap())
    );
    assert_eq!(summary.run_id, run.id);
    assert_eq!(summary.spec_revision, run.spec_revision);
    assert_eq!(summary.attempt, run.attempt);
    assert_eq!(summary.invocation_stage, run.stage);
    assert_eq!(serde_json::to_vec(&run).unwrap(), before);
    run.reviews
        .last_mut()
        .unwrap()
        .findings
        .push("actual unresolved defect".into());
    let stale_summary = verification::prior_risk_binding_summary(&run);
    assert!(
        stale_summary
            .stale_or_missing_prior_risk_ids
            .contains(&risk)
    );
    assert!(!stale_summary.all_checked_prior_bindings_current);
    assert_eq!(
        stale_summary.current_prior_risk_count
            + stale_summary.stale_or_missing_prior_risk_ids.len(),
        stale_summary.checked_prior_risk_count
    );
}

#[test]
fn binding_observation_excludes_current_and_future_stages_and_is_receipt_bound() {
    let (_root, run, _old, _old_read, fresh, _fresh_read, _risk) = fixture();
    assert_eq!(run.stage, Stage::Discovery);
    let summary = verification::prior_risk_binding_summary(&run);
    assert_eq!(summary.checked_prior_risk_count, 0);
    assert_eq!(summary.current_prior_risk_count, 0);
    assert!(summary.stale_or_missing_prior_risk_ids.is_empty());
    let context = orchestration::context(&run).unwrap();
    assert_eq!(
        context["prior_risk_binding_status"],
        serde_json::to_value(&summary).unwrap()
    );
    assert!(
        context["prior_risk_binding_status_scope"]
            .as_str()
            .unwrap()
            .contains("not quality acceptance")
    );
    let receipt = orchestration::observe(&run, &fresh.hash).unwrap();
    assert_eq!(receipt.output_hash, hash(&receipt.output));
    assert_eq!(
        receipt.input_hash.as_deref(),
        Some(summary.input_hash.as_str())
    );
    let body: serde_json::Value = serde_json::from_str(&receipt.output).unwrap();
    assert_eq!(
        body["prior_risk_binding_status"],
        context["prior_risk_binding_status"]
    );
}

#[test]
#[ignore = "read-only binding observation from an explicitly supplied saved run"]
fn saved_prior_binding_observation_when_explicitly_supplied() {
    let path =
        std::env::var_os("ANTISLOP_PRIOR_RISK_RUN_JSON").expect("explicit saved run required");
    let before = std::fs::read(&path).unwrap();
    let run: Run = architect::transport::decode(&before).unwrap();
    let summary = verification::prior_risk_binding_summary(&run);
    println!(
        "prior_binding_summary={}",
        serde_json::to_string(&summary).unwrap()
    );
    assert_eq!(summary.invocation_stage, Stage::Delivery);
    assert!(
        summary.all_checked_prior_bindings_current,
        "actual stale IDs: {:?}",
        summary.stale_or_missing_prior_risk_ids
    );
    assert_eq!(
        summary.current_prior_risk_count,
        summary.checked_prior_risk_count
    );
    assert_eq!(
        serde_json::to_vec(&run).unwrap(),
        serde_json::to_vec(&architect::transport::decode::<Run>(&before).unwrap()).unwrap()
    );
    assert_eq!(std::fs::read(path).unwrap(), before);
}

#[test]
fn duplicate_stale_labels_do_not_inflate_current_binding_count() {
    let (_root, mut run, _old, _old_read, _fresh, _fresh_read, risk) = fixture();
    run.stage = Stage::Planning;
    run.risk_coverage.get_mut(&risk).unwrap().assessment = None;
    let duplicate = run.risk_coverage[&risk].clone();
    run.risk_coverage
        .insert("malformed-duplicate-key".into(), duplicate);
    let summary = verification::prior_risk_binding_summary(&run);
    let prior: Vec<_> = run
        .risk_coverage
        .values()
        .filter(|entry| entry.stage == Stage::Discovery)
        .collect();
    let current = prior
        .iter()
        .filter(|entry| verification::risk_assessment_is_current(&run, entry))
        .count();
    assert_eq!(summary.checked_prior_risk_count, prior.len());
    assert_eq!(summary.current_prior_risk_count, current);
    assert_eq!(summary.checked_prior_risk_count - current, 2);
    assert_eq!(summary.stale_or_missing_prior_risk_ids.len(), 1);
    assert!(!summary.all_checked_prior_bindings_current);
}

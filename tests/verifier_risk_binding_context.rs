//! Independent observations of real binding predicates, never acceptance grants.
use architect::{
    domain::*,
    hash, id, orchestration,
    providers::{ObservedInvocation, Provider, fake::FakeProvider},
    verification,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    sync::{Arc, atomic::AtomicBool},
};

fn fixture() -> (tempfile::TempDir, Run, String) {
    let root = tempfile::tempdir().unwrap();
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().to_string_lossy().into();
    let mut run = Run::new(id(), spec).unwrap();
    run.attempt = 1;
    let (author, content) = FakeProvider
        .execute(&run, Arc::new(AtomicBool::new(false)), &mut |_| Ok(()))
        .unwrap();
    let output: ExecutionOutput = serde_json::from_str(&content).unwrap();
    let artifact = Artifact {
        id: id(),
        stage: run.stage,
        attempt: run.attempt,
        spec_revision: run.spec_revision,
        session_id: author,
        hash: hash(&content),
        content,
    };
    run.artifacts.push(artifact.clone());
    verification::register_artifact(&mut run, &artifact, &output).unwrap();
    let read = verification::observe(&artifact);
    run.evidence.push(read.clone());
    let review = FakeProvider
        .verify(
            &run,
            &artifact,
            &read,
            Arc::new(AtomicBool::new(false)),
            &mut |_| Ok(()),
        )
        .unwrap();
    verification::validate_review(&run, &artifact, &review).unwrap();
    verification::record_review(&mut run, &artifact, &review).unwrap();
    run.reviews.push(review);
    let risk = run
        .risk_coverage
        .values()
        .find(|r| r.stage == Stage::Discovery)
        .unwrap()
        .risk_id
        .clone();
    run.stage = Stage::Planning;
    run.attempt = 2;
    run.diagnostic = Some("Historical rejection sentinel: must remain unchanged 👩‍💻".into());
    (root, run, risk)
}

fn decoded_document(registry: &Value, id: &str, visiting: &mut BTreeSet<String>) -> String {
    assert!(visiting.insert(id.into()));
    let tuple = registry["documents"][id].as_array().unwrap();
    assert_eq!(tuple.len(), 3);
    let bytes = tuple[1].as_u64().unwrap();
    let raw = if let Some(raw) = tuple[2].as_str() {
        raw.to_owned()
    } else {
        let mut raw = String::new();
        for part in tuple[2].as_array().unwrap() {
            if let Some(text) = part.as_str() {
                raw.push_str(text);
            } else {
                let index = part.as_i64().unwrap();
                assert_ne!(index, 0);
                let child = format!("d{}", index.unsigned_abs());
                assert!(registry["documents"][&child][1].as_u64().unwrap() < bytes);
                let text = decoded_document(registry, &child, visiting);
                raw.push_str(&if index < 0 {
                    serde_json::to_string(&text).unwrap()
                } else {
                    text
                });
            }
        }
        raw
    };
    assert_eq!(hash(&raw), tuple[0]);
    assert_eq!(raw.len() as u64, bytes);
    visiting.remove(id);
    raw
}
fn section(prompt: &str, label: &str) -> Value {
    let raw = prompt
        .split_once("\nBEGIN INLINE CONTEXT REGISTRY\n")
        .unwrap()
        .1
        .split_once("\nEND INLINE CONTEXT REGISTRY")
        .unwrap()
        .0;
    let registry: Value = serde_json::from_str(raw).unwrap();
    assert_eq!(registry["format"], "inline-exact-utf8-v7");
    let reference = &registry["sections"][label];
    if let Some(id) = reference["json_value_ref"].as_str() {
        serde_json::from_str(&decoded_document(&registry, id, &mut BTreeSet::new())).unwrap()
    } else {
        json!(decoded_document(
            &registry,
            reference["document_ref"].as_str().unwrap(),
            &mut BTreeSet::new()
        ))
    }
}

#[test]
fn summary_recomputes_backend_failures_without_mutating_history_or_trusting_pass_labels() {
    let (_root, run, risk) = fixture();
    let before = serde_json::to_vec(&run).unwrap();
    let summary = verification::prior_risk_binding_summary(&run);
    assert!(summary.all_checked_prior_bindings_current);
    assert!(summary.checked_prior_risk_count > 0);
    assert_eq!(
        summary.checked_prior_risk_count,
        summary.current_prior_risk_count
    );
    for entry in run
        .risk_coverage
        .values()
        .filter(|e| e.stage == Stage::Discovery)
    {
        assert!(verification::risk_assessment_is_current(&run, entry));
    }
    for damage in [
        "missing-assessment",
        "missing-review",
        "corrupt-read",
        "source",
        "node",
    ] {
        let mut bad = run.clone();
        let binding = bad.risk_coverage[&risk].assessment.clone().unwrap();
        match damage {
            "missing-assessment" => bad.risk_coverage.get_mut(&risk).unwrap().assessment = None,
            "missing-review" => {
                bad.reviews
                    .retain(|r| hash(serde_json::to_vec(r).unwrap()) != binding.review_hash);
                bad.provenance_reviews.remove(&binding.review_hash);
            }
            "corrupt-read" => bad
                .evidence
                .iter_mut()
                .find(|e| e.id == binding.evidence_ids[0])
                .unwrap()
                .output
                .push('X'),
            "source" => bad.spec.objective.push('X'),
            "node" => bad
                .provenance
                .get_mut(&binding.node_id)
                .unwrap()
                .content
                .push('X'),
            _ => unreachable!(),
        }
        // Keep displayed Pass/cache flags intact: authentication must decide.
        assert_eq!(bad.risk_coverage[&risk].status, CheckStatus::Pass);
        let observed = verification::prior_risk_binding_summary(&bad);
        let expected: BTreeSet<_> = bad
            .risk_coverage
            .values()
            .filter(|e| {
                e.stage == Stage::Discovery && !verification::risk_assessment_is_current(&bad, e)
            })
            .map(|e| e.risk_id.clone())
            .collect();
        assert_eq!(
            observed.stale_or_missing_prior_risk_ids, expected,
            "{damage}"
        );
        assert!(
            observed.stale_or_missing_prior_risk_ids.contains(&risk),
            "{damage}"
        );
        assert!(!observed.all_checked_prior_bindings_current, "{damage}");
        assert_eq!(
            observed.current_prior_risk_count + expected.len(),
            observed.checked_prior_risk_count
        );
    }
    assert_eq!(serde_json::to_vec(&run).unwrap(), before);
}

#[test]
fn historical_diagnostic_and_binding_only_receipt_are_exact_and_never_accept_the_run() {
    let (_root, run, _risk) = fixture();
    let before = serde_json::to_vec(&run).unwrap();
    let prepared = FakeProvider
        .prepare_execute(&ObservedInvocation {
            turn_id: "independent-binding-proof",
            run: &run,
            resume: None,
            context_packet: None,
        })
        .unwrap();
    assert_eq!(
        section(&prepared.prompt, "Historical runtime diagnostic"),
        json!(run.diagnostic)
    );
    let encoded = section(&prepared.prompt, "Machine-owned orchestration context");
    let context: Value = serde_json::from_str(encoded.as_str().unwrap()).unwrap();
    assert_eq!(
        context["prior_risk_binding_status"],
        serde_json::to_value(verification::prior_risk_binding_summary(&run)).unwrap()
    );
    assert!(
        context["prior_risk_binding_status_scope"]
            .as_str()
            .unwrap()
            .contains("not quality acceptance")
    );
    let receipt = orchestration::observe(&run, &run.artifacts.last().unwrap().hash).unwrap();
    assert_eq!(receipt.output_hash, hash(&receipt.output));
    assert_eq!(
        serde_json::from_str::<Value>(&receipt.output).unwrap(),
        context
    );
    assert_eq!(run.status, Status::AwaitingStart);
    assert_eq!(serde_json::to_vec(&run).unwrap(), before);
}

#[test]
#[ignore = "read-only audit of an explicitly supplied saved Delivery11 run"]
fn saved_delivery_eleven_has_current_prior_bindings_but_its_actual_rejection_remains() {
    let path = std::env::var("ANTISLOP_PRIOR_BINDING_RUN_JSON").unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let run: Run = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        (run.stage, run.attempt, &run.status),
        (Stage::Delivery, 11, &Status::ChangesRequested)
    );
    let before = serde_json::to_vec(&run).unwrap();
    let summary = verification::prior_risk_binding_summary(&run);
    assert!(summary.all_checked_prior_bindings_current);
    assert!(summary.stale_or_missing_prior_risk_ids.is_empty());
    assert_eq!(
        summary.checked_prior_risk_count,
        run.risk_coverage
            .values()
            .filter(|e| e.stage != Stage::Delivery)
            .count()
    );
    let artifact = run.artifacts.last().unwrap();
    let review = run.reviews.last().unwrap();
    assert!(!review.findings.is_empty());
    assert!(verification::validate_review(&run, artifact, review).is_err());
    let context = orchestration::context(&run).unwrap();
    assert_eq!(
        context["prior_risk_binding_status"],
        serde_json::to_value(&summary).unwrap()
    );
    assert_eq!(serde_json::to_vec(&run).unwrap(), before);
    assert_eq!(std::fs::read(path).unwrap(), bytes);
    eprintln!(
        "Saved Delivery11: {}/{} prior bindings current; original finding, diagnostic, native raw outputs and rejection unchanged",
        summary.current_prior_risk_count, summary.checked_prior_risk_count
    );
}

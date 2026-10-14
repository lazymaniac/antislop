use architect::{
    domain::*,
    hash, id,
    quality::{self, *},
    verification,
};
use std::collections::{BTreeMap, BTreeSet};

fn fixture(family: TaskFamily) -> (tempfile::TempDir, Run) {
    let root = tempfile::tempdir().unwrap();
    let run = Run::new(
        id(),
        RunSpec {
            task_family: family,
            config: None,
            objective: "Verify the agreed contract".into(),
            workspace: root.path().to_string_lossy().into(),
            provider: "fake".into(),
            model: None,
            requirements: vec![Requirement {
                id: "R1".into(),
                original_text: "Preserve the agreed behavior".into(),
                source_ref: "user".into(),
                criteria: BTreeMap::from([(
                    "behavior".into(),
                    "Contract remains satisfied".into(),
                )]),
            }],
            plan_items: vec![PlanItem {
                id: "P1".into(),
                dependencies: BTreeSet::new(),
                requirement_ids: BTreeSet::from(["R1".into()]),
                description: "Verify model and actual result".into(),
            }],
            input_files: vec![],
            editable_files: vec![],
            check_command: vec![],
        },
    )
    .unwrap();
    (root, run)
}

fn artifact(run: &mut Run, stage: Stage) -> Artifact {
    run.stage = stage;
    run.attempt += 1;
    use architect::providers::{Provider, fake::FakeProvider};
    let (session_id, content) = FakeProvider
        .execute(
            run,
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            &mut |_| Ok(()),
        )
        .unwrap();
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
    run.evidence.push(verification::observe(&artifact));
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
    artifact
}

fn review(run: &Run, artifact: &Artifact) -> Review {
    let evidence = run
        .evidence
        .iter()
        .find(|evidence| {
            evidence.artifact_hash == artifact.hash && evidence.source == "runtime:artifact-read"
        })
        .unwrap();
    Review {
        artifact_hash: artifact.hash.clone(),
        spec_revision: run.spec_revision,
        checklist_version: checklist_version(),
        executor_session: artifact.session_id.clone(),
        verifier_session: id(),
        checks: quality::expected_checks(run)
            .into_iter()
            .map(|check_id| CheckResult {
                check_id,
                status: CheckStatus::Pass,
                method: "Synthetic independent fixture inspection".into(),
                evidence_ids: vec![evidence.id.clone()],
                rationale: "Synthetic fixture only; not product quality evidence".into(),
            })
            .collect(),
        findings: vec![],
        quality: Some(quality::synthetic_artifacts(run)),
    }
}

fn runner(run: &mut Run, artifact: &Artifact, stderr: &str) -> String {
    let output = serde_json::json!({"stdout":"analysis completed","stderr":stderr,"exit_code":0})
        .to_string();
    let evidence = Evidence {
        input_hash: Some(hash(serde_json::to_vec(&run.input_snapshot).unwrap())),
        id: id(),
        artifact_hash: artifact.hash.clone(),
        spec_revision: run.spec_revision,
        source: format!(
            "runner:{}",
            serde_json::to_string(&run.spec.check_command).unwrap()
        ),
        output_hash: hash(&output),
        output,
        success: true,
    };
    let id = evidence.id.clone();
    run.evidence.push(evidence);
    id
}

#[test]
fn role_catalog_has_atomic_smells_and_family_specific_checks() {
    let catalog = quality::catalog();
    assert_eq!(
        catalog
            .iter()
            .filter(|definition| definition.check_id.starts_with("SM"))
            .count(),
        23
    );
    let ids: BTreeSet<_> = catalog
        .iter()
        .map(|definition| &definition.check_id)
        .collect();
    assert_eq!(ids.len(), catalog.len());
    assert!(
        catalog
            .iter()
            .all(|definition| !definition.risk_ids.is_empty()
                && !definition.question.is_empty()
                && !definition.method.is_empty()
                && !definition.rejection_condition.is_empty()
                && !definition.not_applicable_condition.is_empty())
    );
    let (_root, mut run) = fixture(TaskFamily::Software);
    run.stage = Stage::Execution;
    assert!(quality::expected_checks(&run).contains("SM23"));
    run.spec.task_family = TaskFamily::Research;
    let checks = quality::expected_checks(&run);
    assert!(!checks.contains("SM23"));
    assert!(checks.contains("ROLE:research:claim_support"));
    assert!(!checks.iter().any(|id| id.starts_with("ROLE:code:")));
}

#[test]
fn generated_provider_schema_is_strict_and_self_contained() {
    let (_root, run) = fixture(TaskFamily::Software);
    fn inspect(value: &serde_json::Value) {
        if let Some(object) = value.as_object() {
            assert!(!object.contains_key("$ref"));
            assert!(!object.contains_key("$defs"));
            if let Some(properties) = object.get("properties").and_then(|value| value.as_object()) {
                assert_eq!(object["additionalProperties"], false);
                let required: BTreeSet<_> = object["required"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|value| value.as_str().unwrap())
                    .collect();
                assert_eq!(
                    required,
                    properties.keys().map(|key| key.as_str()).collect()
                );
            }
            for value in object.values() {
                inspect(value);
            }
        } else if let Some(values) = value.as_array() {
            for value in values {
                inspect(value);
            }
        }
    }
    inspect(&quality::schema(&run));
}

#[test]
fn input_matrix_rejects_missing_classes_and_pairwise_for_three_way_rules() {
    let (_root, run) = fixture(TaskFamily::Software);
    let quality = quality::synthetic_artifacts(&run);
    let mut matrix = quality.software.unwrap().input_domain;
    quality::validate_input_domain(&run, &matrix).unwrap();
    matrix.inputs[0].categories.pop();
    assert!(quality::validate_input_domain(&run, &matrix).is_err());
    let quality = quality::synthetic_artifacts(&run);
    matrix = quality.software.unwrap().input_domain;
    let original = matrix.inputs[0].clone();
    for name in ["option-b", "state-c"] {
        let mut input = original.clone();
        input.id = name.into();
        matrix.inventory.insert(name.into());
        matrix.inputs.push(input);
    }
    matrix.interactions.push(InputInteraction {
        input_ids: matrix.inventory.clone(),
        rule: "Permission requires option B and persisted state C".into(),
        required_order: 3,
        tested_order: 2,
        expected_result: "Only the authorized triple succeeds".into(),
        scenario_ids: BTreeSet::from(["three-way".into()]),
    });
    assert!(quality::validate_input_domain(&run, &matrix).is_err());
    matrix.interactions[0].tested_order = 3;
    quality::validate_input_domain(&run, &matrix).unwrap();
}

#[test]
fn static_gate_rejects_hidden_diagnostics_suppression_and_scope_weakening() {
    let (_root, mut run) = fixture(TaskFamily::Software);
    run.spec.check_command = vec!["contracted-analyzer".into()];
    let artifact = artifact(&mut run, Stage::Validation);
    let evidence_id = runner(
        &mut run,
        &artifact,
        "warning: unresolved old baseline finding",
    );
    let quality = quality::synthetic_artifacts(&run);
    let software = quality.software.unwrap();
    let mut report = software.static_analysis.unwrap();
    assert!(
        quality::validate_static_analysis(&run, &artifact, &software.contract, &report).is_err()
    );
    let evidence = run
        .evidence
        .iter_mut()
        .find(|evidence| evidence.id == evidence_id)
        .unwrap();
    evidence.output =
        serde_json::json!({"stdout":"completed","stderr":"","exit_code":0}).to_string();
    evidence.output_hash = hash(&evidence.output);
    quality::validate_static_analysis(&run, &artifact, &software.contract, &report).unwrap();
    report.analyses[0].configurations.clear();
    assert!(
        quality::validate_static_analysis(&run, &artifact, &software.contract, &report).is_err()
    );
    report.analyses[0].configurations = software.contract.analyzers[0].configurations.clone();
    report.suppressions.push(SuppressionReview {
        rule: "warning-rule".into(),
        original_finding: "existing warning".into(),
        justification: "make the pipeline green".into(),
        evidence_ids: vec![evidence_id.clone()],
        independently_confirmed_false_positive: false,
    });
    assert!(
        quality::validate_static_analysis(&run, &artifact, &software.contract, &report).is_err()
    );
    report.suppressions.clear();
    let evidence = run
        .evidence
        .iter_mut()
        .find(|evidence| evidence.id == evidence_id)
        .unwrap();
    evidence.source = "runner:baseline:contracted-analyzer".into();
    assert!(
        quality::validate_static_analysis(&run, &artifact, &software.contract, &report).is_err()
    );
}

#[test]
fn dependency_gate_rejects_unknown_licenses_missing_transitives_and_obligations() {
    let (_root, run) = fixture(TaskFamily::Software);
    let mut dependencies = quality::synthetic_artifacts(&run)
        .software
        .unwrap()
        .dependencies;
    dependencies.inventory.insert("transitive-lib".into());
    assert!(quality::validate_dependencies(&dependencies).is_err());
    dependencies.licenses.push(DependencyLicense {
        package: "transitive-lib".into(),
        version: "1.0".into(),
        features: BTreeSet::new(),
        provenance: "pinned registry artifact".into(),
        license_expression: "UNKNOWN".into(),
        interpretation: "not resolved".into(),
        compatible: true,
        obligations: vec![LicenseObligation {
            obligation: "retain notices".into(),
            fulfillment: "bundled notice".into(),
        }],
        unresolved: vec![],
    });
    assert!(quality::validate_dependencies(&dependencies).is_err());
    dependencies.licenses[0].license_expression = "MIT".into();
    dependencies.licenses[0].interpretation = "retain copyright and license in distribution".into();
    quality::validate_dependencies(&dependencies).unwrap();
    dependencies.licenses[0].obligations[0].fulfillment.clear();
    assert!(quality::validate_dependencies(&dependencies).is_err());
}

#[test]
fn every_family_has_complete_gates_and_source_change_reopens_prior_design_gates() {
    for family in [
        TaskFamily::Software,
        TaskFamily::Research,
        TaskFamily::Document,
        TaskFamily::Analysis,
    ] {
        let (_root, mut run) = fixture(family);
        for stage in [
            Stage::Discovery,
            Stage::Planning,
            Stage::Execution,
            Stage::Validation,
            Stage::Integration,
            Stage::Delivery,
        ] {
            let artifact = artifact(&mut run, stage);
            let review = review(&run, &artifact);
            quality::validate_review(&run, &artifact, &review).unwrap();
            quality::record_review(&mut run, &artifact, &review).unwrap();
            run.reviews.push(review.clone());
            quality::validate_review(&run, &artifact, &review).unwrap();
        }
        assert!(run.quality_gates.contains_key("context_ready:design"));
        assert!(
            run.quality_gates
                .contains_key("requirements_complete:delivery")
        );
        if family == TaskFamily::Software {
            assert!(
                run.quality_gates
                    .contains_key("architecture_verified:design")
            );
            assert!(
                run.quality_gates
                    .contains_key("architecture_verified:delivery")
            );
        }
        run.input_snapshot
            .insert("changed-source".into(), "new source".into());
        assert!(quality::expected_checks(&run).contains("GATE:context_ready:design"));
        let artifact = artifact(&mut run, Stage::Delivery);
        let mut review = review(&run, &artifact);
        review.quality.as_mut().unwrap().context = None;
        assert!(quality::validate_review(&run, &artifact, &review).is_err());
    }
}

#[test]
fn actual_execution_gates_cannot_use_an_artifact_read_as_runner_evidence() {
    let (_root, mut run) = fixture(TaskFamily::Software);
    run.spec.check_command = vec!["check-command".into()];
    let artifact = artifact(&mut run, Stage::Execution);
    let mut review = review(&run, &artifact);
    assert!(quality::validate_review(&run, &artifact, &review).is_err());
    let runner_id = runner(&mut run, &artifact, "");
    review.quality = Some(quality::synthetic_artifacts(&run));
    for check in &mut review.checks {
        if check.check_id.starts_with("GATE:") {
            check.evidence_ids.push(runner_id.clone());
        }
    }
    quality::validate_review(&run, &artifact, &review).unwrap();
    review
        .quality
        .as_mut()
        .unwrap()
        .software
        .as_mut()
        .unwrap()
        .refactor
        .smell_assessments
        .pop();
    assert!(quality::validate_review(&run, &artifact, &review).is_err());
}

#[test]
fn fixed_stage_review_cannot_assert_an_unapproved_native_task_change_contract() {
    let (_root, mut run) = fixture(TaskFamily::Software);
    let artifact = artifact(&mut run, Stage::Discovery);
    let mut review = review(&run, &artifact);
    quality::validate_review(&run, &artifact, &review).unwrap();
    review.quality.as_mut().unwrap().change_review = Some(architect::team::changes::ChangeReview {
        contract_hash: "unapproved-contract".into(),
        kind: architect::team::changes::ChangeKind::Feature,
        classification_assessment: "Unsupported native task proof in a fixed stage".into(),
        classification_evidence_ids: BTreeSet::from([run.evidence[0].id.clone()]),
        bugfixes: vec![],
    });
    let error = quality::validate_review(&run, &artifact, &review).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("no approved native task change contract")
    );
}

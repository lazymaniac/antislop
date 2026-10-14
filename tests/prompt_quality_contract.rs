use architect::{
    domain::*,
    providers::{self, ObservedInvocation},
    quality::{self, GateAssessment, GatePhase, TaskFamily},
    verification,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
fn fixture(stage: Stage, family: TaskFamily) -> (tempfile::TempDir, Run, Artifact, Evidence) {
    let root = tempfile::tempdir().unwrap();
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().to_string_lossy().into();
    spec.task_family = family;
    spec.provider = "codex".into();
    spec.check_command = vec!["real-check".into()];
    let mut run = Run::new(architect::id(), spec).unwrap();
    run.stage = stage;
    let content = "current source-bound artifact".to_owned();
    let a = Artifact {
        id: architect::id(),
        stage,
        attempt: run.attempt,
        spec_revision: run.spec_revision,
        session_id: "executor".into(),
        hash: architect::hash(&content),
        content,
    };
    run.artifacts.push(a.clone());
    let e = verification::observe(&a);
    run.evidence.push(e.clone());
    (root, run, a, e)
}
fn verify_schema(run: &Run, a: &Artifact, e: &Evidence) -> providers::PreparedTurn {
    providers::build("fake")
        .unwrap()
        .prepare_verify(
            &ObservedInvocation {
                turn_id: "schema-test",
                run,
                resume: None,
                context_packet: None,
            },
            a,
            e,
        )
        .unwrap()
}
fn nullable(schema: &Value) -> bool {
    schema["type"] == "null"
        || schema["type"]
            .as_array()
            .is_some_and(|types| types.contains(&json!("null")))
        || schema["anyOf"]
            .as_array()
            .is_some_and(|branches| branches.iter().any(nullable))
}
fn stale_gate(run: &mut Run, id: &str) {
    run.quality_gates.insert(
        id.into(),
        GateAssessment {
            gate_id: id.into(),
            phase: GatePhase::Design,
            status: CheckStatus::Pass,
            spec_revision: run.spec_revision,
            source_hashes: BTreeMap::new(),
            artifact_hash: "old".into(),
            review_hash: "old-review".into(),
            check_id: format!("GATE:{id}:design"),
            evidence_ids: vec![],
            report_hash: "old-report".into(),
        },
    );
}
#[test]
fn execution_stale_context_and_plan_gates_require_actual_typed_dtos() {
    let (_root, mut run, a, e) = fixture(Stage::Execution, TaskFamily::Software);
    stale_gate(&mut run, "context_ready");
    stale_gate(&mut run, "plan_ready");
    let request = verify_schema(&run, &a, &e);
    let q = &request.schema["properties"]["quality"]["properties"];
    for field in ["context", "planning", "software"] {
        assert!(
            !nullable(&q[field]),
            "{field} cannot be null for actual expected gates"
        );
        assert_eq!(q[field]["type"], "object");
    }
    assert!(nullable(&q["delivery"]));
    assert!(
        request
            .prompt
            .contains("including prior-stage gates that became stale")
    );
    assert!(
        request
            .prompt
            .contains("all-null quality object cannot pass")
    );
    let mut all_null = quality::synthetic_artifacts(&run);
    all_null.synthetic = false;
    all_null.context = None;
    all_null.planning = None;
    all_null.software = None;
    all_null.research = None;
    all_null.analysis = None;
    all_null.editorial = None;
    all_null.delivery = None;
    let review = Review {
        quality: Some(all_null),
        artifact_hash: a.hash.clone(),
        spec_revision: run.spec_revision,
        checklist_version: run.checklist_version.clone(),
        executor_session: a.session_id.clone(),
        verifier_session: "independent".into(),
        checks: run
            .expected_checks()
            .into_iter()
            .map(|check_id| CheckResult {
                check_id,
                status: CheckStatus::Pass,
                method: "inspection".into(),
                evidence_ids: vec![e.id.clone()],
                rationale: "claimed pass".into(),
            })
            .collect(),
        findings: vec![],
    };
    assert!(quality::validate_review(&run, &a, &review).is_err());
}
#[test]
fn schema_requires_family_reports_and_actual_delivery_proof_reports_only_when_needed() {
    for (family, field) in [
        (TaskFamily::Research, "research"),
        (TaskFamily::Analysis, "analysis"),
        (TaskFamily::Document, "editorial"),
    ] {
        for stage in [Stage::Execution, Stage::Validation, Stage::Integration] {
            let (_root, run, a, e) = fixture(stage, family);
            let q = verify_schema(&run, &a, &e).schema;
            assert!(!nullable(&q["properties"]["quality"]["properties"][field]));
        }
    }
    let (_root, run, a, e) = fixture(Stage::Validation, TaskFamily::Software);
    let request = verify_schema(&run, &a, &e);
    let software = &request.schema["properties"]["quality"]["properties"]["software"]["properties"];
    assert!(!nullable(&software["static_analysis"]));
    assert!(!nullable(&software["behavior"]));
    let (_root, run, a, e) = fixture(Stage::Delivery, TaskFamily::Software);
    let q = verify_schema(&run, &a, &e).schema;
    assert!(!nullable(
        &q["properties"]["quality"]["properties"]["delivery"]
    ));
}
#[test]
fn executor_planning_does_not_require_future_runner_reports_and_missing_proof_can_be_reported() {
    let (_root, run, _a, _e) = fixture(Stage::Planning, TaskFamily::Software);
    let request = providers::build("fake")
        .unwrap()
        .prepare_execute(&ObservedInvocation {
            turn_id: "schema-test",
            run: &run,
            resume: None,
            context_packet: None,
        })
        .unwrap();
    let q = &request.schema["properties"]["quality"]["properties"];
    assert!(!nullable(&q["planning"]));
    assert!(!nullable(&q["software"]));
    assert!(nullable(&q["software"]["properties"]["static_analysis"]));
    assert!(nullable(&q["software"]["properties"]["behavior"]));
    let (_root, run, a, e) = fixture(Stage::Validation, TaskFamily::Software);
    let request = verify_schema(&run, &a, &e);
    let software = &request.schema["properties"]["quality"]["properties"]["software"]["properties"];
    let raw = &software["static_analysis"]["properties"]["analyses"]["items"]["properties"]["raw_evidence_ids"];
    assert!(raw.get("minItems").is_none_or(|v| v.as_u64() == Some(0)));
    assert!(raw["items"].get("enum").is_none());
    assert!(
        request
            .prompt
            .contains("Empty proof arrays remain schema-representable")
    );
}

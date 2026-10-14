//! Independent typed-acceptance schema and honest rejection tests, no native calls.
use architect::{
    domain::*,
    hash,
    providers::{ObservedInvocation, Provider, fake::FakeProvider},
    quality::{self, TaskFamily},
};
use serde_json::{Value, json};
use std::collections::BTreeSet;

fn accepts(schema: &Value, value: &Value) -> bool {
    if let Some(values) = schema.get("enum").and_then(Value::as_array)
        && !values.contains(value)
    {
        return false;
    }
    if let Some(choices) = schema.get("anyOf").and_then(Value::as_array) {
        return choices.iter().any(|choice| accepts(choice, value));
    }
    let kind = match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    };
    if let Some(expected) = schema.get("type") {
        let matches = |name: &str| name == kind || (name == "number" && kind == "integer");
        if !(expected.as_str().is_some_and(matches)
            || expected
                .as_array()
                .is_some_and(|types| types.iter().any(|t| t.as_str().is_some_and(matches))))
        {
            return false;
        }
    }
    if let Some(items) = value.as_array() {
        if schema
            .get("minItems")
            .and_then(Value::as_u64)
            .is_some_and(|min| items.len() < (min as usize))
            || schema
                .get("maxItems")
                .and_then(Value::as_u64)
                .is_some_and(|max| items.len() > (max as usize))
        {
            return false;
        }
        if let Some(item_schema) = schema.get("items")
            && !items.iter().all(|item| accepts(item_schema, item))
        {
            return false;
        }
    }
    if let Some(fields) = value.as_object() {
        if schema
            .get("required")
            .and_then(Value::as_array)
            .is_some_and(|required| {
                required
                    .iter()
                    .any(|k| !fields.contains_key(k.as_str().unwrap()))
            })
        {
            return false;
        }
        if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
            for (name, value) in fields {
                if let Some(child) = properties.get(name) {
                    if !accepts(child, value) {
                        return false;
                    }
                } else if schema["additionalProperties"] == false {
                    return false;
                }
            }
        }
    }
    true
}
fn minimal(schema: &Value) -> Value {
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        return values[0].clone();
    }
    if let Some(choices) = schema.get("anyOf").and_then(Value::as_array) {
        return minimal(
            choices
                .iter()
                .find(|choice| choice["type"] == "null")
                .unwrap_or(&choices[0]),
        );
    }
    if let Some(kinds) = schema["type"].as_array() {
        if kinds.iter().any(|kind| kind == "null") {
            return Value::Null;
        }
        let mut specific = schema.clone();
        specific["type"] = kinds[0].clone();
        return minimal(&specific);
    }
    match schema["type"].as_str().unwrap() {
        "object" => Value::Object(
            schema["properties"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(name, child)| (name.clone(), minimal(child)))
                .collect(),
        ),
        "array" => Value::Array(
            (0..schema["minItems"].as_u64().unwrap_or(0))
                .map(|_| minimal(&schema["items"]))
                .collect(),
        ),
        "string" => json!(""),
        "boolean" => json!(false),
        "integer" | "number" => json!(0),
        "null" => Value::Null,
        other => panic!("unhandled schema kind {other}"),
    }
}
fn fixture(stage: Stage, family: TaskFamily) -> (tempfile::TempDir, Run, Artifact, Evidence) {
    let root = tempfile::tempdir().unwrap();
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().canonicalize().unwrap().to_string_lossy().into();
    spec.provider = "codex".into();
    spec.task_family = family;
    let mut run = Run::new(architect::id(), spec).unwrap();
    run.stage = stage;
    run.attempt = 1;
    let content = "Exact pinned artifact".to_string();
    let artifact = Artifact {
        id: "independent-artifact".into(),
        stage,
        attempt: 1,
        spec_revision: 1,
        session_id: "executor".into(),
        hash: hash(&content),
        content,
    };
    run.artifacts.push(artifact.clone());
    let read = architect::verification::observe(&artifact);
    run.evidence.push(read.clone());
    let snapshot = serde_json::to_string(&run.input_snapshot).unwrap();
    run.evidence.push(Evidence {
        id: "actual-current-inputs".into(),
        artifact_hash: artifact.hash.clone(),
        spec_revision: 1,
        source: "runtime:input-snapshot".into(),
        output_hash: hash(&snapshot),
        input_hash: Some(hash(&snapshot)),
        output: snapshot,
        success: true,
    });
    (root, run, artifact, read)
}
fn prepared(
    run: &Run,
    artifact: &Artifact,
    read: &Evidence,
    verifier: bool,
) -> architect::providers::PreparedTurn {
    let invocation = ObservedInvocation {
        turn_id: "offline-quality-contract",
        run,
        resume: None,
        context_packet: None,
    };
    if verifier {
        FakeProvider
            .prepare_verify(&invocation, artifact, read)
            .unwrap()
    } else {
        FakeProvider.prepare_execute(&invocation).unwrap()
    }
}
fn required(run: &Run, verifier: bool) -> BTreeSet<&'static str> {
    let mut result = BTreeSet::new();
    if !verifier {
        if run.stage == Stage::Discovery {
            result.insert("context");
        }
        if run.stage == Stage::Planning {
            result.insert("planning");
            if run.spec.task_family == TaskFamily::Software {
                result.insert("software");
            }
        }
        return result;
    }
    let checks = run.expected_checks();
    for gate in quality::gate_definitions() {
        if !checks.contains(&quality::gate_check_id(&gate.gate_id, gate.phase)) {
            continue;
        }
        match gate.gate_id.as_str() {
            "context_ready" => {
                result.insert("context");
            }
            "solution_selected" | "plan_ready" => {
                result.insert("planning");
            }
            "requirements_complete" | "review_complete" => {
                result.insert("delivery");
            }
            _ if gate.software_only => {
                result.insert("software");
            }
            _ => (),
        }
    }
    if matches!(
        run.stage,
        Stage::Execution | Stage::Validation | Stage::Integration
    ) {
        match run.spec.task_family {
            TaskFamily::Research => {
                result.insert("research");
            }
            TaskFamily::Document => {
                result.insert("editorial");
            }
            TaskFamily::Analysis => {
                result.insert("analysis");
            }
            TaskFamily::Software => (),
        }
    }
    result
}
#[test]
fn required_report_types_follow_executor_phase_and_all_verifier_gate_obligations() {
    for stage in [
        Stage::Discovery,
        Stage::Planning,
        Stage::Execution,
        Stage::Validation,
        Stage::Integration,
        Stage::Delivery,
    ] {
        for family in [
            TaskFamily::Software,
            TaskFamily::Research,
            TaskFamily::Document,
            TaskFamily::Analysis,
        ] {
            let (_root, run, artifact, read) = fixture(stage, family);
            for verifier in [false, true] {
                let prepared = prepared(&run, &artifact, &read, verifier);
                let schema = &prepared.schema["properties"]["quality"];
                let mandatory = required(&run, verifier);
                for field in [
                    "context",
                    "planning",
                    "software",
                    "research",
                    "editorial",
                    "analysis",
                    "delivery",
                ] {
                    assert_eq!(
                        !accepts(&schema["properties"][field], &Value::Null),
                        mandatory.contains(field),
                        "{stage:?} {family:?} verifier={verifier} field={field}"
                    );
                }
                assert!(accepts(&schema["properties"]["family"], &json!(family)));
                assert!(!accepts(&schema["properties"]["synthetic"], &json!(true)));
                assert!(!accepts(
                    &schema["properties"]["spec_revision"],
                    &json!(run.spec_revision + 1)
                ));
                assert_eq!(
                    schema["properties"]["source_hashes"]["minItems"],
                    run.source_hashes().len()
                );
                assert_eq!(
                    schema["properties"]["source_hashes"]["maxItems"],
                    run.source_hashes().len()
                );
                assert!(
                    prepared
                        .prompt
                        .contains("Required non-null quality report DTOs for THIS invocation")
                );
            }
        }
    }
}
#[test]
fn absent_runner_proof_still_allows_truthful_typed_blocked_response() {
    let (_root, run, artifact, read) = fixture(Stage::Validation, TaskFamily::Software);
    let prepared = prepared(&run, &artifact, &read, true);
    let schema = &prepared.schema["properties"]["quality"];
    let mut report = minimal(schema);
    report["source_hashes"]=json!(run.source_hashes().into_iter().map(|(source_ref,source_hash)|json!({"source_ref":source_ref,"source_hash":source_hash})).collect::<Vec<_>>());
    assert!(
        accepts(schema, &report),
        "required DTOs with empty proof arrays must be schema representable"
    );
    let parsed: quality::QualityArtifacts = serde_json::from_value(report.clone()).unwrap();
    assert!(!parsed.synthetic);
    assert!(report["software"]["static_analysis"].is_object());
    assert!(report["software"]["behavior"].is_object());
    let mut response = json!({"quality":report,"checks":[],"findings":["Required runner observations are absent; acceptance blocked."],"agent_messages":[]});
    response["checks"]=json!(run.expected_checks().iter().map(|check_id|json!({"check_id":check_id,"status":"blocked","method":"Inspected supplied source evidence","evidence_ids":["actual-current-inputs"],"rationale":"Actual runner proof unavailable"})).collect::<Vec<_>>());
    assert!(
        accepts(&prepared.schema, &response),
        "honest blocked response must remain possible without inventing runner IDs"
    );
    let review = Review {
        quality: Some(parsed),
        artifact_hash: artifact.hash.clone(),
        spec_revision: run.spec_revision,
        checklist_version: run.checklist_version.clone(),
        executor_session: artifact.session_id.clone(),
        verifier_session: "independent-offline-verifier".into(),
        checks: vec![],
        findings: vec!["Missing actual runner proof".into()],
    };
    assert!(architect::verification::validate_review(&run, &artifact, &review).is_err());
}
#[test]
fn stale_prior_stage_gate_requires_context_and_planning_even_during_execution() {
    let (_root, mut run, artifact, read) = fixture(Stage::Execution, TaskFamily::Software);
    for (name, gate_id) in [
        ("context-old", "context_ready"),
        ("planning-old", "plan_ready"),
    ] {
        run.quality_gates.insert(
            name.into(),
            quality::GateAssessment {
                gate_id: gate_id.into(),
                phase: quality::GatePhase::Design,
                status: CheckStatus::Pass,
                spec_revision: run.spec_revision,
                source_hashes: run.source_hashes(),
                artifact_hash: hash("historical artifact"),
                review_hash: hash("missing prior review"),
                check_id: quality::gate_check_id(gate_id, quality::GatePhase::Design),
                evidence_ids: vec!["historical receipt".into()],
                report_hash: hash("historical typed report"),
            },
        );
    }
    let prepared = prepared(&run, &artifact, &read, true);
    for field in ["context", "planning", "software"] {
        assert!(!accepts(
            &prepared.schema["properties"]["quality"]["properties"][field],
            &Value::Null
        ));
    }
    assert_eq!(
        prepared
            .schema
            .pointer("/properties/checks/items/properties/check_id/enum")
            .unwrap(),
        &json!(run.expected_checks())
    );
    assert!(prepared.prompt.contains("prior-stage gates"));
}

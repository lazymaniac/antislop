//! Structural model-contract counterexamples; these are not authenticated model judgments.
use architect::{
    domain::*,
    team::{CatalogRole, TaskRecord, TaskStage, models::*},
};
use std::collections::BTreeSet;

#[test]
fn flat_context_finish_candidate_is_rejected_instead_of_becoming_an_artifact() {
    let candidate = serde_json::json!({"contract_version":CONTRACT,"artifact":"Rejected native-shape regression fixture; no model judgment", "edits":[],"agent_messages":[],
        "context":{"facts":[],"unknowns":[],"sections":{}},"solution":null});
    assert!(
        architect::transport::decode::<Output>(&serde_json::to_vec(&candidate).unwrap()).is_err()
    );
}

fn fixture() -> (tempfile::TempDir, Run, TaskRecord, Output) {
    let root = tempfile::tempdir().unwrap();
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().to_string_lossy().into();
    let run = Run::new(architect::id(), spec).unwrap();
    let requirements = run
        .spec
        .requirements
        .iter()
        .map(|r| r.id.clone())
        .collect::<BTreeSet<_>>();
    let criteria = run
        .spec
        .requirements
        .iter()
        .flat_map(|r| r.criteria.keys().map(|c| format!("REQ:{}:{c}", r.id)))
        .collect();
    let task = TaskRecord::new(
        "options".into(),
        "owner".into(),
        "verifier".into(),
        "Compare models".into(),
        TaskStage::Planning,
        requirements.clone(),
        criteria,
    );
    let mut model = SolutionModel {
        criteria: vec!["Preserve required behavior".into()],
        options: vec![
            SolutionOption {
                id: "a".into(),
                mechanism: "Direct transition".into(),
                requirement_ids: requirements.clone(),
                evidence: "Structural fixture A".into(),
            },
            SolutionOption {
                id: "b".into(),
                mechanism: "Queued transition".into(),
                requirement_ids: requirements,
                evidence: "Structural fixture B".into(),
            },
        ],
        chosen_option: "a".into(),
        rationale: "Structural comparison fixture".into(),
        invariants: vec![Invariant {
            id: "I1".into(),
            statement: "Transition remains authorized".into(),
        }],
        simulations: vec![],
    };
    let scenarios = SCENARIO_KINDS
        .iter()
        .map(|kind| {
            let scenario = SimulationCase {
                kind: (*kind).into(),
                initial_state: "Stopped with authorized input".into(),
                events: vec![format!("Observe {kind} condition")],
                expected_result: "Preserve invariant and required output".into(),
                invariant_ids: BTreeSet::from(["I1".into()]),
            };
            model.simulations.push(scenario.clone());
            ModelScenario {
                scenario_id: (*kind).into(),
                model_revision: run.spec_revision,
                criterion_ids: task.criterion_ids.clone(),
                scenario,
                outcome: ScenarioOutcome::Pass,
                rationale: "Structural fixture evaluation".into(),
            }
        })
        .collect();
    let output = Output {
        contract_version: CONTRACT.into(),
        artifact: "Structural model fixture".into(),
        edits: vec![],
        agent_messages: vec![],
        context: None,
        solution: Some(SolutionArtifact { model, scenarios }),
    };
    (root, run, task, output)
}

#[test]
fn multiple_cases_in_one_class_preserve_all_obligations_without_a_scenario_cap() {
    let (_root, run, task, mut output) = fixture();
    let solution = output.solution.as_mut().unwrap();
    for index in 0..512 {
        let mut assessment = solution.scenarios[0].clone();
        assessment.scenario_id = format!("additional-success-{index}");
        assessment.scenario.initial_state = format!("Authorized source variant {index}");
        solution.model.simulations.push(assessment.scenario.clone());
        solution.scenarios.push(assessment);
    }
    validate_output(&run, &task, CatalogRole::OptionsAnalyst, &output).unwrap();
    output
        .solution
        .as_mut()
        .unwrap()
        .scenarios
        .retain(|s| s.scenario.kind != "counterexample");
    assert!(
        validate_output(&run, &task, CatalogRole::OptionsAnalyst, &output).is_err(),
        "hundreds of successful cases cannot compensate for an omitted class"
    );
}

#[test]
fn justified_na_is_allowed_but_never_replaces_mandatory_simulation() {
    let (_root, run, task, mut output) = fixture();
    for assessment in &mut output.solution.as_mut().unwrap().scenarios {
        if !matches!(
            assessment.scenario.kind.as_str(),
            "success" | "failure" | "counterexample"
        ) {
            assessment.outcome = ScenarioOutcome::NotApplicable;
            assessment.rationale = "This isolated arithmetic transition owns no asynchronous process, integration or external resource; fixture only".into();
        }
    }
    validate_output(&run, &task, CatalogRole::OptionsAnalyst, &output).unwrap();
    output.solution.as_mut().unwrap().scenarios[0].outcome = ScenarioOutcome::NotApplicable;
    assert!(validate_output(&run, &task, CatalogRole::OptionsAnalyst, &output).is_err());
}

#[test]
fn explicit_absence_is_distinct_from_missing_nullable_fields_in_native_contract() {
    let (_root, _run, _task, output) = fixture();
    let mut value = serde_json::to_value(output).unwrap();
    architect::transport::decode::<Output>(&serde_json::to_vec(&value).unwrap()).unwrap();
    value.as_object_mut().unwrap().remove("context");
    assert!(architect::transport::decode::<Output>(&serde_json::to_vec(&value).unwrap()).is_err());
    let schema = output_schema();
    assert!(!serde_json::to_string(&schema).unwrap().contains("\"$ref\""));
    assert_eq!(
        schema["properties"]["context"]["anyOf"][0]["additionalProperties"],
        false
    );
}

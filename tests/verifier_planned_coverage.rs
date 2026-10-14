//! A plan must not claim that its future tests have already run.
use architect::{
    domain::{Run, RunSpec, Stage},
    quality::{self, InputInteraction},
};
use std::collections::BTreeSet;

#[test]
fn planning_accepts_unexecuted_scenarios_but_validation_requires_full_interaction_order() {
    let root = tempfile::tempdir().unwrap();
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().to_string_lossy().into();
    let mut run = Run::new(architect::id(), spec).unwrap();
    let mut matrix = quality::synthetic_artifacts(&run)
        .software
        .unwrap()
        .input_domain;
    let original = matrix.inputs[0].clone();
    for name in ["flag", "persisted-state"] {
        let mut input = original.clone();
        input.id = name.into();
        matrix.inventory.insert(name.into());
        matrix.inputs.push(input);
    }
    matrix.interactions = vec![InputInteraction {
        input_ids: matrix.inventory.clone(),
        rule: "All three inputs jointly determine authorization".into(),
        required_order: 3,
        tested_order: 0,
        expected_result: "Only the authorized triple succeeds".into(),
        scenario_ids: BTreeSet::from(["planned-three-way".into()]),
    }];
    run.stage = Stage::Planning;
    assert!(
        quality::validate_input_domain(&run, &matrix).is_ok(),
        "A plan must permit explicitly unexecuted future scenarios"
    );
    matrix.interactions[0].scenario_ids.clear();
    assert!(
        quality::validate_input_domain(&run, &matrix).is_err(),
        "Planning still needs an explicit scenario plan"
    );
    matrix.interactions[0]
        .scenario_ids
        .insert("planned-three-way".into());
    run.stage = Stage::Validation;
    assert!(quality::validate_input_domain(&run, &matrix).is_err());
    matrix.interactions[0].tested_order = 2;
    assert!(
        quality::validate_input_domain(&run, &matrix).is_err(),
        "Pairwise cannot stand in for three-way coverage"
    );
    matrix.interactions[0].tested_order = 3;
    quality::validate_input_domain(&run, &matrix).unwrap();
}

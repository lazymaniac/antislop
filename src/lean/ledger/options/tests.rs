use super::super::Run;
use super::SolutionChoice;
use super::model::Alternative;

#[test]
fn choice_cannot_select_an_uncompared_approach_or_use_incomparable_criteria() {
    let run = Run {
        id: "run".into(),
        conversation_id: "conversation".into(),
        goal: "goal".into(),
        spec_revision: 2,
        state: "active".into(),
        start_message: 1,
    };
    let alternative = |id: &str| Alternative {
        id: id.into(),
        approach: format!("Use {id}"),
        evaluations: vec!["Meets the requirement".into()],
        risks: "Risk is addressed by checks".into(),
        validation: "Run the acceptance checks".into(),
    };
    let mut choice = SolutionChoice {
        run_id: run.id.clone(),
        spec_revision: 2,
        criteria: vec!["Requirement coverage".into()],
        alternatives: vec![alternative("a"), alternative("b")],
        selected: "a".into(),
        rationale: "A meets the requirements with less complexity".into(),
        review_findings: vec![],
    };
    choice.validate(&run).unwrap();
    choice.selected = "uncompared".into();
    assert!(choice.validate(&run).is_err());
    choice.selected = "a".into();
    choice.alternatives[1].evaluations.clear();
    assert!(choice.validate(&run).is_err());
    choice.alternatives[1]
        .evaluations
        .push("Covers scope".into());
    choice.spec_revision = 1;
    assert!(choice.validate(&run).is_err());
}

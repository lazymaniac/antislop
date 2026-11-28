use super::*;
use serde_json::json;

#[test]
fn plans_that_write_get_the_required_quality_gates() {
    let tasks = submitted(json!({"tasks":[
        {"id":"t1","title":"A","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"g","change_kind": "code", "kind":"implementation", "open_decisions":["Choose the storage approach"]},
        {"id":"t2","title":"T","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"g","depends_on":["t1"],"change_kind": "code", "kind":"testing", "open_decisions":["Choose the compatibility test fixture"]}
    ]}))
    .unwrap();
    let ids: Vec<_> = missing_quality_agents(&tasks)
        .iter()
        .map(|a| a.id)
        .collect();
    assert_eq!(
        ids,
        vec![
            "quality-refactoring",
            "static-analyzer",
            "security-auditor",
            "performance-auditor",
            "integration-gate"
        ]
    );
    let full = with_options(with_required_agents(tasks));
    assert_eq!(full.len(), 9);
    validate(&full).unwrap();
    let gate = |kind| full.iter().find(|task| task.kind == kind).unwrap();
    let options: Vec<_> = full
        .iter()
        .filter(|task| task.options_for.is_some())
        .collect();
    for quality_gate in full.iter().filter(|task| task.quality_gate) {
        assert!(quality_gate.options_for.is_none());
        assert!(
            quality_gate
                .depends_on
                .iter()
                .all(|dependency| { options.iter().all(|option| option.id != *dependency) })
        );
    }
    let refactor = gate(WorkKind::Refactoring);
    assert_eq!(refactor.depends_on, vec!["t1", "t2"]);
    let lint = gate(WorkKind::StaticAnalysis);
    assert_eq!(lint.depends_on, vec!["t1", "t2", "quality-refactoring"]);
    assert!(serde_json::to_value(lint).unwrap().get("scope").is_none());
    assert!(!gate(WorkKind::Security).change_kind.has_delivery());
    assert_eq!(
        gate(WorkKind::Integration).depends_on,
        vec![
            "t1",
            "t2",
            "quality-refactoring",
            "static-analyzer",
            "security-auditor",
            "performance-auditor"
        ]
    );
    assert!(missing_quality_agents(&full).is_empty());

    // A read-only plan needs no gates.
    let tasks =
        submitted(json!({"tasks":[{"id":"t1","title":"a","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"b","change_kind": "none", "kind":"analysis"}]})).unwrap();
    assert_eq!(with_required_agents(tasks).len(), 1);
}

#[test]
fn plans_with_unknown_dependencies_are_rejected() {
    let task = |depends_on: &str| json!({"tasks":[{"id":"t1","title":"a","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"b","depends_on":[depends_on],"change_kind": "none", "kind":"analysis"}]});
    assert!(
        submitted(task("t9"))
            .unwrap_err()
            .to_string()
            .contains("t9")
    );
    assert!(submitted(task("t1")).is_err());
}

#[test]
fn login_and_limit_errors_wait_for_the_user() {
    assert!(needs_user_action("Error: Not logged in. Run `agy login`."));
    assert!(needs_user_action("429 Too Many Requests"));
    assert!(needs_user_action("You've hit your usage limit"));
    assert!(!needs_user_action("exit status 1: panicked at src/main.rs"));
}

use super::*;

#[test]
fn task_ids_reject_paths_and_empty_fragments() {
    for id in [
        "",
        "/tmp/task",
        "a/b",
        r"a\b",
        "..",
        "a..b",
        ".a",
        "a.",
        "a//b",
        "a b",
        "é",
    ] {
        let task = json!({"id": id, "title": "Task", "goal": "Assess", "kind": "analysis"});
        let error = submitted(json!({"tasks": [task]})).unwrap_err().to_string();
        assert!(error.contains("invalid task id"), "{id:?}: {error}");
    }
    let mut task: TaskContract = serde_json::from_value(json!({
        "id": "x".repeat(257), "title": "Task", "goal": "Assess",
        "kind": "analysis", "verifier": {"focus": "claims"}
    }))
    .unwrap();
    assert!(validate(&[task.clone()]).is_err());
    task.id = "x".repeat(256);
    task.writes = true;
    assert!(validate(&[task.clone()]).is_ok());
    let generated = with_options(vec![
        task.clone(),
        TaskContract {
            quality_gate: false,
            options_for: None,
            id: format!("{}-2", "x".repeat(254)),
            ..task
        },
    ]);
    validate(&generated).unwrap();
}

#[test]
fn saved_plans_keep_valid_ids_and_generated_options() {
    let ids = [
        "W1",
        "W2",
        "options-W1",
        "W1-options",
        "W1-options-2",
        "task_1.v2",
    ];
    let saved = json!({"tasks": ids.iter().map(|id| json!({
            "id": id, "title": "Task", "goal": "Assess", "kind": "implementation"
        })).collect::<Vec<_>>()});
    let tasks = submitted(saved).unwrap();
    assert_eq!(
        tasks
            .iter()
            .map(|task| task.id.as_str())
            .collect::<Vec<_>>(),
        ids
    );
    let restored: Vec<TaskContract> =
        serde_json::from_str(&serde_json::to_string(&tasks).unwrap()).unwrap();
    assert_eq!(restored, tasks);
    validate(&with_options(with_required_agents(restored))).unwrap();
}

#[test]
fn a_planned_task_gets_writes_from_its_kind_unless_it_says_otherwise() {
    let tasks = submitted(json!({"tasks":[
        {"id":"t1","title":"Storage","goal":"Assess storage","scope":["src/storage.rs"],"kind":"architecture"},
        {"id":"t2","title":"TUI","goal":"Change the TUI","depends_on":["t1"],"kind":"implementation"},
        {"id":"t3","title":"Probe","goal":"Run the probe","depends_on":["t1"],"kind":"analysis","writes":true}
    ]}))
    .unwrap();
    assert_eq!(tasks[1].depends_on, ["t1"]);
    assert_eq!(
        tasks.iter().map(|task| task.writes).collect::<Vec<_>>(),
        [false, true, true]
    );
    assert!(tasks.iter().all(|task| task.verifier.summary.is_some()));
}

#[test]
fn plan_summaries_are_optional_roundtrip_and_keep_the_strict_schema() {
    let base = json!({"id": "t1", "title": "Storage", "goal": "A detailed storage analysis.", "kind": "analysis"});
    let parse = |task: &Value| submitted(json!({"tasks": [task]}));
    let plain = parse(&base).unwrap();
    assert!(plain[0].summary.is_none());
    assert!(
        serde_json::to_value(&plain[0])
            .unwrap()
            .get("summary")
            .is_none()
    );
    for summary in [Value::Null, Value::String("Assess storage.".into())] {
        let mut spec = base.clone();
        spec["summary"] = summary.clone();
        let tasks = parse(&spec).unwrap();
        assert_eq!(tasks[0].summary, summary.as_str().map(str::to_owned));
        let roundtrip: TaskContract =
            serde_json::from_value(serde_json::to_value(&tasks[0]).unwrap()).unwrap();
        assert_eq!(roundtrip, tasks[0]);
    }
    for field in [
        "summery",
        "inputs",
        "outputs",
        "verifier",
        "requirement_ids",
    ] {
        let mut unknown = base.clone();
        unknown[field] = json!([]);
        assert!(parse(&unknown).is_err(), "{field}");
    }
}

#[test]
fn a_stored_contract_with_legacy_fields_still_loads() {
    let stored: TaskContract = serde_json::from_value(json!({
        "id": "t2", "title": "Use", "goal": "Use it", "kind": "analysis",
        "inputs": [{"name": "api", "task": "t1", "output": "api"}], "outputs": ["report"],
        "verifier": {"focus": "claims", "risks": ["units"], "risk_ids": ["R002"], "summary": "Check."}
    }))
    .unwrap();
    assert_eq!(stored.verifier.summary.as_deref(), Some("Check."));
    let saved = serde_json::to_value(&stored).unwrap();
    assert!(saved.get("inputs").is_none() && saved["verifier"].get("focus").is_none());
}

#[test]
fn a_plan_with_out_of_order_dependencies_is_ordered_and_a_cycle_is_rejected() {
    let first = json!({"id": "consumer", "title": "Consumer", "goal": "Use it",
        "kind": "analysis", "depends_on": ["scout"]});
    let second = json!({"id": "scout", "title": "Scout", "goal": "Find it", "kind": "analysis"});
    let tasks = submitted(json!({"tasks": [first, second]})).unwrap();
    let ids: Vec<_> = tasks.iter().map(|task| task.id.as_str()).collect();
    assert_eq!(ids, ["scout", "consumer"]);

    let cycle = |a: &str, b: &str| json!({"id": a, "title": a, "goal": "g", "kind": "analysis", "depends_on": [b]});
    assert!(submitted(json!({"tasks": [cycle("a", "b"), cycle("b", "a")]})).is_err());
}

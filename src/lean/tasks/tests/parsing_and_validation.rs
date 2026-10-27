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
        let task = json!({"id": id, "title": "Task", "goal": "Assess",
                "kind": "analysis", "verifier": {"focus": "claims"}});
        let answer = format!("```plan\n{}\n```", json!({"tasks": [task]}));
        let error = parse_plan(&answer).1.unwrap().unwrap_err().to_string();
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
            inputs: vec![],
            outputs: vec![],
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
            "id": id, "title": "Task", "goal": "Assess", "writes": true,
            "kind": "implementation", "verifier": {"focus": "claims"}
        })).collect::<Vec<_>>()});
    let answer = format!("```plan\n{saved}\n```");
    let tasks = parse_plan(&answer).1.unwrap().unwrap();
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
fn plan_block_is_split_from_the_answer() {
    let answer = "I will split this into two tasks.\n\n```plan\n{\"tasks\":[\
            {\"id\":\"t1\",\"title\":\"Storage\",\"goal\":\"Assess storage\",\"scope\":[\"src/storage.rs\"],\
             \"kind\":\"architecture\",\"verifier\":{\"focus\":\"claims\",\"risks\":[\"data loss\"]}},\
            {\"id\":\"t2\",\"title\":\"TUI\",\"goal\":\"Assess TUI\",\"depends_on\":[\"t1\"],\
             \"kind\":\"analysis\",\"verifier\":{\"focus\":\"claims\"}}]}\n```\n";
    let (text, plan) = parse_plan(answer);
    assert_eq!(text, "I will split this into two tasks.");
    let plan = plan.unwrap().unwrap();
    assert_eq!(plan.len(), 2);
    assert_eq!(plan[1].depends_on, ["t1"]);
    assert!(!plan[0].writes);
}

#[test]
fn plan_summaries_are_optional_roundtrip_and_keep_the_strict_schema() {
    let base = serde_json::json!({
        "id": "t1", "title": "Storage", "goal": "A detailed storage analysis.",
        "kind": "analysis", "verifier": {"focus": "Check the analysis in detail."}
    });
    let parse = |task: &Value| {
        parse_plan(&format!(
            "```plan\n{}\n```",
            serde_json::json!({"tasks": [task]})
        ))
        .1
        .unwrap()
    };
    let legacy = parse(&base).unwrap();
    assert!(legacy[0].summary.is_none());
    assert!(legacy[0].verifier.summary.is_none());
    let legacy_json = serde_json::to_value(&legacy[0]).unwrap();
    assert!(legacy_json.get("summary").is_none());
    assert!(legacy_json["verifier"].get("summary").is_none());
    for summary in [
        Value::Null,
        Value::String(" \n\t ".into()),
        Value::String("Assess storage.".into()),
    ] {
        let mut spec = base.clone();
        spec["summary"] = summary.clone();
        spec["verifier"]["summary"] = summary.clone();
        let tasks = parse(&spec).unwrap();
        assert_eq!(tasks[0].summary, summary.as_str().map(str::to_owned));
        assert_eq!(tasks[0].verifier.summary, tasks[0].summary);
        let roundtrip: TaskContract =
            serde_json::from_value(serde_json::to_value(&tasks[0]).unwrap()).unwrap();
        assert_eq!(roundtrip, tasks[0]);
    }
    let mut unknown = base.clone();
    unknown["summery"] = Value::String("typo".into());
    assert!(parse(&unknown).is_err());
    unknown = base.clone();
    unknown["verifier"]["summery"] = Value::String("typo".into());
    assert!(parse(&unknown).is_err());
}

#[test]
fn a_plan_with_out_of_order_dependencies_and_foreign_risks_is_repaired_not_rejected() {
    let first = json!({"id": "consumer", "title": "Consumer", "goal": "Use it",
        "kind": "analysis", "depends_on": ["scout"],
        "verifier": {"focus": "claims", "risk_ids": ["R002", "R999"]}});
    let second = json!({"id": "scout", "title": "Scout", "goal": "Find it",
        "kind": "analysis", "verifier": {"focus": "claims"}});
    let answer = format!("```plan\n{}\n```", json!({"tasks": [first, second]}));
    let tasks = parse_plan(&answer).1.unwrap().unwrap();
    let ids: Vec<_> = tasks.iter().map(|task| task.id.as_str()).collect();
    assert_eq!(ids, ["scout", "consumer"]);
    assert!(tasks[1].verifier.risk_ids.iter().all(|id| id != "R999"));

    let cycle = |a: &str, b: &str| {
        json!({"id": a, "title": a, "goal": "g", "kind": "analysis",
            "depends_on": [b], "verifier": {"focus": "claims"}})
    };
    let answer = format!(
        "```plan\n{}\n```",
        json!({"tasks": [cycle("a", "b"), cycle("b", "a")]})
    );
    assert!(parse_plan(&answer).1.unwrap().is_err());
}

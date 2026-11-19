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
        let task = json!({"id": id, "title": "Task", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal": "Assess", "change_kind": "none", "kind": "analysis"});
        let error = submitted(json!({"tasks": [task]})).unwrap_err().to_string();
        assert!(error.contains("invalid task id"), "{id:?}: {error}");
    }
    let mut task: TaskContract = serde_json::from_value(json!({
        "id": "x".repeat(257), "title": "Task", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal": "Assess",
        "change_kind": "none", "kind": "analysis", "reviewer": {"summary": "claims"}}))
    .unwrap();
    assert!(validate(&[task.clone()]).is_err());
    task.id = "x".repeat(256);
    task.change_kind = crate::lean::tasks::ChangeKind::Code;
    assert!(validate(&[task.clone()]).is_ok());
    let generated = with_options(vec![
        task.clone(),
        TaskContract {
            change_kind: crate::lean::tasks::ChangeKind::None,
            outputs: Vec::new(),
            delivery: None,
            quality_gate: false,
            integration_batch: false,
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
            "id": id, "title": "Task", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal": "Assess", "change_kind": "code", "kind": "implementation"
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
fn planned_tasks_preserve_explicit_delivery_independently_of_work_kind() {
    let tasks = submitted(json!({"tasks":[
        {"id":"t1","title":"Storage","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Assess storage","change_kind": "none", "kind":"architecture"},
        {"id":"t2","title":"TUI","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Change the TUI","depends_on":["t1"],"change_kind": "code", "kind":"implementation"},
        {"id":"t3","title":"Probe","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Run the probe","depends_on":["t1"],"change_kind": "code", "kind":"analysis"}
    ]}))
    .unwrap();
    assert_eq!(tasks[1].depends_on, ["t1"]);
    assert_eq!(
        tasks
            .iter()
            .map(|task| task.change_kind.has_delivery())
            .collect::<Vec<_>>(),
        [false, true, true]
    );
    assert!(tasks.iter().all(|task| task.reviewer.summary.is_some()));
}

#[test]
fn writer_checkout_names_are_frozen_from_planner_input_and_readers_have_none() {
    let tasks = submitted(json!({"tasks":[
        {"id":"writer-one","title":"Cache Layers & Paths","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Change storage","change_kind":"code","kind":"implementation"},
        {"id":"writer-two","title":"Custom label","checkout_name":"cache-v2","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Change storage too","change_kind":"code","kind":"implementation"},
        {"id":"reader-one","title":"Inspect","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Inspect storage","change_kind":"none","kind":"analysis"}
    ]}))
    .unwrap();
    assert_eq!(
        tasks[0].checkout_name.as_deref(),
        Some("cache-layers-paths")
    );
    assert_eq!(tasks[1].checkout_name.as_deref(), Some("cache-v2"));
    assert_eq!(tasks[2].checkout_name, None);
    let restored: Vec<TaskContract> =
        serde_json::from_value(serde_json::to_value(&tasks).unwrap()).unwrap();
    assert_eq!(restored, tasks);
}

#[test]
fn checkout_names_reject_invalid_labels_and_non_writers() {
    for name in [
        "",
        "Uppercase",
        "two words",
        "path/name",
        "-leading",
        "trailing-",
        &"a".repeat(49),
    ] {
        let error = submitted(json!({"tasks":[
            {"id":"writer-one","title":"Writer","checkout_name":name,"acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Write files","change_kind":"code","kind":"implementation"}
        ]}))
        .unwrap_err()
        .to_string();
        assert!(error.contains("checkout_name"), "{name:?}: {error}");
    }

    let tasks = submitted(json!({"tasks":[
        {"id":"reader-one","title":"Reader","checkout_name":"reader-label","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Inspect files","change_kind":"none","kind":"analysis"}
    ]}))
    .unwrap();
    assert_eq!(tasks[0].checkout_name, None);
}

#[test]
fn generated_writer_checkout_slug_respects_length_limit_at_separator_boundary() {
    let forty_seven = "a".repeat(47);
    let over_limit_title = format!("{forty_seven} b");
    let slug = crate::lean::tasks::checkout_slug(&over_limit_title);
    assert_eq!(slug, forty_seven);
    assert!(crate::lean::tasks::valid_checkout_name(&slug));

    let forty_six = "a".repeat(46);
    let exact_limit_title = format!("{forty_six} b");
    let slug = crate::lean::tasks::checkout_slug(&exact_limit_title);
    assert_eq!(slug, format!("{forty_six}-b"));
    assert_eq!(slug.len(), 48);
    assert!(crate::lean::tasks::valid_checkout_name(&slug));
}

#[test]
fn plan_summaries_are_optional_roundtrip_and_keep_the_strict_schema() {
    let base = json!({"id": "t1", "title": "Storage", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal": "A detailed storage analysis.", "change_kind": "none", "kind": "analysis"});
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
    for field in ["summery", "inputs", "reviewer", "requirement_ids"] {
        let mut unknown = base.clone();
        unknown[field] = json!([]);
        assert!(parse(&unknown).is_err(), "{field}");
    }
}

#[test]
fn contracts_reject_removed_fields_and_missing_change_kind() {
    let current = json!({"id":"t1","title":"Task","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Inspect","kind":"analysis","change_kind":"none"});
    assert!(serde_json::from_value::<TaskContract>(current.clone()).is_ok());
    for key in ["scope", "writes", "inputs"] {
        let mut old = current.clone();
        old[key] = json!([]);
        assert!(
            serde_json::from_value::<TaskContract>(old.clone()).is_err(),
            "{key}"
        );
        assert!(serde_json::from_value::<PlannedTask>(old).is_err(), "{key}");
    }
    let mut missing = current.clone();
    missing.as_object_mut().unwrap().remove("change_kind");
    assert!(serde_json::from_value::<TaskContract>(missing.clone()).is_err());
    assert!(serde_json::from_value::<PlannedTask>(missing).is_err());
    let mut reviewer = current;
    reviewer["reviewer"] = json!({"focus":"claims"});
    assert!(serde_json::from_value::<TaskContract>(reviewer).is_err());
}

#[test]
fn a_plan_with_out_of_order_dependencies_is_ordered_and_a_cycle_is_rejected() {
    let first = json!({"id": "consumer", "title": "Consumer", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal": "Use it",
        "change_kind": "none", "kind": "analysis", "depends_on": ["scout"]});
    let second = json!({"id": "scout", "title": "Scout", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal": "Find it", "change_kind": "none", "kind": "analysis"});
    let tasks = submitted(json!({"tasks": [first, second]})).unwrap();
    let ids: Vec<_> = tasks.iter().map(|task| task.id.as_str()).collect();
    assert_eq!(ids, ["scout", "consumer"]);

    let cycle = |a: &str, b: &str| json!({"id": a, "title": a, "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal": "g", "change_kind": "none", "kind": "analysis", "depends_on": [b]});
    assert!(submitted(json!({"tasks": [cycle("a", "b"), cycle("b", "a")]})).is_err());
}

#[test]
fn local_criteria_are_required_unique_nonempty_and_distinct_from_runtime_checks() {
    let mut value = serde_json::json!({"id":"t","title":"Export","goal":"Export locally","kind":"implementation","change_kind":"code"});
    let check = |value: &serde_json::Value| {
        let task: TaskContract = serde_json::from_value(value.clone()).unwrap();
        super::super::validate(&[task])
    };
    assert!(check(&value).is_err());
    for criteria in [
        serde_json::json!([{"id":"goal","text":"x"}]),
        serde_json::json!([{"id":"R001","text":"x"}]),
        serde_json::json!([{"id":"C1","text":" "}]),
        serde_json::json!([{"id":"C1","text":"x"},{"id":"C1","text":"y"}]),
    ] {
        value["acceptance_criteria"] = criteria;
        assert!(check(&value).is_err(), "{value}");
    }
    value["acceptance_criteria"] =
        serde_json::json!([{"id":"C1","text":"Do not upload records; preserve café 👩‍💻"}]);
    assert!(check(&value).is_ok());
    let task: TaskContract = serde_json::from_value(value.clone()).unwrap();
    let prompt = super::super::obligations(&task);
    assert!(
        prompt
            .iter()
            .any(|(id, text)| id == "C1" && text.contains("café 👩‍💻"))
    );
    value["covers"] = serde_json::json!(["C1"]);
    assert!(serde_json::from_value::<TaskContract>(value).is_err());
}

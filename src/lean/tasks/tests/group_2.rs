use super::*;

#[test]
fn progress_lines_are_read_however_they_are_decorated() {
    let update = |id: &str, status: Option<&str>, note: &str| {
        Some(CheckUpdate {
            id: id.into(),
            status: status.map(Into::into),
            note: note.into(),
        })
    };
    assert_eq!(parse_progress("CHECKING R1"), update("R1", None, ""));
    assert_eq!(
        parse_progress("- **CHECK V2 ok**: src/a.rs:10 handles it"),
        update("V2", Some("ok"), "src/a.rs:10 handles it")
    );
    assert_eq!(
        parse_progress("`CHECK C3 N/A: nothing to migrate`"),
        update("C3", Some("n/a"), "nothing to migrate")
    );
    assert_eq!(parse_progress("CHECK R1 maybe: x"), None);
    assert_eq!(parse_progress("CHECK X1 ok: x"), None);
    assert_eq!(parse_progress("Checking the code now"), None);
}

#[test]
fn progress_survives_lines_split_across_deltas() {
    let mut progress = Progress::default();
    assert!(progress.push("CHECKING R").is_empty());
    let first = progress.push("1\nCHECK R1 issue: br");
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].status, None);
    assert!(progress.push("oken").is_empty());
    let last = progress.finish();
    assert_eq!(last[0].note, "broken");
    assert_eq!(last[0].status.as_deref(), Some("issue"));
}
#[test]
fn a_recover_block_is_split_off_with_its_steps() {
    let answer = "Retrying.\n```recover\n{\"actions\":[{\"task\":\"t1\",\"action\":\"extend_rounds\"}]}\n```";
    let (text, steps) = parse_recoveries(answer);
    assert_eq!(text, "Retrying.");
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].task, "t1");
    assert_eq!(parse_recoveries("a\n```recover\nnope\n```").1, vec![]);
    assert_eq!(parse_recoveries("plain"), ("plain".to_owned(), vec![]));
}

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
    let mut task: TaskSpec = serde_json::from_value(json!({
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
        TaskSpec {
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
    let restored: Vec<TaskSpec> =
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
        let roundtrip: TaskSpec =
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

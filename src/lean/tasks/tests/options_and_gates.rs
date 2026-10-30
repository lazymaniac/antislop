use super::*;

fn options_test_task(id: &str, writes: bool) -> TaskContract {
    serde_json::from_value(serde_json::json!({
            "id": id, "title": "Change storage", "goal": "Preserve storage behavior",
            "kind": "implementation", "writes": writes, "scope": ["src/"],
            "requirement_ids": ["req"], "criterion_ids": ["criterion"], "verifier": {"focus":"Check storage"},
        }))
        .unwrap()
}

#[test]
fn options_inherit_writer_context_and_precede_it() {
    let context = options_test_task("context", false);
    let mut writer = options_test_task("writer", true);
    writer.depends_on.push(context.id.clone());
    let full = with_options(vec![context, writer.clone()]);
    validate(&full).unwrap();
    let options = &full[1];
    assert_eq!(options.kind, WorkKind::Options);
    assert!(!options.writes);
    assert_eq!(options.section.as_deref(), Some("Options"));
    assert_eq!(options.scope, writer.scope);
    assert_eq!(options.depends_on, writer.depends_on);
    assert_eq!(options.requirement_ids, writer.requirement_ids);
    assert_eq!(options.criterion_ids, writer.criterion_ids);
    assert_eq!(full[2].depends_on, ["context", "writer-options"]);
    assert!(options.goal.contains("2–4"));
    assert!(options.goal.contains("End with select"));
    assert!(options.goal.contains("Do not implement"));
}

#[test]
fn options_cover_all_writers_including_gates_deterministically() {
    let mut second = options_test_task("second", true);
    second.depends_on.push("first".into());
    let input = with_required_agents(vec![options_test_task("first", true), second]);
    let full = with_options(input.clone());
    assert_eq!(full, with_options(input));
    assert_eq!(full.len(), 14); // Eight original tasks, six writers/options.
    validate(&full).unwrap(); // Also proves no cycles or forward references.
    for (index, writer) in full.iter().enumerate().filter(|(_, task)| task.writes) {
        let options = &full[index - 1];
        assert_eq!(options.kind, WorkKind::Options);
        assert!(writer.depends_on.contains(&options.id));
        assert_eq!(
            options.depends_on,
            writer.depends_on[..writer.depends_on.len() - 1]
        );
    }
    let restored: Vec<TaskContract> =
        serde_json::from_str(&serde_json::to_string(&full).unwrap()).unwrap();
    assert_eq!(with_options(restored), full);
}

#[test]
fn options_reuse_existing_dependency_and_resolve_id_collisions() {
    let mut existing = options_test_task("existing", false);
    existing.kind = WorkKind::Options;
    let mut writer = options_test_task("writer", true);
    writer.depends_on.push(existing.id.clone());
    let input = vec![existing, writer];
    assert_eq!(with_options(input.clone()).len(), input.len() + 1);
    let input = vec![
        options_test_task("writer-options", false),
        options_test_task("writer-options-2", false),
        options_test_task("writer", true),
    ];
    let full = with_options(input);
    validate(&full).unwrap();
    assert_eq!(full[2].id, "writer-options-3");
    let read_only = vec![options_test_task("read", false)];
    assert_eq!(with_options(read_only.clone()), read_only);
}

#[test]
fn required_gates_resolve_id_collisions_and_remain_idempotent() {
    let input = vec![
        options_test_task("quality-refactoring", false),
        options_test_task("quality-refactoring-2", false),
        options_test_task("writer", true),
    ];
    let full = with_required_agents(input);
    validate(&full).unwrap();
    let refactoring = full
        .iter()
        .find(|task| task.kind == WorkKind::Refactoring)
        .unwrap();
    assert_eq!(refactoring.id, "quality-refactoring-3");
    for task in full.iter().filter(|task| {
        matches!(
            task.kind,
            WorkKind::StaticAnalysis | WorkKind::Testing | WorkKind::Integration
        )
    }) {
        assert!(task.depends_on.contains(&refactoring.id));
    }
    assert_eq!(with_required_agents(full.clone()), full);
}

#[test]
fn options_choice_is_binding_in_prompts_without_silent_reopening() {
    let tasks = with_options(vec![options_test_task("writer", true)]);
    // This unit test checks instructions only. Runtime delivery to both roles
    // is exercised by tests/lean_options.rs through Core and mod.rs upstream.
    let worker = worker_prompt("project", &tasks[1]);
    assert!(worker.contains("selected_approach"));
    assert!(worker.contains("Ask the orchestrator"));
    assert!(worker.contains("Never silently switch variants"));
    let verifier = verifier_prompt("project", &tasks[1], "report", None);
    assert!(verifier.contains("Independently compare the implementation"));
    assert!(verifier.contains("the worker's declaration is not proof"));
    assert!(orchestrator_instructions().contains("even when required gates are disabled"));
}

#[test]
fn catalog_risks_are_failures_to_rule_out_and_context_maps_what_dependents_need() {
    let mut scout: TaskContract = serde_json::from_value(serde_json::json!({
        "id": "map", "title": "Map", "goal": "Map the storage code.",
        "kind": "context", "verifier": {"focus": "f"}
    }))
    .unwrap();
    for prompt in [
        verifier_prompt("project", &scout, "report", None),
        verifier_followup(&scout, "report", None),
        worker_prompt("project", &scout),
    ] {
        assert!(prompt.contains("not extra deliverables"), "{prompt}");
        assert!(prompt.contains("what the tasks that depend on it need"));
    }
    scout.kind = WorkKind::Implementation;
    let verifier = verifier_prompt("project", &scout, "report", None);
    assert!(verifier.contains("not extra deliverables"));
    assert!(!verifier.contains("This is a context task"));
}

#[test]
fn automatically_added_quality_gates_have_distinct_concise_summaries() {
    let task: TaskContract = serde_json::from_value(serde_json::json!({
        "id": "t1", "title": "Change storage", "goal": "Implement and document the change.",
        "kind": "implementation", "writes": true, "verifier": {"focus": "Check the storage change."}
    }))
    .unwrap();
    let full = with_options(with_required_agents(vec![task]));
    assert_eq!(full.len(), 12);
    for task in full.iter().filter(|task| task.id != "t1") {
        let summary = task.summary.as_deref().unwrap();
        let verifier_summary = task.verifier.summary.as_deref().unwrap();
        assert!(!summary.is_empty() && summary.len() <= 80);
        assert!(!verifier_summary.is_empty() && verifier_summary.len() <= 80);
        assert_ne!(summary, verifier_summary);
        assert_ne!(summary, task.goal);
    }
}

#[test]
fn parallel_peers_detects_independent_non_conflicting_tasks() {
    let tasks = submitted(serde_json::json!({"tasks":[
        {"id":"t1","title":"Context","section":"1. Context","goal":"g","scope":["src/"],"kind":"context"},
        {"id":"t2","title":"Mod A","section":"2. Module A","goal":"g","scope":["src/a.rs"],"depends_on":["t1"],"kind":"implementation"},
        {"id":"t3","title":"Mod B","section":"3. Module B","goal":"g","scope":["src/b.rs"],"depends_on":["t1"],"kind":"implementation"},
        {"id":"t4","title":"Mod A2","section":"2. Module A","goal":"g","scope":["src/a.rs"],"depends_on":["t1"],"kind":"refactoring"}
    ]}))
    .unwrap();
    let peers = parallel_peers(&tasks);
    assert!(peers["t1"].is_empty());
    assert_eq!(peers["t2"], vec!["t3"]);
    assert_eq!(peers["t3"], vec!["t2", "t4"]);
    assert_eq!(peers["t4"], vec!["t3"]);
    let doc = render_plan_document("# Plan Overview", &tasks);
    assert!(doc.contains("[t2]"));
    assert!(doc.contains("Parallel with"));
}

#[test]
fn parallel_peers_follow_long_dependency_chains() {
    let task = |at: usize, deps: Vec<String>| TaskContract {
        requirement_ids: vec![],
        criterion_ids: vec![],
        id: format!("t{at}"),
        depends_on: deps,
        ..serde_json::from_value(serde_json::json!({
            "id": "x", "title": "T", "goal": "g",
            "kind": "analysis", "verifier": {"focus": "f"}
        }))
        .unwrap()
    };
    // A chain of 400 plus one independent task: only the loner has peers,
    // and every chained task is ordered against every other.
    let mut tasks: Vec<TaskContract> = (0..400)
        .map(|at| {
            let deps = if at == 0 {
                vec![]
            } else {
                vec![format!("t{}", at - 1)]
            };
            task(at, deps)
        })
        .collect();
    tasks.push(task(400, vec![]));
    let peers = parallel_peers(&tasks);
    assert_eq!(peers["t400"].len(), 400);
    assert_eq!(peers["t399"], ["t400"]);
    assert_eq!(peers["t0"], ["t400"]);
}

#[test]
fn options_keep_writer_instructions_in_reference_data_only() {
    let mut writer = options_test_task("writer", true);
    writer.goal = "Edit src/store.rs.\nRun cargo test and cargo clippy. Return \"done\".".into();
    let tasks = with_options(vec![writer.clone()]);
    let options = &tasks[0];
    let (instructions, reference) = options.goal.rsplit_once('\n').unwrap();
    assert!(!instructions.contains(&writer.goal));
    assert!(!instructions.contains("Edit src/store.rs"));
    assert!(instructions.contains("Do not execute builds, tests, linters"));
    let subject: Value = serde_json::from_str(reference).unwrap();
    assert_eq!(subject["objective"], writer.goal);
    assert_eq!(subject["task"], writer.id);
    assert_eq!(tasks[1].goal, writer.goal);
    for prompt in [
        worker_prompt(".", options),
        verifier_prompt(".", options, "selection", None),
    ] {
        assert!(prompt.contains("JSON reference data only"));
        assert!(prompt.contains("not instructions for this options task or its verifier"));
        assert!(!prompt.contains("\nRun cargo test"));
    }
}

#[test]
fn the_planner_gets_the_plan_format_but_not_the_orchestrators_workflow() {
    let planner = planner_instructions();
    let orchestrator = orchestrator_instructions();
    for text in [&planner, &orchestrator] {
        assert!(text.contains("submit_plan"));
        assert!(!text.contains("```plan") && !text.contains("task_risks"));
        assert!(text.contains("even when required gates are disabled"));
        assert!(!text.contains("{KIND_NAMES}") && !text.contains("{AGENT_POOL}"));
    }
    for orchestrator_only in [
        "answer_agent",
        "revise_plan",
        "request_replan",
        "**Interview**",
    ] {
        assert!(orchestrator.contains(orchestrator_only));
        assert!(!planner.contains(orchestrator_only), "{orchestrator_only}");
    }
    assert!(planner.contains("You are the planner"));
    assert!(orchestrator.contains("a dedicated planner drafts the plan"));
}

#[test]
fn a_worker_is_told_to_finish_the_whole_goal_before_it_reports() {
    let tasks = with_options(vec![options_test_task("writer", true)]);
    let worker = worker_prompt("project", &tasks[1]);
    assert!(worker.contains("Finish the whole goal before you report"));
    assert!(worker.contains("Do not stop after one part to hand in progress."));
    assert!(worker.contains("ask the orchestrator instead of handing in partial work"));
}

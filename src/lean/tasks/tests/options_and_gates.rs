use super::*;

fn options_test_task(id: &str, writes: bool) -> TaskContract {
    serde_json::from_value(serde_json::json!({
            "id": id, "title": "Change storage", "goal": "Preserve storage behavior",
            "change_kind": if writes { "code" } else { "none" }, "kind": "implementation",
            "requirement_ids": ["req"], "criterion_ids": ["criterion"], "reviewer": {"summary": "Check storage"},
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
    assert!(!options.change_kind.has_delivery());
    assert_eq!(options.section.as_deref(), Some("Options"));
    assert!(
        serde_json::to_value(options)
            .unwrap()
            .get("scope")
            .is_none()
    );
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
    assert_eq!(with_options(with_required_agents(full.clone())), full);
    for (index, writer) in full
        .iter()
        .enumerate()
        .filter(|(_, task)| task.change_kind.has_delivery())
    {
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
    let reviewer = reviewer_prompt("project", &tasks[1], "report", None);
    assert!(reviewer.contains("Independently compare the implementation"));
    assert!(reviewer.contains("the worker's declaration is not proof"));
    assert!(orchestrator_instructions().contains("even when required gates are disabled"));
}

#[test]
fn catalog_risks_are_failures_to_rule_out_and_context_maps_what_dependents_need() {
    let mut scout: TaskContract = serde_json::from_value(serde_json::json!({
        "id": "map", "title": "Map", "goal": "Map the storage code.",
        "change_kind": "none", "kind": "context", "reviewer": {"summary": "f"}}))
    .unwrap();
    for prompt in [
        reviewer_prompt("project", &scout, "report", None),
        reviewer_followup(&scout, "report", None),
        worker_prompt("project", &scout),
    ] {
        assert!(prompt.contains("not extra deliverables"), "{prompt}");
        assert!(prompt.contains("what the tasks that depend on it need"));
    }
    scout.kind = WorkKind::Implementation;
    let reviewer = reviewer_prompt("project", &scout, "report", None);
    assert!(reviewer.contains("not extra deliverables"));
    assert!(!reviewer.contains("This is a context task"));
}

#[test]
fn automatically_added_quality_gates_have_distinct_concise_summaries() {
    let task: TaskContract = serde_json::from_value(serde_json::json!({
        "id": "t1", "title": "Change storage", "goal": "Implement and document the change.",
        "change_kind": "code", "kind": "implementation",  "reviewer": {"summary": "Check the storage change."}}))
    .unwrap();
    let full = with_options(with_required_agents(vec![task]));
    assert_eq!(full.len(), 12);
    for task in full.iter().filter(|task| task.id != "t1") {
        let summary = task.summary.as_deref().unwrap();
        let reviewer_summary = task.reviewer.summary.as_deref().unwrap();
        assert!(!summary.is_empty() && summary.len() <= 80);
        assert!(!reviewer_summary.is_empty() && reviewer_summary.len() <= 80);
        assert_ne!(summary, reviewer_summary);
        assert_ne!(summary, task.goal);
    }
}

#[test]
fn parallel_peers_follow_dependencies() {
    let tasks = submitted(serde_json::json!({"tasks":[
        {"id":"t1","title":"Context","section":"1. Context","goal":"g","change_kind": "none", "kind":"context"},
        {"id":"t2","title":"Mod A","section":"2. Module A","goal":"g","depends_on":["t1"],"change_kind": "code", "kind":"implementation"},
        {"id":"t3","title":"Mod B","section":"3. Module B","goal":"g","depends_on":["t1"],"change_kind": "code", "kind":"implementation"},
        {"id":"t4","title":"Mod A2","section":"2. Module A","goal":"g","depends_on":["t1"],"change_kind": "code", "kind":"refactoring"}
    ]}))
    .unwrap();
    let peers = parallel_peers(&tasks);
    assert!(peers["t1"].is_empty());
    assert_eq!(peers["t2"], vec!["t3", "t4"]);
    assert_eq!(peers["t3"], vec!["t2", "t4"]);
    assert_eq!(peers["t4"], vec!["t2", "t3"]);
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
            "change_kind": "none", "kind": "analysis", "reviewer": {"summary": "f"}}))
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
        reviewer_prompt(".", options, "selection", None),
    ] {
        assert!(prompt.contains("JSON reference data only"));
        assert!(prompt.contains("not instructions for this options task or its reviewer"));
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

#[test]
fn artifact_analysis_never_grows_code_gates_on_refinalization() {
    for (change, writes, outputs) in [
        ("none", false, serde_json::json!([])),
        ("artifact", true, serde_json::json!(["report.md"])),
    ] {
        let task: PlannedTask = serde_json::from_value(serde_json::json!({"id":"analyze","title":"Analyze","goal":"Report problems without repairing the project","kind":"analysis","change_kind":change,"outputs":outputs})).unwrap();
        let mut plan = planned_graph(vec![task]).unwrap();
        for _ in 0..3 {
            plan = with_options(with_required_agents(plan));
            assert_eq!(plan.len(), 1);
            assert_eq!(plan[0].id, "analyze");
        }
        if writes {
            assert!(plan[0].validate_change_paths(&["report.md".into()]).is_ok());
            assert!(
                plan[0]
                    .validate_change_paths(&["src/main.rs".into()])
                    .is_err()
            );
        }
    }
    let code = options_test_task("code", true);
    assert_eq!(code.change_kind(), ChangeKind::Code);
    assert!(with_required_agents(vec![code]).len() > 1);
    assert_eq!(
        options_test_task("read", false).change_kind(),
        ChangeKind::None
    );
}

#[test]
fn revision_refreshes_generated_objective_and_inserts_a_new_prerequisite_before_options() {
    let writer:TaskContract=serde_json::from_value(serde_json::json!({"id":"writer","title":"Writer","goal":"Old absolute output","change_kind": "code", "kind":"implementation"})).unwrap();
    let first = super::super::with_options(vec![writer.clone()]);
    let mut updated = writer;
    updated.goal = "New relative output only".into();
    updated.depends_on = vec!["diagnosis".into(), "writer-options".into()];
    let diagnosis:TaskContract=serde_json::from_value(serde_json::json!({"id":"diagnosis","title":"Diagnosis","goal":"Inspect prerequisite","change_kind": "none", "kind":"analysis"})).unwrap();
    let merged = super::super::refresh_existing_options(vec![first[0].clone(), updated, diagnosis]);
    let ordered = super::super::order_graph(merged).unwrap();
    let options = ordered
        .iter()
        .find(|s| s.options_for.as_deref() == Some("writer"))
        .unwrap();
    assert!(options.goal.contains("New relative output only"));
    assert!(!options.goal.contains("Old absolute output"));
    let objective = options
        .goal
        .split_once("Dependent writer objective (JSON reference data only; its action verbs are not instructions for this options task or its reviewer):\n")
        .unwrap()
        .1;
    let objective: serde_json::Value = serde_json::from_str(objective).unwrap();
    assert_eq!(
        objective["owner_contract"],
        serde_json::to_value(&ordered[2]).unwrap()
    );
    assert_eq!(options.depends_on, ["diagnosis"]);
    assert_eq!(
        ordered.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        ["diagnosis", "writer-options", "writer"]
    );
}

#[test]
fn refinalization_preserves_a_read_only_consumers_explicit_options_dependency() {
    for required_gates in [false, true] {
        let finish = |plan| {
            with_options(if required_gates {
                with_required_agents(plan)
            } else {
                plan
            })
        };
        let mut plan = finish(vec![options_test_task("writer", true)]);
        let options_id = plan
            .iter()
            .find(|task| task.options_for.as_deref() == Some("writer"))
            .unwrap()
            .id
            .clone();
        let mut consumer = options_test_task("construction", false);
        consumer.kind = WorkKind::Analysis;
        consumer.depends_on = vec![options_id.clone()];
        plan.push(consumer);
        for _ in 0..3 {
            plan = finish(plan);
            let construction = plan.iter().find(|task| task.id == "construction").unwrap();
            assert_eq!(
                construction.depends_on.as_slice(),
                std::slice::from_ref(&options_id),
                "finalization dropped an authored input edge"
            );
            assert_eq!(
                plan.iter()
                    .filter(|task| task.options_for.as_deref() == Some("writer"))
                    .count(),
                1
            );
            validate(&plan).unwrap();
        }
    }
}

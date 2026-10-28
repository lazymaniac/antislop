use super::core_plan_helpers::*;
use super::*;

#[test]
fn finalization_leaves_out_options_when_local_options_are_off() {
    let writer: TaskContract = serde_json::from_value(json!({
        "id": "writer", "title": "Implement change", "goal": "Preserve behavior",
        "kind": "implementation", "writes": true, "scope": ["src/"],
        "verifier": {"focus": "Check change"}
    }))
    .unwrap();
    for required_gates in [false, true] {
        let with = finalize_plan(vec![writer.clone()], required_gates, true);
        let without = finalize_plan(with.clone(), required_gates, false);
        assert!(without.iter().all(|spec| spec.kind != WorkKind::Options));
        assert!(without.iter().all(|spec| spec.options_for.is_none()));
        let ids: Vec<_> = without.iter().map(|spec| &spec.id).collect();
        assert!(
            without
                .iter()
                .flat_map(|spec| &spec.depends_on)
                .all(|dependency| ids.contains(&dependency))
        );
        tasks::validate(&without).unwrap();
        assert_eq!(
            without,
            finalize_plan(without.clone(), required_gates, false)
        );
    }
}

#[test]
fn finalization_adds_options_with_and_without_required_gates() {
    let writer: TaskContract = serde_json::from_value(json!({
        "id": "writer", "title": "Implement change", "goal": "Preserve behavior",
        "kind": "implementation", "writes": true, "scope": ["src/"],
        "verifier": {"focus": "Check change"}
    }))
    .unwrap();
    for required_gates in [false, true] {
        let input = vec![writer.clone()];
        let specs = finalize_plan(input.clone(), required_gates, true);
        assert_eq!(specs, finalize_plan(input, required_gates, true));
        assert_eq!(specs, finalize_plan(specs.clone(), required_gates, true));
        if required_gates {
            assert!(tasks::missing_quality_agents(&specs).is_empty());
            assert!(specs.iter().filter(|spec| spec.writes).count() > 1);
        } else {
            assert_eq!(specs.len(), 2);
            assert!(!tasks::missing_quality_agents(&specs).is_empty());
        }
        for writer in specs.iter().filter(|spec| spec.writes) {
            assert!(writer.depends_on.iter().any(|id| {
                specs
                    .iter()
                    .any(|spec| spec.id == *id && spec.kind == WorkKind::Options && !spec.writes)
            }));
        }
    }
}

#[test]
fn writer_base_through_options_includes_both_dependency_chains_and_local_user_changes() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    let init = std::process::Command::new("git")
        .args(["init", "--quiet", "-b", "main"])
        .current_dir(root)
        .status()
        .unwrap();
    assert!(init.success());
    std::fs::write(root.join("user.txt"), "committed\n").unwrap();
    let head = git::commit_all(root, "initial").unwrap();
    let config = Config::from_toml(include_str!("../../examples/antislop.toml")).unwrap();
    let core = Core::open(&root.join(".antislop"), config).unwrap();
    let conversation = core.start(root).unwrap();
    let specs: Vec<_> = [
        ("t1", vec![]),
        ("t2", vec!["t1"]),
        ("t3", vec![]),
        ("t4", vec!["t3"]),
        ("t5", vec!["t2", "t4"]),
    ]
    .into_iter()
    .map(|(id, deps)| {
        json!({
            "id": id, "title": id, "goal": "test", "kind": "implementation",
            "writes": true, "depends_on": deps, "verifier": {"focus": "inspect diff"}
        })
    })
    .collect();
    let specs = finalize_plan(
        specs
            .into_iter()
            .map(|spec| serde_json::from_value(spec).unwrap())
            .collect(),
        false,
        true,
    );
    let values: Vec<_> = specs
        .iter()
        .map(|spec| serde_json::to_value(spec).unwrap())
        .collect();
    let stored = core.store.add_plan(&conversation.id, &values).unwrap();
    for mut options in stored
        .iter()
        .filter(|task| task.spec["kind"] == "options")
        .cloned()
    {
        assert_eq!(options.spec["writes"], false);
        assert!(options.base.is_none() && options.result.is_none());
        options.status = status::APPROVED.into();
        core.store.update_task(&options).unwrap();
    }
    let mut tasks: Vec<_> = stored
        .into_iter()
        .filter(|task| task.spec["writes"] == true)
        .collect();
    for index in 0..4 {
        let base = if index % 2 == 0 {
            head.clone()
        } else {
            tasks[index - 1].result.clone().unwrap()
        };
        let path = root
            .join(".antislop/worktrees")
            .join(format!("input-{index}"));
        let branch = format!("test/input-{index}");
        git::add_worktree(root, &path, &branch, &base).unwrap();
        let name = if index < 2 { "config.txt" } else { "ui.txt" };
        std::fs::write(path.join(name), format!("revision {index}\n")).unwrap();
        tasks[index].base = Some(base);
        tasks[index].result = Some(git::commit_all(&path, name).unwrap());
        tasks[index].status = status::APPROVED.into();
        tasks[index].round = 1;
        core.store.update_task(&tasks[index]).unwrap();
        git::remove_worktree(root, &path, &branch).unwrap();
    }
    std::fs::write(root.join("user.txt"), "local edit\n").unwrap();
    std::fs::write(root.join("draft.txt"), "untracked\n").unwrap();
    let spec: TaskContract = serde_json::from_value(tasks[4].spec.clone()).unwrap();
    let dependencies = core.dependency_tasks(&tasks[4].plan_id, &spec).unwrap();
    assert!(
        dependencies
            .iter()
            .any(|task| task.spec["kind"] == "options")
    );
    for writer in &tasks[..4] {
        assert!(dependencies.iter().any(|task| task.id == writer.id));
    }
    let base = core.writer_base(&tasks[4].plan_id, &spec, root).unwrap();
    let output = std::process::Command::new("git")
        .args(["show", &format!("{base}:config.txt")])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"revision 1\n");
    for (file, expected) in [
        ("ui.txt", "revision 3\n"),
        ("user.txt", "local edit\n"),
        ("draft.txt", "untracked\n"),
    ] {
        let output = std::process::Command::new("git")
            .args(["show", &format!("{base}:{file}")])
            .current_dir(root)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, expected.as_bytes());
    }
    assert_eq!(git::head(root).unwrap(), head);
    assert_eq!(
        std::fs::read_to_string(root.join("user.txt")).unwrap(),
        "local edit\n"
    );
    assert!(!root.join("config.txt").exists());
    let scratch = writer_base_scratch(&root.join(".antislop"), &tasks[4].plan_id, "t5");
    assert!(!scratch.exists());
    assert!(!scratch.with_extension("patch").exists());

    // The baseline is fixed for the plan: an edit the user makes afterwards
    // does not change it, and clashes with the results only on integration,
    // which protects it. Every task of the plan starts from one commit.
    std::fs::write(root.join("config.txt"), "user conflict\n").unwrap();
    let again = core.writer_base(&tasks[4].plan_id, &spec, root).unwrap();
    assert_eq!(
        git::tree_identity(root, &again).unwrap(),
        git::tree_identity(root, &base).unwrap()
    );
    assert!(!scratch.exists());
    assert_eq!(git::head(root).unwrap(), head);
}

#[test]
fn the_owner_goal_reaches_its_generated_options_task() {
    let mut writer: TaskContract = serde_json::from_value(json!({
        "id": "writer", "title": "Implement change",
        "goal": "Keep the public API source compatible",
        "kind": "implementation", "writes": true, "scope": ["src/"]
    }))
    .unwrap();
    let options_goal = |writer: &TaskContract| {
        finalize_plan(vec![writer.clone()], false, true)
            .into_iter()
            .find(|spec| spec.options_for.is_some())
            .unwrap()
            .goal
    };
    assert!(options_goal(&writer).contains("source compatible"));
    writer.goal = "Do a different thing entirely".into();
    let revised = options_goal(&writer);
    assert!(revised.contains("different thing") && !revised.contains("source compatible"));
}

#[test]
fn explicit_quality_checks_precede_the_final_task_after_finalization() {
    let submitted = json!([
        {"id":"t11", "title":"Implement", "goal":"Implement change", "kind":"implementation"},
        {"id":"t12", "title":"Document", "goal":"Document change", "kind":"writing", "writes":true, "depends_on":["t11"]},
        {"id":"quality", "title":"Simplify", "goal":"Review and simplify change", "kind":"refactoring", "depends_on":["t11","t12"]},
        {"id":"static", "title":"Lint", "goal":"Run static checks", "kind":"static_analysis", "depends_on":["quality"]},
        {"id":"tests", "title":"Test", "goal":"Run regression tests", "kind":"testing", "depends_on":["quality"]},
        {"id":"security", "title":"Audit security", "goal":"Audit changed paths", "kind":"security", "depends_on":["quality"]},
        {"id":"performance", "title":"Audit performance", "goal":"Audit changed paths", "kind":"performance", "depends_on":["quality"]},
        {"id":"integration", "title":"Integrate", "goal":"Check combined results", "kind":"integration", "depends_on":["static","tests","security","performance"]},
        {"id":"t13", "title":"Summarize", "goal":"Summarize checked results", "kind":"analysis", "depends_on":["integration"]}
    ]);
    let tasks = tasks::planned_graph(serde_json::from_value(submitted).unwrap()).unwrap();
    for required_gates in [false, true] {
        let finalized = finalize_plan(tasks.clone(), required_gates, true);
        tasks::validate(&finalized).unwrap();
        let position = |id: &str| finalized.iter().position(|task| task.id == id).unwrap();
        assert!(position("t12") < position("quality"));
        assert!(position("quality") < position("static"));
        assert!(position("static") < position("t13"));
        assert_eq!(
            finalized
                .iter()
                .filter(|task| task.kind == WorkKind::Refactoring)
                .count(),
            1
        );
        assert_eq!(
            finalized
                .iter()
                .filter(|task| task.kind == WorkKind::StaticAnalysis)
                .count(),
            1
        );
        let final_task = &finalized[position("t13")];
        assert_eq!(final_task.depends_on, ["integration"]);
        assert!(position("integration") < position("t13"));
        for kind in tasks::GATE_KINDS {
            assert_eq!(finalized.iter().filter(|task| task.kind == kind).count(), 1);
        }
        assert_eq!(
            finalized,
            finalize_plan(finalized.clone(), required_gates, true)
        );
    }
}

#[test]
fn generated_gates_can_be_prerequisites_of_authored_tasks() {
    let submitted = json!([
        {"id":"t11", "title":"Implement", "goal":"Implement change", "kind":"implementation"},
        {"id":"t12", "title":"Review", "goal":"Review change", "kind":"analysis", "depends_on":["t11"]},
        {"id":"t13", "title":"Report", "goal":"Report checked results", "kind":"analysis", "depends_on":["quality-refactoring","static-analyzer"]},
        {"id":"t14", "title":"Deliver", "goal":"Prepare delivery", "kind":"writing", "writes":true, "depends_on":["t13"]},
        {"id":"t15", "title":"Close", "goal":"Close delivery", "kind":"analysis", "depends_on":["t14","integration-gate"]}
    ]);
    let specs = tasks::planned_graph(serde_json::from_value(submitted).unwrap()).unwrap();
    let mut config = Config::from_toml(include_str!("../../examples/antislop.toml")).unwrap();
    config.policy.required_gates = true;
    let finalized = super::core_plan_proposal::checked_plan(&specs, &config).unwrap();
    tasks::validate(&finalized).unwrap();
    let task = |id: &str| finalized.iter().find(|task| task.id == id).unwrap();
    assert!(
        task("quality-refactoring")
            .depends_on
            .contains(&"t12".to_owned())
    );
    assert!(
        task("static-analyzer")
            .depends_on
            .contains(&"quality-refactoring".to_owned())
    );
    assert!(
        task("t13")
            .depends_on
            .contains(&"static-analyzer".to_owned())
    );
    for gate in finalized.iter().filter(|task| task.quality_gate) {
        assert!(
            !gate
                .depends_on
                .iter()
                .any(|id| ["t13", "t14", "t15"].contains(&id.as_str()))
        );
    }
    assert_eq!(
        finalized,
        super::core_plan_proposal::checked_plan(&finalized, &config).unwrap()
    );
    config.policy.required_gates = false;
    assert!(
        super::core_plan_proposal::checked_plan(&specs, &config)
            .unwrap_err()
            .contains("unknown task")
    );
}

#[test]
fn generated_gate_references_do_not_hide_unknown_dependencies() {
    let submitted = json!([
        {"id":"t1", "title":"Report", "goal":"Report", "kind":"analysis", "depends_on":["static-analyzer", "typo"]}
    ]);
    assert!(
        tasks::planned_graph(serde_json::from_value(submitted).unwrap())
            .unwrap_err()
            .to_string()
            .contains("typo")
    );
}

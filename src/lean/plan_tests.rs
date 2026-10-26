use super::core_plan_helpers::*;
use super::*;

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
        let specs = finalize_plan(input.clone(), required_gates);
        assert_eq!(specs, finalize_plan(input, required_gates));
        assert_eq!(specs, finalize_plan(specs.clone(), required_gates));
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

    // A genuine conflict must still let the writer start with exact inputs
    // and actionable guidance, without exposing a broken primary tree.
    std::fs::write(root.join("config.txt"), "user conflict\n").unwrap();
    let error = core
        .writer_base(&tasks[4].plan_id, &spec, root)
        .unwrap_err();
    assert!(error.to_string().contains("git apply"));
    assert!(!scratch.exists());
    assert_eq!(git::head(root).unwrap(), head);
}

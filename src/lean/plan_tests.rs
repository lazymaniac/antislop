use super::*;
use crate::lean::core::plan::helpers::*;

fn test_tempdir() -> tempfile::TempDir {
    std::fs::create_dir_all(".antislop/tmp").unwrap();
    tempfile::tempdir_in(".antislop/tmp").unwrap()
}

#[test]
fn task_role_recognizes_only_explicit_worker_and_reviewer_addresses() {
    assert_eq!(task_role("worker:task-1"), Some(("task-1", false)));
    assert_eq!(task_role("reviewer:task-1"), Some(("task-1", true)));
    for key in ["task-1", "orchestrator", "review:task-1"] {
        assert_eq!(task_role(key), None, "unexpected task role for {key:?}");
    }
}

#[test]
fn finalization_derives_options_and_gates_from_rigor() {
    let writer: TaskContract = serde_json::from_value(json!({"id":"writer","title":"Write","goal":"Deliver behavior","kind":"implementation","change_kind":"code","acceptance_criteria":[{"id":"C1","text":"Behavior works"}]})).unwrap();
    for team in [
        tasks::Team::Feature,
        tasks::Team::SmallChange,
        tasks::Team::Refactoring,
        tasks::Team::SecurityFix,
        tasks::Team::NoQualityGates,
    ] {
        let plan = finalize_team_plan(vec![writer.clone()], team);
        assert_eq!(plan.iter().any(|t| t.options_for.is_some()), team.options());
        let kinds = plan
            .iter()
            .filter(|t| t.quality_gate)
            .map(|t| t.kind)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(kinds, team.gates().iter().copied().collect());
        assert_eq!(plan, finalize_team_plan(plan.clone(), team));
    }
}

#[test]
fn code_review_selection_accepts_only_report_only_review_tasks() {
    use crate::lean::core::plan::proposal::checked_team_plan;

    let config = Config::from_toml(include_str!("../../examples/antislop.toml")).unwrap();

    let analysis: TaskContract = serde_json::from_value(json!({
        "id":"review", "title":"Review target", "goal":"Inspect target and report verified findings without edits.",
        "change_kind":"none", "kind":"analysis",
        "acceptance_criteria":[{"id":"C1","text":"The report states scope, verdict, evidence and limits"}]
    })).unwrap();
    let checked = checked_team_plan(
        std::slice::from_ref(&analysis),
        &config,
        tasks::Team::CodeReview,
    )
    .unwrap();
    assert_eq!(checked.len(), 1, "code-review adds no gates or options");
    assert_eq!(checked[0].id, "review");
    assert!(tasks::is_report_only_review_task(&checked[0]));
    assert_eq!(checked[0].change_kind(), tasks::ChangeKind::None);
    assert!(!checked[0].quality_gate && checked[0].options_for.is_none());
    let ordinary = checked_team_plan(&checked, &config, tasks::Team::Feature).unwrap();
    assert!(!tasks::is_report_only_review_task(&ordinary[0]));

    let mut forbidden = Vec::new();
    let mut code = analysis.clone();
    code.change_kind = tasks::ChangeKind::Code;
    forbidden.push(code);
    let mut artifact = analysis.clone();
    artifact.change_kind = tasks::ChangeKind::Artifact;
    artifact.outputs = vec!["review.md".into()];
    forbidden.push(artifact);
    for kind in [
        WorkKind::Implementation,
        WorkKind::Refactoring,
        WorkKind::Options,
        WorkKind::Writing,
        WorkKind::Delivery,
    ] {
        let mut task = analysis.clone();
        task.kind = kind;
        forbidden.push(task);
    }
    let mut gate = analysis.clone();
    gate.quality_gate = true;
    forbidden.push(gate);
    let mut options = analysis;
    options.options_for = Some("review".into());
    forbidden.push(options);

    for task in forbidden {
        let result = checked_team_plan(
            std::slice::from_ref(&task),
            &config,
            tasks::Team::CodeReview,
        );
        assert!(result.is_err(), "accepted forbidden task {}", task.id);
        let error = result.unwrap_err();
        assert!(error.contains(&task.id), "{error}");
    }
}

#[test]
fn no_quality_gates_preserves_requested_dependencies_without_gates_options_or_audit_task() {
    use crate::lean::core::plan::proposal::checked_team_plan;

    let config = Config::from_toml(include_str!("../../examples/antislop.toml")).unwrap();

    let parse = |value| serde_json::from_value::<TaskContract>(value).unwrap();
    let first = parse(json!({
        "id":"first", "title":"Implement parser", "goal":"Add parser behavior",
        "change_kind":"code", "kind":"implementation",
        "depends_on":[], "acceptance_criteria":[{"id":"C1","text":"Parser works"}]
    }));
    let second = parse(json!({
        "id":"second", "title":"Add parser tests", "goal":"Test parser behavior",
        "change_kind":"code", "kind":"testing", "depends_on":["first"],
        "acceptance_criteria":[{"id":"C1","text":"Tests cover parser behavior"}]
    }));

    {
        let plan = checked_team_plan(
            &[first.clone(), second.clone()],
            &config,
            tasks::Team::NoQualityGates,
        )
        .unwrap();
        assert_eq!(plan.len(), 3);
        assert_eq!(plan[1].depends_on, ["first"]);
        let finalizer = plan.last().unwrap();
        assert!(finalizer.integration_batch);
        assert!(!finalizer.quality_gate && finalizer.options_for.is_none());
        assert_eq!(finalizer.depends_on, ["first", "second"]);
        assert_eq!(finalizer.kind, WorkKind::Integration);
        assert_eq!(
            plan.iter().filter(|task| task.quality_gate).count(),
            0,
            "the selected rigor must not add a quality pipeline"
        );
        assert!(plan.iter().all(|task| task.kind != WorkKind::Options));
        assert_eq!(
            plan,
            checked_team_plan(&plan, &config, tasks::Team::NoQualityGates).unwrap(),
            "runtime-generated final integration remains stable through persistence revalidation"
        );
        tasks::validate(&plan).unwrap();
    }

    let legacy = checked_team_plan(&[first], &config, tasks::Team::CodeReview);
    assert!(
        legacy.is_err(),
        "code-review still rejects implementation work"
    );
}

#[test]
fn writer_base_through_options_includes_both_dependency_chains_and_local_user_changes() {
    let project = test_tempdir();
    let root = project.path();
    let init = std::process::Command::new("git")
        .args(["init", "--quiet", "-b", "main"])
        .current_dir(root)
        .status()
        .unwrap();
    assert!(init.success());
    std::fs::write(root.join("user.txt"), "committed\n").unwrap();
    let head = git::commit_all(root, git::CommitPurpose::TestFixture).unwrap();
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
            "id": id, "title": id, "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal": "test", "change_kind": "code", "kind": "implementation",
             "depends_on": deps, "reviewer": {"summary": "inspect diff"}})
    })
    .collect();
    let specs = finalize_plan(
        specs
            .into_iter()
            .map(|spec| serde_json::from_value(spec).unwrap())
            .collect(),
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
        assert_eq!(options.spec["change_kind"], "none");
        assert!(options.base.is_none() && options.result.is_none());
        options.status = status::APPROVED.into();
        core.store.update_task(&options).unwrap();
    }
    let mut tasks: Vec<_> = stored
        .into_iter()
        .filter(|task| {
            task.spec["change_kind"]
                .as_str()
                .is_some_and(|kind| kind != "none")
        })
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
        tasks[index].result =
            Some(git::commit_all(&path, git::CommitPurpose::TestFixture).unwrap());
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
    for (file, expected) in [("ui.txt", "revision 3\n"), ("user.txt", "committed\n")] {
        let output = std::process::Command::new("git")
            .args(["show", &format!("{base}:{file}")])
            .current_dir(root)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, expected.as_bytes());
    }
    let output = std::process::Command::new("git")
        .args(["cat-file", "-e", &format!("{base}:draft.txt")])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "uncommitted user files stay out of the session base"
    );
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
fn a_landed_result_releases_the_plan_base_for_later_tasks() {
    let project = test_tempdir();
    let root = project.path();
    let init = std::process::Command::new("git")
        .args(["init", "--quiet", "-b", "main"])
        .current_dir(root)
        .status()
        .unwrap();
    assert!(init.success());
    std::fs::write(root.join(".gitignore"), ".antislop/\n").unwrap();
    std::fs::write(root.join("a.txt"), "v0\n").unwrap();
    let head = git::commit_all(root, git::CommitPurpose::TestFixture).unwrap();
    let config = Config::from_toml(include_str!("../../examples/antislop.toml")).unwrap();
    let core = Core::open(&root.join(".antislop"), config).unwrap();
    let conversation = core.start(root).unwrap();
    let specs: Vec<_> = [("t1", vec![]), ("t2", vec!["t1"])]
        .into_iter()
        .map(|(id, deps)| {
            json!({
                "id": id, "title": id, "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal": "test", "change_kind": "code", "kind": "implementation",
                 "depends_on": deps, "reviewer": {"summary": "inspect diff"}})
        })
        .collect();
    let mut tasks = core.store.add_plan(&conversation.id, &specs).unwrap();
    let first: TaskContract = serde_json::from_value(tasks[0].spec.clone()).unwrap();
    let base = core.writer_base(&tasks[0].plan_id, &first, root).unwrap();
    assert_eq!(base, head);

    // t1 changes a.txt and lands on the session branch.
    let path = root.join(".antislop/worktrees/t1");
    git::add_worktree(root, &path, "test/t1", &base).unwrap();
    std::fs::write(path.join("a.txt"), "v1\n").unwrap();
    let result = git::commit_all(&path, git::CommitPurpose::TestFixture).unwrap();
    git::remove_worktree(root, &path, "test/t1").unwrap();
    let session = core.session_worktree(&conversation.id, root).unwrap();
    assert!(matches!(
        git::integrate(&session, &base, &result, true).unwrap(),
        git::Integration::Committed(_)
    ));
    tasks[0].base = Some(base);
    tasks[0].result = Some(result.clone());
    tasks[0].status = status::APPROVED.into();
    core.store.update_task(&tasks[0]).unwrap();
    std::fs::write(root.join("a.txt"), "v2\n").unwrap();
    let moved = git::head(&session).unwrap();
    core.record_plan_landing(&tasks[0].plan_id, &tasks[0].id, &result)
        .unwrap();

    let second: TaskContract = serde_json::from_value(tasks[1].spec.clone()).unwrap();
    let base = core.writer_base(&tasks[1].plan_id, &second, root).unwrap();
    assert_eq!(
        base, moved,
        "starts from the session branch holding the landed result"
    );
    // A later edit on the user's checked-out branch does not move the session line.
    std::fs::write(root.join("a.txt"), "user edit\n").unwrap();
    assert_eq!(
        core.writer_base(&tasks[1].plan_id, &second, root).unwrap(),
        moved
    );
}

#[test]
fn the_owner_goal_reaches_its_generated_options_task() {
    let mut writer: TaskContract = serde_json::from_value(json!({
        "id": "writer", "title": "Implement change",
        "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal": "Keep the public API source compatible",
        "change_kind": "code", "kind": "implementation", }))
    .unwrap();
    let options_goal = |writer: &TaskContract| {
        finalize_plan(vec![writer.clone()])
            .into_iter()
            .find(|spec| spec.options_for.is_some())
            .unwrap()
            .goal
    };
    assert!(!options_goal(&writer).contains("source compatible"));
    assert!(options_goal(&writer).contains("read_task_contract"));
    writer.goal = "Do a different thing entirely".into();
    let revised = finalize_plan(vec![writer.clone()]);
    let owner = revised.iter().find(|task| task.id == writer.id).unwrap();
    assert_eq!(owner.goal, writer.goal);
    assert!(!options_goal(&writer).contains("different thing"));
}

#[test]
fn explicit_quality_checks_precede_the_final_task_after_finalization() {
    let submitted = json!([
        {"id":"t11", "title":"Implement", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Implement change", "change_kind": "code", "kind":"implementation"},
        {"id":"t12", "title":"Document", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Document change", "change_kind": "code", "kind":"writing",  "depends_on":["t11"]},
        {"id":"quality", "title":"Simplify", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Review and simplify change", "change_kind": "code", "kind":"refactoring", "depends_on":["t11","t12"]},
        {"id":"static", "title":"Lint", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Run static checks", "change_kind": "code", "kind":"static_analysis", "depends_on":["quality"]},
        {"id":"tests", "title":"Test", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Run regression tests", "change_kind": "code", "kind":"testing", "depends_on":["quality"]},
        {"id":"security", "title":"Audit security", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Audit changed paths", "change_kind": "none", "kind":"security", "depends_on":["quality"]},
        {"id":"performance", "title":"Audit performance", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Audit changed paths", "change_kind": "none", "kind":"performance", "depends_on":["quality"]},
        {"id":"integration", "title":"Integrate", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Check combined results", "change_kind": "code", "kind":"integration", "depends_on":["static","tests","security","performance"]},
        {"id":"t13", "title":"Summarize", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Summarize checked results", "change_kind": "none", "kind":"analysis", "depends_on":["integration"]}
    ]);
    let tasks = tasks::planned_graph(serde_json::from_value(submitted).unwrap()).unwrap();
    {
        let finalized = tasks::with_options(tasks::with_required_agents(tasks.clone()));
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
            tasks::with_options(tasks::with_required_agents(finalized.clone()))
        );
    }
}

#[test]
fn small_change_rigor_adds_only_static_analysis_and_testing_gates() {
    assert!(
        tasks::Team::SmallChange
            .summary()
            .contains("options: before each eligible code writer")
    );
    let writer: TaskContract = serde_json::from_value(json!({
        "id": "writer",
        "title": "Small fix",
        "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal": "Make a contained code change.",
        "change_kind": "code",
        "kind": "implementation"
    }))
    .unwrap();

    let finalized = finalize_team_plan(vec![writer.clone()], tasks::Team::SmallChange);
    tasks::validate(&finalized).unwrap();

    let gates: Vec<_> = finalized
        .iter()
        .filter(|task| task.quality_gate)
        .map(|task| task.kind)
        .collect();
    assert_eq!(gates, [WorkKind::StaticAnalysis, WorkKind::Testing]);
    for writer in finalized
        .iter()
        .filter(|task| task.changes_code() && !task.quality_gate)
    {
        let options = finalized
            .iter()
            .find(|task| task.options_for.as_deref() == Some(writer.id.as_str()))
            .unwrap_or_else(|| panic!("missing options task for {}", writer.id));
        assert_eq!(options.kind, WorkKind::Options);
        assert!(
            options
                .reviewer
                .summary
                .as_deref()
                .is_some_and(|summary| summary.contains("Review the selection"))
        );
        assert!(writer.depends_on.contains(&options.id));
        assert!(
            finalized.iter().position(|task| task.id == options.id)
                < finalized.iter().position(|task| task.id == writer.id)
        );
        assert!(
            options
                .depends_on
                .iter()
                .all(|dependency| writer.depends_on.contains(dependency))
        );
    }
    for gate in finalized.iter().filter(|task| task.quality_gate) {
        assert!(
            gate.depends_on
                .iter()
                .any(|dependency| dependency == "writer")
        );
        assert!(
            finalized
                .iter()
                .all(|task| task.options_for.as_deref() != Some(gate.id.as_str())),
            "quality gate {} must not have an options task",
            gate.id
        );
        assert!(
            gate.depends_on.iter().all(|dependency| {
                finalized
                    .iter()
                    .find(|task| task.id == *dependency)
                    .is_none_or(|task| task.options_for.is_none())
            }),
            "quality gate {} must not depend on an options task",
            gate.id
        );
    }

    let options_disabled = finalize_team_plan(vec![writer.clone()], tasks::Team::SecurityFix);
    tasks::validate(&options_disabled).unwrap();
    assert!(
        options_disabled
            .iter()
            .all(|task| task.kind != WorkKind::Options)
    );
    assert!(
        options_disabled
            .iter()
            .all(|task| task.options_for.is_none())
    );
    assert!(options_disabled.iter().all(|task| {
        task.depends_on.iter().all(|dependency| {
            options_disabled
                .iter()
                .any(|candidate| &candidate.id == dependency)
        })
    }));

    let gates_disabled = finalize_team_plan(vec![writer], tasks::Team::NoQualityGates);
    assert!(gates_disabled.iter().all(|task| !task.quality_gate));
    assert!(
        gates_disabled
            .iter()
            .all(|task| task.kind != WorkKind::Options)
    );
}

#[test]
fn small_change_adds_no_options_for_tasks_without_code_changes() {
    let artifact: TaskContract = serde_json::from_value(json!({
        "id": "document",
        "title": "Update guide",
        "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal": "Clarify the setup guide.",
        "change_kind": "none",
        "kind": "writing"
    }))
    .unwrap();

    let finalized = finalize_team_plan(vec![artifact], tasks::Team::SmallChange);
    tasks::validate(&finalized).unwrap();
    assert!(finalized.iter().all(|task| task.kind != WorkKind::Options));
    assert!(finalized.iter().all(|task| !task.quality_gate));
}

#[test]
fn small_change_revision_rebuilds_feature_gates_from_the_selected_team() {
    let writer: TaskContract = serde_json::from_value(json!({
        "id": "writer",
        "title": "Small fix",
        "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal": "Make a contained code change.",
        "change_kind": "code",
        "kind": "implementation"
    }))
    .unwrap();

    // Revisions finalize the merged baseline again. A baseline may already
    // contain generated feature gates, which must be replaced by this team's.
    let baseline = finalize_team_plan(vec![writer], tasks::Team::Feature);
    let revised = finalize_team_plan(baseline, tasks::Team::SmallChange);
    tasks::validate(&revised).unwrap();

    let gates: Vec<_> = revised
        .iter()
        .filter(|task| task.quality_gate)
        .map(|task| task.kind)
        .collect();
    assert_eq!(gates, [WorkKind::StaticAnalysis, WorkKind::Testing]);
    assert!(
        revised
            .iter()
            .any(|task| task.options_for.as_deref() == Some("writer"))
    );
}

#[test]
fn small_change_report_checkpoint_preserves_producer_and_check_order() {
    let authored: Vec<TaskContract> = serde_json::from_value(json!([
        {"id":"writer","title":"Fix","goal":"Preserve drafts","kind":"implementation","change_kind":"code","acceptance_criteria":[{"id":"C1","text":"Failed sends preserve drafts"}]},
        {"id":"report","title":"Report","goal":"Report delivered changes and validation","kind":"analysis","change_kind":"none","depends_on":["integration-gate"],"acceptance_criteria":[{"id":"C1","text":"Report the combined result"}]}
    ])).unwrap();
    for initial_team in [tasks::Team::Feature, tasks::Team::SmallChange] {
        let baseline = finalize_team_plan(authored.clone(), initial_team);
        let plan = finalize_team_plan(baseline, tasks::Team::SmallChange);
        tasks::validate(&plan).unwrap();
        let report = plan.iter().find(|task| task.id == "report").unwrap();
        assert_eq!(report.depends_on, ["static-analyzer", "review-testing"]);
        for dependency in &report.depends_on {
            let gate = plan.iter().find(|task| task.id == *dependency).unwrap();
            assert!(gate.depends_on.contains(&"writer".to_owned()));
        }
        assert_eq!(
            finalize_team_plan(plan.clone(), tasks::Team::SmallChange),
            plan
        );
    }
}

#[test]
fn a_retargeted_checkpoint_uses_reserved_gate_ids_before_a_code_consumer() {
    let authored: Vec<TaskContract> = serde_json::from_value(json!([
        {"id":"static-analyzer","title":"Context","goal":"Inspect inputs","kind":"context","change_kind":"none","acceptance_criteria":[{"id":"C1","text":"Relevant context is reported"}]},
        {"id":"review-testing","title":"Context","goal":"Inspect outputs","kind":"context","change_kind":"none","acceptance_criteria":[{"id":"C1","text":"Relevant context is reported"}]},
        {"id":"writer","title":"Producer","goal":"Implement storage","kind":"implementation","change_kind":"code","acceptance_criteria":[{"id":"C1","text":"Storage behavior is delivered"}]},
        {"id":"consumer","title":"Consumer","goal":"Use validated storage","kind":"implementation","change_kind":"code","depends_on":["integration-gate"],"acceptance_criteria":[{"id":"C1","text":"Use the checked storage interface"}]}
    ])).unwrap();
    let plan = finalize_team_plan(authored, tasks::Team::SmallChange);
    tasks::validate(&plan).unwrap();
    let consumer = plan.iter().find(|task| task.id == "consumer").unwrap();
    assert_eq!(
        consumer.depends_on,
        ["static-analyzer-2", "review-testing-2", "consumer-options"]
    );
    for id in consumer
        .depends_on
        .iter()
        .filter(|id| id.as_str() != "consumer-options")
    {
        let gate = plan.iter().find(|task| task.id == *id).unwrap();
        assert!(gate.quality_gate);
        assert!(gate.depends_on.contains(&"writer".to_owned()));
        assert!(!gate.depends_on.contains(&"consumer".to_owned()));
    }
}

#[test]
fn generated_gates_can_be_prerequisites_of_authored_tasks() {
    let submitted = json!([
        {"id":"t11", "title":"Implement", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Implement change", "change_kind": "code", "kind":"implementation"},
        {"id":"t12", "title":"Review", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Review change", "change_kind": "none", "kind":"analysis", "depends_on":["t11"]},
        {"id":"t13", "title":"Report", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Report checked results", "change_kind": "none", "kind":"analysis", "depends_on":["quality-refactoring","static-analyzer"]},
        {"id":"t14", "title":"Deliver", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Prepare delivery", "change_kind": "code", "kind":"writing",  "depends_on":["t13"]},
        {"id":"t15", "title":"Close", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Close delivery", "change_kind": "none", "kind":"analysis", "depends_on":["t14","integration-gate"]}
    ]);
    let specs = tasks::planned_graph(serde_json::from_value(submitted).unwrap()).unwrap();

    let finalized = tasks::with_options(tasks::with_required_agents(specs));
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
        tasks::with_options(tasks::with_required_agents(finalized.clone()))
    );
}

#[test]
fn generated_gate_references_do_not_hide_unknown_dependencies() {
    let submitted = json!([
        {"id":"t1", "title":"Report", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Report", "change_kind": "none", "kind":"analysis", "depends_on":["static-analyzer", "typo"]}
    ]);
    assert!(
        tasks::planned_graph(serde_json::from_value(submitted).unwrap())
            .unwrap_err()
            .to_string()
            .contains("typo")
    );
}

#[test]
fn canonical_gate_references_resolve_to_all_declared_checks_of_that_kind() {
    let submitted = json!([
        {"id":"writer", "title":"Implement", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Implement change", "change_kind": "code", "kind":"implementation"},
        {"id":"tests-a", "title":"Test A", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Run tests", "change_kind": "code", "kind":"testing", "depends_on":["writer"]},
        {"id":"tests-b", "title":"Test B", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Run tests", "change_kind": "code", "kind":"testing", "depends_on":["writer"]},
        {"id":"whole-change-validation", "title":"Validate", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Validate whole change", "change_kind": "code", "kind":"integration", "depends_on":["review-testing"]},
        {"id":"final-report", "title":"Report", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Report checks", "change_kind": "none", "kind":"analysis", "depends_on":["integration-gate"]}
    ]);
    let specs = tasks::planned_graph(serde_json::from_value(submitted).unwrap()).unwrap();

    let final_plan = tasks::with_options(tasks::with_required_agents(specs));
    tasks::validate(&final_plan).unwrap();
    let task = |id: &str| final_plan.iter().find(|task| task.id == id).unwrap();
    assert!(
        task("whole-change-validation")
            .depends_on
            .contains(&"tests-a".to_owned())
    );
    assert!(
        task("whole-change-validation")
            .depends_on
            .contains(&"tests-b".to_owned())
    );
    assert_eq!(task("final-report").depends_on, ["whole-change-validation"]);
    assert!(
        task("whole-change-validation-options")
            .depends_on
            .contains(&"tests-b".to_owned())
    );
    assert!(
        !final_plan
            .iter()
            .any(|task| task.id == "integration-gate" || task.id == "review-testing")
    );
    assert_eq!(
        final_plan,
        tasks::with_options(tasks::with_required_agents(final_plan.clone()))
    );
}

#[test]
fn a_gate_alias_cannot_hide_a_self_dependency() {
    let submitted = json!([
        {"id":"tests", "title":"Test", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Run tests", "change_kind": "code", "kind":"testing", "depends_on":["review-testing"]}
    ]);
    let authored = tasks::planned_graph(serde_json::from_value(submitted).unwrap()).unwrap();
    let error = tasks::order_graph(tasks::with_required_agents(authored)).unwrap_err();
    assert!(error.to_string().contains("cycle"), "{error}");
}

#[test]
fn deferred_future_work_keeps_its_options_and_gates_held_across_finalization() {
    let planned: tasks::PlannedTask = serde_json::from_value(json!({"id":"future","title":"Future","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Preserve code work","change_kind": "code", "kind":"implementation","deferred":true})).unwrap();
    let writer = planned.into_contract();
    assert!(writer.deferred);
    let diagnostic: TaskContract = serde_json::from_value(json!({"id":"diagnose","title":"Diagnose","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Read retained results","change_kind": "none", "kind":"analysis"})).unwrap();
    assert!(!diagnostic.deferred);
    assert!(
        serde_json::to_value(&diagnostic)
            .unwrap()
            .get("deferred")
            .is_none(),
        "legacy contracts retain their serialized form"
    );
    let graph = finalize_plan(vec![writer, diagnostic]);
    tasks::validate(&graph).unwrap();
    for task in &graph {
        if task.id == "diagnose" {
            assert!(!task.deferred);
        } else {
            assert!(task.deferred, "{} must stay held", task.id);
        }
    }
    assert_eq!(finalize_plan(graph.clone()), graph);
    let restored: Vec<TaskContract> =
        serde_json::from_value(serde_json::to_value(&graph).unwrap()).unwrap();
    assert_eq!(restored, graph, "holds survive persistence and restart");
}

#[test]
fn a_team_runs_only_its_own_gates_and_options() {
    let writer: TaskContract = serde_json::from_value(json!({
        "id": "writer", "title": "Fix typo", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal": "Correct the message",
        "change_kind": "code", "kind": "implementation",
        "reviewer": {"summary": "Check change"}}))
    .unwrap();
    let feature = finalize_team_plan(vec![writer.clone()], tasks::Team::Feature);
    assert_eq!(feature, finalize_plan(vec![writer.clone()]));
    let small = finalize_team_plan(vec![writer.clone()], tasks::Team::SmallChange);
    let gates: Vec<_> = small
        .iter()
        .filter(|spec| spec.quality_gate)
        .map(|spec| spec.kind)
        .collect();
    assert_eq!(gates, [WorkKind::StaticAnalysis, WorkKind::Testing]);
    let options: Vec<_> = small
        .iter()
        .filter(|spec| spec.kind == WorkKind::Options)
        .collect();
    let code_writers: Vec<_> = small
        .iter()
        .filter(|spec| spec.options_for.is_none() && spec.changes_code() && !spec.quality_gate)
        .collect();
    assert_eq!(options.len(), code_writers.len());
    assert!(options.iter().all(|option| {
        option.options_for.as_ref().is_some_and(|owner| {
            code_writers
                .iter()
                .any(|writer| writer.id == *owner && writer.depends_on.contains(&option.id))
        })
    }));
    for gate in small.iter().filter(|spec| spec.quality_gate) {
        assert!(
            options
                .iter()
                .all(|option| option.options_for.as_deref() != Some(gate.id.as_str())),
            "quality gate {} must not have an options task",
            gate.id
        );
        assert!(
            gate.depends_on
                .iter()
                .all(|dependency| { options.iter().all(|option| option.id != *dependency) }),
            "quality gate {} must not depend on an options task",
            gate.id
        );
    }
    tasks::validate(&small).unwrap();
    assert_eq!(
        small,
        finalize_team_plan(small.clone(), tasks::Team::SmallChange)
    );
    // Switching back to feature restores the full gate pipeline.
    assert_eq!(finalize_team_plan(small, tasks::Team::Feature), feature);

    let misfit = tasks::Team::Research
        .misfit(std::slice::from_ref(&writer))
        .unwrap();
    assert!(misfit.contains("`code-review`"));
    assert!(tasks::Team::SmallChange.misfit(&[writer]).is_none());
    assert_eq!(
        tasks::Team::parse("security-fix").unwrap(),
        tasks::Team::SecurityFix
    );
    assert!(tasks::Team::parse("everything").is_err());
}

#[test]
fn code_review_team_round_trips_and_adds_no_code_gates_or_options() {
    let team = tasks::Team::CodeReview;
    assert_eq!(team.name(), "code-review");
    assert_eq!(tasks::Team::parse("code-review").unwrap(), team);
    let encoded = serde_json::to_string(&team).unwrap();
    assert_eq!(encoded, "\"code-review\"");
    assert_eq!(serde_json::from_str::<tasks::Team>(&encoded).unwrap(), team);
    assert_eq!(team.gates(), &[]);
    assert!(!team.options());

    let writer: TaskContract = serde_json::from_value(json!({
        "id": "writer", "title": "Implement", "goal": "Make the requested code change",
        "change_kind": "code", "kind": "implementation",
        "acceptance_criteria": [{"id": "C1", "text": "The change is implemented"}]
    }))
    .unwrap();
    let plan = finalize_team_plan(vec![writer.clone()], team);
    assert_eq!(plan, vec![writer]);
    assert!(
        plan.iter()
            .all(|task| !task.quality_gate && task.kind != WorkKind::Options)
    );
    assert!(team.misfit(&plan).is_none());
    assert!(tasks::Team::Research.misfit(&plan).is_some());
    assert!(tasks::Team::Document.misfit(&plan).is_some());

    let catalog = tasks::team_catalog();
    assert!(catalog.contains("`code-review`: reviewing code changes without generated gates; gates: no gates; options: no"));
    for (team, gates) in [
        (
            tasks::Team::SmallChange,
            vec![WorkKind::StaticAnalysis, WorkKind::Testing],
        ),
        (tasks::Team::Feature, tasks::GATE_KINDS.to_vec()),
        (
            tasks::Team::Refactoring,
            vec![
                WorkKind::Refactoring,
                WorkKind::StaticAnalysis,
                WorkKind::Testing,
                WorkKind::Integration,
            ],
        ),
        (
            tasks::Team::SecurityFix,
            vec![
                WorkKind::StaticAnalysis,
                WorkKind::Testing,
                WorkKind::Security,
                WorkKind::Integration,
            ],
        ),
    ] {
        assert_eq!(team.gates(), gates, "{} gates changed", team.name());
    }
}

#[test]
fn context_scout_is_not_predefined_or_required_and_legacy_context_tasks_round_trip() {
    assert!(
        tasks::PREDEFINED_AGENT_POOL
            .iter()
            .all(|agent| agent.id != "context-scout" && agent.kind != WorkKind::Context)
    );
    assert!(!tasks::team_catalog().contains("context scout"));
    for instructions in [
        tasks::planner_instructions(),
        tasks::orchestrator_instructions(),
    ] {
        assert!(!instructions.contains("context-scout"));
        assert!(!instructions.contains("context scout"));
    }

    let writer: TaskContract = serde_json::from_value(json!({
        "id": "writer", "title": "Implement", "goal": "Make the requested change",
        "change_kind": "code", "kind": "implementation",
        "acceptance_criteria":[{"id":"C1","text":"The requested change is implemented"}]
    }))
    .unwrap();
    for team in tasks::Team::ALL {
        let plan = finalize_team_plan(vec![writer.clone()], team);
        tasks::validate(&plan).unwrap();
        assert!(plan.iter().all(|task| task.kind != WorkKind::Context));
        let gates: Vec<_> = plan
            .iter()
            .filter(|task| task.quality_gate)
            .map(|task| task.kind)
            .collect();
        assert_eq!(gates, team.gates());
    }

    let historical = json!({
        "id": "context", "title": "Context", "goal": "Read existing contracts",
        "change_kind": "none", "kind": "context"
    });
    let restored: TaskContract = serde_json::from_value(historical.clone()).unwrap();
    assert_eq!(restored.kind, WorkKind::Context);
    let serialized = serde_json::to_value(&restored).unwrap();
    assert_eq!(serialized["kind"], historical["kind"]);
    assert_eq!(
        serde_json::from_value::<TaskContract>(serialized).unwrap(),
        restored
    );
}

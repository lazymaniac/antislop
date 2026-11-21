use super::*;
use crate::lean::core::task::execution::task_execution::worker::writer_worktree;

struct Fixture {
    _directory: tempfile::TempDir,
    project: PathBuf,
    core: Arc<Core>,
    conversation: String,
    tasks: Vec<Task>,
    specs: Vec<TaskContract>,
    head: String,
    index: Vec<u8>,
}

impl Fixture {
    fn approve_task_with_completion(
        core: &Core,
        conversation: &str,
        task: &mut Task,
        summary: &str,
        report: &str,
    ) {
        task.round = 1;
        task.status = status::WORKING.into();
        core.store.update_task(task).unwrap();
        let turn = core
            .store
            .start_turn(
                conversation,
                &format!("worker:{}", task.id),
                "artifact test fixture",
                0,
                Some(&task.plan_id),
            )
            .unwrap();
        core.store
            .save_turn_result(
                &turn.id,
                &turn_result::TurnResult::Completed {
                    summary: summary.into(),
                    report: report.into(),
                    commit_message: "docs: complete fixture task".into(),
                    selection: None,
                },
            )
            .unwrap();
        task.status = status::APPROVED.into();
        core.store.update_task(task).unwrap();
    }
}

fn fixture(output: &str, without_receipt: bool) -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    std::fs::create_dir(&project).unwrap();
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q", "-b", "main"])
            .current_dir(&project)
            .status()
            .unwrap()
            .success()
    );
    std::fs::write(project.join("source"), "user source").unwrap();
    std::fs::write(project.join("user.md"), "user document").unwrap();
    std::fs::write(project.join(".gitignore"), ".antislop/\n").unwrap();
    let head = git::commit_all(&project, git::CommitPurpose::TestFixture).unwrap();
    let config = Config::from_toml(include_str!("../../../../../examples/antislop.toml")).unwrap();
    let core = Core::open(&directory.path().join("state"), config).unwrap();
    let conversation = core.start(&project).unwrap().id;
    let values = [
        json!({"id":"report","title":"Report","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Write report","kind":"analysis","change_kind":"artifact","delivery":"untracked","outputs":[output]}),
        json!({"id":"correction","title":"Correction","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Correct report","kind":"analysis","change_kind":"artifact","delivery":"untracked","outputs":[output],"depends_on":["report"]}),
        json!({"id":"other-correction","title":"Correction","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Correct report","kind":"analysis","change_kind":"artifact","delivery":"untracked","outputs":[output],"depends_on":["report"]}),
    ];
    let specs = values
        .iter()
        .cloned()
        .map(serde_json::from_value)
        .collect::<std::result::Result<Vec<TaskContract>, _>>()
        .unwrap();
    let encoded = specs
        .iter()
        .map(|spec| serde_json::to_value(spec).unwrap())
        .collect::<Vec<_>>();
    let mut tasks = core.store.add_plan(&conversation, &encoded).unwrap();
    let worktree = writer_worktree(&core, &mut tasks[0], &specs[0], &project)
        .unwrap()
        .unwrap();
    std::fs::create_dir_all(worktree.join(output).parent().unwrap()).unwrap();
    std::fs::write(worktree.join(output), "old report").unwrap();
    let change = git::commit_artifacts(
        &worktree,
        &specs[0].outputs,
        git::CommitPurpose::TestFixture,
        true,
    )
    .unwrap();
    tasks[0].result = Some(change.clone());
    core.store.update_task(&tasks[0]).unwrap();
    let base = tasks[0].base.clone().unwrap();
    if without_receipt {
        assert_eq!(
            git::deliver_untracked(&project, &base, &change).unwrap(),
            git::Integration::Applied
        );
    } else {
        assert_eq!(
            core.deliver_artifact_change(
                &tasks[0],
                &specs[0],
                &project,
                &base,
                &change,
                &AtomicBool::new(false)
            )
            .unwrap(),
            git::Integration::Applied
        );
    }
    core.record_plan_landing(&tasks[0].plan_id, &tasks[0].id, &change)
        .unwrap();
    tasks[0].status = status::APPROVED.into();
    tasks[0].report = Some("full report".into());
    core.store.update_task(&tasks[0]).unwrap();
    let index = std::fs::read(project.join(".git/index")).unwrap();
    Fixture {
        _directory: directory,
        project,
        core,
        conversation,
        tasks,
        specs,
        head,
        index,
    }
}

fn correction(f: &mut Fixture, n: usize, text: &str) -> PathBuf {
    let path = writer_worktree(&f.core, &mut f.tasks[n], &f.specs[n], &f.project)
        .unwrap()
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(path.join(&f.specs[n].outputs[0])).unwrap(),
        "old report"
    );
    std::fs::write(path.join(&f.specs[n].outputs[0]), text).unwrap();
    f.tasks[n].result = Some(
        git::commit_artifacts(
            &path,
            &f.specs[n].outputs,
            git::CommitPurpose::TestFixture,
            true,
        )
        .unwrap(),
    );
    f.core.store.update_task(&f.tasks[n]).unwrap();
    f.core
        .validate_artifact_delta(&f.tasks[n], &f.specs[n], &path)
        .unwrap();
    path
}

fn deliver(f: &Fixture, n: usize) -> git::Integration {
    f.core
        .deliver_artifact_change(
            &f.tasks[n],
            &f.specs[n],
            &f.project,
            f.tasks[n].base.as_deref().unwrap(),
            f.tasks[n].result.as_deref().unwrap(),
            &AtomicBool::new(false),
        )
        .unwrap()
}

fn checkpoint_test_root() -> (Option<tempfile::TempDir>, PathBuf) {
    if let Some(root) = std::env::var_os("ANTISLOP_CHECKPOINT_TEST_ROOT") {
        (None, PathBuf::from(root))
    } else {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_path_buf();
        (Some(directory), root)
    }
}

fn checkpoint_project(root: &Path) -> (PathBuf, Arc<Core>, String, String, Config) {
    std::fs::create_dir_all(root).unwrap();
    let project = root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q", "-b", "main"])
            .current_dir(&project)
            .status()
            .unwrap()
            .success()
    );
    std::fs::write(project.join(".gitignore"), ".antislop/\n").unwrap();
    std::fs::write(project.join("source.txt"), "base source").unwrap();
    let initial = git::commit_all(&project, git::CommitPurpose::TestFixture).unwrap();
    let config = Config::from_toml(include_str!("../../../../../examples/antislop.toml")).unwrap();
    let core = Core::open(&root.join("state"), config.clone()).unwrap();
    let conversation = core.start(&project).unwrap().id;
    (project, core, conversation, initial, config)
}

#[test]
fn stale_report_checkpoint_replays_over_approved_dependency_after_restart() {
    let (_temporary, root) = checkpoint_test_root();
    let (project, core, conversation, initial, config) = checkpoint_project(&root);
    let state = root.join("state");
    let producer: TaskContract = serde_json::from_value(json!({
        "id":"adapter", "title":"Add approved capture adapter documentation",
        "acceptance_criteria":[{"id":"C1","text":"The adapter document is present"}],
        "goal":"Add the approved adapter document", "kind":"implementation", "change_kind":"code"
    }))
    .unwrap();
    let report: TaskContract = serde_json::from_value(json!({
        "id":"readiness", "title":"Refresh readiness report",
        "acceptance_criteria":[{"id":"C1","text":"The report is delivered"}],
        "goal":"Refresh readiness evidence", "kind":"analysis", "change_kind":"artifact",
        "delivery":"untracked", "outputs":["reports/tui-redesign/readiness.md","artifacts/tui-redesign/reference-renders.json",".antislop/readiness.md"],
        "depends_on":["adapter"]
    }))
    .unwrap();
    let mut tasks = core
        .store
        .add_plan(
            &conversation,
            &[
                serde_json::to_value(&producer).unwrap(),
                serde_json::to_value(&report).unwrap(),
            ],
        )
        .unwrap();
    let producer_path = writer_worktree(&core, &mut tasks[0], &producer, &project)
        .unwrap()
        .unwrap();
    std::fs::create_dir_all(producer_path.join("docs")).unwrap();
    std::fs::write(
        producer_path.join("docs/tui-capture-adapter.md"),
        "approved adapter documentation\n",
    )
    .unwrap();
    std::fs::create_dir_all(producer_path.join("artifacts/tui-redesign")).unwrap();
    std::fs::write(
        producer_path.join("artifacts/tui-redesign/reference-renders.json"),
        "frozen producer artifact\n",
    )
    .unwrap();
    tasks[0].result =
        Some(git::commit_all(&producer_path, git::CommitPurpose::TestFixture).unwrap());
    Fixture::approve_task_with_completion(
        &core,
        &conversation,
        &mut tasks[0],
        "Added the approved capture adapter document",
        "The capture adapter documentation is present.",
    );
    let frozen_producer = tasks[0].result.clone().unwrap();
    assert_eq!(
        git::deliver_untracked(
            &project,
            tasks[0].base.as_deref().unwrap(),
            &frozen_producer,
        )
        .unwrap(),
        git::Integration::Applied
    );
    core.store
        .record_artifact_delivery(
            &conversation,
            "artifacts/tui-redesign/reference-renders.json",
            &ArtifactReceipt {
                task: tasks[0].id.clone(),
                generation: tasks[0].input_generation,
                commit: frozen_producer.clone(),
            },
        )
        .unwrap();
    let report_path = writer_worktree(&core, &mut tasks[1], &report, &project)
        .unwrap()
        .unwrap();
    for output in &report.outputs {
        if output != "artifacts/tui-redesign/reference-renders.json" {
            std::fs::create_dir_all(report_path.join(output).parent().unwrap()).unwrap();
            std::fs::write(report_path.join(output), format!("checkpoint: {output}\n")).unwrap();
        }
    }
    let checkpoint = git::commit_artifacts(
        &report_path,
        &report.outputs,
        git::CommitPurpose::TestFixture,
        true,
    )
    .unwrap();
    tasks[1].result = Some(checkpoint.clone());
    core.store.update_task(&tasks[1]).unwrap();
    core.store
        .db()
        .execute(
            "UPDATE task_inputs SET generation=2 WHERE task_id=?1",
            [&tasks[1].id],
        )
        .unwrap();
    tasks[1].input_generation = 2;
    core.store.update_task(&tasks[1]).unwrap();
    assert_eq!(
        git::tree_identity(&project, tasks[1].base.as_deref().unwrap()).unwrap(),
        git::tree_identity(&project, &frozen_producer).unwrap()
    );

    std::fs::write(
        producer_path.join("docs/tui-capture-adapter.md"),
        "approved adapter documentation revision\n",
    )
    .unwrap();
    tasks[0].result =
        Some(git::commit_all(&producer_path, git::CommitPurpose::TestFixture).unwrap());
    Fixture::approve_task_with_completion(
        &core,
        &conversation,
        &mut tasks[0],
        "Updated the approved capture adapter document",
        "The capture adapter documentation is present.",
    );
    let approved_producer = tasks[0].result.clone().unwrap();
    tasks[1].feedback = Some(json!({"continuation_review":true}));
    core.store.update_task(&tasks[1]).unwrap();
    let plan_id = tasks[1].plan_id.clone();
    drop(core);

    let core = Core::open(&state, config).unwrap();
    let mut tasks = core.store.plan(&plan_id).unwrap();
    assert_eq!(tasks[1].result.as_deref(), Some(checkpoint.as_str()));
    assert_eq!(
        git::tree_identity(&project, tasks[1].base.as_deref().unwrap()).unwrap(),
        git::tree_identity(&project, &frozen_producer).unwrap()
    );
    assert_eq!(tasks[1].input_generation, 2);
    let frozen_input = core
        .artifact_commits(&tasks[1], &report)
        .unwrap()
        .remove("artifacts/tui-redesign/reference-renders.json")
        .unwrap();
    assert_eq!(frozen_input, frozen_producer);
    let worktree = writer_worktree(&core, &mut tasks[1], &report, &project)
        .unwrap()
        .unwrap();
    let first_normalized = tasks[1].result.clone();
    let first_normalized_tree =
        git::tree_identity(&project, first_normalized.as_deref().unwrap()).unwrap();
    let retry = writer_worktree(&core, &mut tasks[1], &report, &project)
        .unwrap()
        .unwrap();
    assert_eq!(retry, worktree);
    assert_eq!(
        git::tree_identity(&project, tasks[1].result.as_deref().unwrap()).unwrap(),
        first_normalized_tree
    );
    assert_eq!(
        std::fs::read_to_string(worktree.join("docs/tui-capture-adapter.md")).unwrap(),
        "approved adapter documentation revision\n"
    );
    assert_eq!(
        std::fs::read_to_string(worktree.join("artifacts/tui-redesign/reference-renders.json"))
            .unwrap(),
        "frozen producer artifact\n"
    );
    for output in &report.outputs {
        let expected = if output == "artifacts/tui-redesign/reference-renders.json" {
            "frozen producer artifact\n".to_owned()
        } else {
            format!("checkpoint: {output}\n")
        };
        assert_eq!(
            std::fs::read_to_string(worktree.join(output)).unwrap(),
            expected
        );
    }
    let normalized_base = tasks[1].base.clone().unwrap();
    let summary = crate::lean::core::task::execution::task_execution::worker::commit_round(
        &mut tasks[1],
        &report,
        Some(&worktree),
        6,
        "Readiness report refreshed",
        "Updated readiness evidence",
        "docs: refresh readiness evidence",
    )
    .unwrap()
    .unwrap();
    assert!(summary.contains("readiness"));
    core.validate_artifact_delta(&tasks[1], &report, &worktree)
        .unwrap();
    let mut changed = git::changed_files(
        &worktree,
        &normalized_base,
        tasks[1].result.as_deref().unwrap(),
    )
    .unwrap();
    changed.sort();
    let mut outputs = report
        .outputs
        .iter()
        .filter(|path| *path != "artifacts/tui-redesign/reference-renders.json")
        .cloned()
        .collect::<Vec<_>>();
    outputs.sort();
    assert_eq!(changed, outputs);

    let saved_base = tasks[1].base.clone();
    let saved_result = tasks[1].result.clone();
    tasks[1].feedback = Some(json!({"continuation_review":true}));
    std::fs::write(worktree.join("user-edit.txt"), "preserve local user edit\n").unwrap();
    let resumed = writer_worktree(&core, &mut tasks[1], &report, &project)
        .unwrap()
        .unwrap();
    assert_eq!(resumed, worktree);
    assert_eq!(tasks[1].base, saved_base);
    assert_eq!(tasks[1].result, saved_result);
    assert_eq!(
        std::fs::read_to_string(resumed.join("user-edit.txt")).unwrap(),
        "preserve local user edit\n"
    );
    assert!(
        tasks[1].feedback.as_ref().unwrap()["checkpoint_merge"]["detail"]
            .as_str()
            .unwrap()
            .contains("local changes")
    );
    std::fs::write(worktree.join("docs/unauthorized.md"), "must be rejected\n").unwrap();
    let error = crate::lean::core::task::execution::task_execution::worker::commit_round(
        &mut tasks[1],
        &report,
        Some(&worktree),
        7,
        "Attempt an unauthorized file",
        "This change is forbidden",
        "docs: attempt unauthorized file",
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("artifact change outside outputs:"));
    std::fs::remove_file(worktree.join("docs/unauthorized.md")).unwrap();
    std::fs::remove_file(worktree.join("user-edit.txt")).unwrap();
    let valid_result = saved_result.as_deref().unwrap();
    git::ok(&worktree, &["reset", "--hard", valid_result]).unwrap();
    tasks[1].result = Some(valid_result.to_owned());
    tasks[1].feedback = Some(json!({"continuation_review":true}));
    core.store.update_task(&tasks[1]).unwrap();

    tasks[1].verdict = Some(json!({"approved":true,"findings":[]}));
    tasks[1].status = status::WORKING.into();
    core.store.update_task(&tasks[1]).unwrap();
    core.run_task(&conversation, &mut tasks[1], &AtomicBool::new(false))
        .unwrap();
    assert_eq!(tasks[1].status, status::APPROVED);
    for output in &report.outputs {
        let receipt = core
            .store
            .artifact_receipt(&conversation, output)
            .unwrap()
            .unwrap();
        if output == "artifacts/tui-redesign/reference-renders.json" {
            assert_eq!(receipt.task, tasks[0].id);
        } else {
            assert_eq!(receipt.task, tasks[1].id);
            assert_eq!(receipt.generation, tasks[1].input_generation);
        }
    }

    let evidence = format!(
        "initial={initial}\napproved_producer={approved_producer}\ncheckpoint={checkpoint}\nnormalized_base={normalized_base}\nresult={}\ngeneration={}\nruntime_resume=Core::run_task/Execution::new; artifact receipts verified\nchanged={changed:?}\nretry=ok\nforbidden={error}\n",
        tasks[1].result.as_deref().unwrap(),
        tasks[1].input_generation,
    );
    std::fs::write(root.join("continuation-evidence.txt"), evidence).unwrap();
}

#[test]
fn run_task_resumes_a_stale_report_checkpoint_after_restart() {
    let (_temporary, root) = checkpoint_test_root();
    let root = root.join("run-task-resume");
    let (project, core, conversation, _initial, config) = checkpoint_project(&root);
    let state = root.join("state");
    let producer: TaskContract = serde_json::from_value(json!({
        "id":"adapter", "title":"Approve adapter documentation",
        "acceptance_criteria":[{"id":"C1","text":"The adapter document is present"}],
        "goal":"Add adapter documentation", "kind":"implementation", "change_kind":"code"
    }))
    .unwrap();
    let report: TaskContract = serde_json::from_value(json!({
        "id":"readiness", "title":"Resume readiness report",
        "acceptance_criteria":[{"id":"C1","text":"The report is delivered"}],
        "goal":"Refresh readiness report", "kind":"analysis", "change_kind":"artifact",
        "delivery":"untracked", "outputs":["reports/readiness.md"], "depends_on":["adapter"]
    }))
    .unwrap();
    let mut tasks = core
        .store
        .add_plan(
            &conversation,
            &[
                serde_json::to_value(&producer).unwrap(),
                serde_json::to_value(&report).unwrap(),
            ],
        )
        .unwrap();
    let producer_worktree = writer_worktree(&core, &mut tasks[0], &producer, &project)
        .unwrap()
        .unwrap();
    std::fs::create_dir_all(producer_worktree.join("docs")).unwrap();
    std::fs::write(
        producer_worktree.join("docs/tui-capture-adapter.md"),
        "approved adapter documentation\n",
    )
    .unwrap();
    tasks[0].result =
        Some(git::commit_all(&producer_worktree, git::CommitPurpose::TestFixture).unwrap());
    Fixture::approve_task_with_completion(
        &core,
        &conversation,
        &mut tasks[0],
        "Added adapter documentation",
        "The adapter documentation is present.",
    );
    let old_producer = tasks[0].result.clone().unwrap();
    assert_eq!(
        git::deliver_untracked(&project, tasks[0].base.as_deref().unwrap(), &old_producer,)
            .unwrap(),
        git::Integration::Applied
    );

    let report_worktree = writer_worktree(&core, &mut tasks[1], &report, &project)
        .unwrap()
        .unwrap();
    std::fs::create_dir_all(report_worktree.join("reports")).unwrap();
    std::fs::write(
        report_worktree.join("reports/readiness.md"),
        "saved report\n",
    )
    .unwrap();
    let checkpoint = git::commit_artifacts(
        &report_worktree,
        &report.outputs,
        git::CommitPurpose::TestFixture,
        true,
    )
    .unwrap();
    tasks[1].result = Some(checkpoint.clone());
    core.store.update_task(&tasks[1]).unwrap();
    core.store
        .db()
        .execute(
            "UPDATE task_inputs SET generation=2 WHERE task_id=?1",
            [&tasks[1].id],
        )
        .unwrap();
    tasks[1].input_generation = 2;
    tasks[1].feedback = Some(json!({"continuation_review":true}));
    core.store.update_task(&tasks[1]).unwrap();
    let old_base = tasks[1].base.clone().unwrap();
    assert_eq!(
        git::tree_identity(&project, &old_base).unwrap(),
        git::tree_identity(&project, &old_producer).unwrap()
    );

    std::fs::write(
        producer_worktree.join("docs/tui-capture-adapter.md"),
        "revised approved adapter documentation\n",
    )
    .unwrap();
    tasks[0].result =
        Some(git::commit_all(&producer_worktree, git::CommitPurpose::TestFixture).unwrap());
    Fixture::approve_task_with_completion(
        &core,
        &conversation,
        &mut tasks[0],
        "Updated adapter documentation",
        "The revised adapter documentation is present.",
    );
    let approved_producer = tasks[0].result.clone().unwrap();
    tasks[1].verdict = Some(json!({"approved":true,"findings":[]}));
    tasks[1].status = status::WORKING.into();
    core.store.update_task(&tasks[1]).unwrap();
    let plan_id = tasks[1].plan_id.clone();
    drop(core);

    let core = Core::open(&state, config).unwrap();
    let mut tasks = core.store.plan(&plan_id).unwrap();
    assert_eq!(tasks[1].base.as_deref(), Some(old_base.as_str()));
    assert_eq!(tasks[1].result.as_deref(), Some(checkpoint.as_str()));
    assert_eq!(tasks[1].input_generation, 2);
    core.run_task(&conversation, &mut tasks[1], &AtomicBool::new(false))
        .unwrap();
    assert_eq!(tasks[1].status, status::APPROVED);
    assert_eq!(tasks[1].input_generation, 2);
    let result = tasks[1].result.clone().unwrap();
    let report_worktree = writer_worktree(&core, &mut tasks[1], &report, &project)
        .unwrap()
        .unwrap();
    assert_eq!(
        git::tree_identity(&project, tasks[1].base.as_deref().unwrap()).unwrap(),
        git::tree_identity(&project, &approved_producer).unwrap()
    );
    assert_eq!(
        git::changed_files(&project, tasks[1].base.as_deref().unwrap(), &result).unwrap(),
        report.outputs
    );
    assert_eq!(
        std::fs::read_to_string(report_worktree.join("docs/tui-capture-adapter.md")).unwrap(),
        "revised approved adapter documentation\n"
    );
    assert_eq!(
        std::fs::read_to_string(report_worktree.join("reports/readiness.md")).unwrap(),
        "saved report\n"
    );
    let receipt = core
        .store
        .artifact_receipt(&conversation, "reports/readiness.md")
        .unwrap()
        .unwrap();
    assert_eq!(receipt.task, tasks[1].id);
    assert_eq!(receipt.generation, 2);

    tasks[1].status = status::WORKING.into();
    core.store.update_task(&tasks[1]).unwrap();
    core.run_task(&conversation, &mut tasks[1], &AtomicBool::new(false))
        .unwrap();
    assert_eq!(tasks[1].status, status::APPROVED);
    assert_eq!(
        git::tree_identity(&project, tasks[1].result.as_deref().unwrap()).unwrap(),
        git::tree_identity(&project, &result).unwrap()
    );
    let retry_receipt = core
        .store
        .artifact_receipt(&conversation, "reports/readiness.md")
        .unwrap()
        .unwrap();
    assert_eq!(retry_receipt.task, receipt.task);
    assert_eq!(retry_receipt.generation, receipt.generation);
    assert_eq!(
        git::tree_identity(&project, &retry_receipt.commit).unwrap(),
        git::tree_identity(&project, &receipt.commit).unwrap()
    );
    std::fs::write(
        root.join("run-task-evidence.txt"),
        format!(
            "old_base={old_base}\nold_checkpoint={checkpoint}\napproved_producer={approved_producer}\nresult={result}\ngeneration={}\ncontinuation=Core::run_task after Core::open\nretry=idempotent\n",
            tasks[1].input_generation,
        ),
    )
    .unwrap();
}

#[test]
fn checkpoint_content_conflict_preserves_pair_and_local_checkout_edit() {
    let (_temporary, root) = checkpoint_test_root();
    let root = root.join("conflict");
    let (project, core, conversation, initial, _config) = checkpoint_project(&root);
    let output = "reports/shared.md";
    let producer: TaskContract = serde_json::from_value(json!({
        "id":"producer", "title":"Produce shared document",
        "acceptance_criteria":[{"id":"C1","text":"The producer document is committed"}],
        "goal":"Produce the shared document", "kind":"analysis", "change_kind":"artifact",
        "outputs":[output]
    }))
    .unwrap();
    let report: TaskContract = serde_json::from_value(json!({
        "id":"report", "title":"Continue report checkpoint",
        "acceptance_criteria":[{"id":"C1","text":"The report checkpoint is preserved"}],
        "goal":"Continue the report", "kind":"analysis", "change_kind":"artifact",
        "delivery":"untracked", "outputs":[output], "depends_on":["producer"]
    }))
    .unwrap();
    let mut tasks = core
        .store
        .add_plan(
            &conversation,
            &[
                serde_json::to_value(&producer).unwrap(),
                serde_json::to_value(&report).unwrap(),
            ],
        )
        .unwrap();
    let report_path = writer_worktree(&core, &mut tasks[1], &report, &project)
        .unwrap()
        .unwrap();
    std::fs::create_dir_all(report_path.join("reports")).unwrap();
    std::fs::write(report_path.join(output), "checkpoint version\n").unwrap();
    let checkpoint = git::commit_artifacts(
        &report_path,
        &report.outputs,
        git::CommitPurpose::TestFixture,
        true,
    )
    .unwrap();
    tasks[1].result = Some(checkpoint.clone());
    core.store.update_task(&tasks[1]).unwrap();

    let producer_path = writer_worktree(&core, &mut tasks[0], &producer, &project)
        .unwrap()
        .unwrap();
    std::fs::create_dir_all(producer_path.join("reports")).unwrap();
    std::fs::write(producer_path.join(output), "approved producer version\n").unwrap();
    let producer_result = git::commit_artifacts(
        &producer_path,
        &producer.outputs,
        git::CommitPurpose::TestFixture,
        true,
    )
    .unwrap();
    tasks[0].result = Some(producer_result.clone());
    tasks[0].status = status::APPROVED.into();
    core.store.update_task(&tasks[0]).unwrap();
    tasks[1].base = Some(producer_result.clone());
    tasks[1].feedback = Some(json!({"continuation_review":true}));
    let old_result = tasks[1].result.clone();
    let old_base = tasks[1].base.clone();
    std::fs::write(report_path.join("local.txt"), "user changed this\n").unwrap();

    let resumed = writer_worktree(&core, &mut tasks[1], &report, &project)
        .unwrap()
        .unwrap();
    assert_eq!(resumed, report_path);
    assert_eq!(tasks[1].result, old_result);
    assert_eq!(tasks[1].base, old_base);
    assert_eq!(
        std::fs::read_to_string(report_path.join(output)).unwrap(),
        "checkpoint version\n"
    );
    assert_eq!(
        std::fs::read_to_string(report_path.join("local.txt")).unwrap(),
        "user changed this\n"
    );
    assert!(
        tasks[1]
            .feedback
            .as_ref()
            .unwrap()
            .get("checkpoint_merge")
            .is_some()
    );
    assert_eq!(git::head(&report_path).unwrap(), checkpoint);
    assert_eq!(git::head(&project).unwrap(), initial);
}

#[test]
fn a_new_writer_can_update_its_delivered_dependency_without_touching_head_or_index() {
    for output in ["report.md", ".antislop/analysis.md"] {
        let mut f = fixture(output, false);
        let path = correction(&mut f, 1, "corrected report");
        assert_eq!(deliver(&f, 1), git::Integration::Applied);
        assert_eq!(
            std::fs::read_to_string(f.project.join(output)).unwrap(),
            "corrected report"
        );
        assert_eq!(f.head, git::head(&f.project).unwrap());
        assert_eq!(
            f.index,
            std::fs::read(f.project.join(".git/index")).unwrap()
        );
        assert_eq!(
            deliver(&f, 1),
            git::Integration::Applied,
            "replay must be idempotent"
        );
        let receipt = f
            .core
            .store
            .artifact_receipt(&f.conversation, output)
            .unwrap()
            .unwrap();
        assert_eq!(receipt.task, f.tasks[1].id);
        // Durable receipts/leases survive daemon reconstruction.
        let state = f.core.state.clone();
        let config = f.core.config_snapshot().as_ref().clone();
        drop(f.core);
        let reopened = Core::open(&state, config).unwrap();
        assert_eq!(
            reopened
                .store
                .artifact_receipt(&f.conversation, output)
                .unwrap(),
            Some(receipt)
        );
        assert_eq!(
            std::fs::read_to_string(path.join(output)).unwrap(),
            "corrected report"
        );
    }
}

#[test]
fn user_changes_or_a_competing_delivery_invalidate_the_frozen_artifact_version() {
    let mut f = fixture("report.md", false);
    let worktree = correction(&mut f, 1, "correction one");
    std::fs::write(f.project.join("report.md"), "USER edit").unwrap();
    assert!(matches!(deliver(&f, 1), git::Integration::UserEdits(_)));
    assert_eq!(
        std::fs::read_to_string(f.project.join("report.md")).unwrap(),
        "USER edit"
    );
    assert_eq!(
        std::fs::read_to_string(worktree.join("report.md")).unwrap(),
        "correction one"
    );
    std::fs::write(f.project.join("report.md"), "old report").unwrap();
    correction(&mut f, 2, "correction two");
    assert_eq!(deliver(&f, 1), git::Integration::Applied);
    assert!(matches!(deliver(&f, 2), git::Integration::UserEdits(_)));
    assert_eq!(
        std::fs::read_to_string(f.project.join("report.md")).unwrap(),
        "correction one"
    );
    assert_eq!(f.head, git::head(&f.project).unwrap());
    assert_eq!(
        f.index,
        std::fs::read(f.project.join(".git/index")).unwrap()
    );
}

#[test]
fn receipt_allows_same_conversation_without_an_old_edge_but_denies_foreign_conversations() {
    let f = fixture("report.md", false);
    for foreign in [false, true] {
        let conversation = if foreign {
            f.core
                .store
                .create_conversation(f.project.to_str().unwrap())
                .unwrap()
                .id
        } else {
            f.conversation.clone()
        };
        let mut spec = f.specs[1].clone();
        spec.id = "unrelated".into();
        spec.depends_on.clear();
        let mut task = f
            .core
            .store
            .add_plan(&conversation, &[serde_json::to_value(&spec).unwrap()])
            .unwrap()
            .remove(0);
        let result = writer_worktree(&f.core, &mut task, &spec, &f.project);
        assert_eq!(result.is_err(), foreign);
        assert_eq!(
            std::fs::read_to_string(f.project.join("report.md")).unwrap(),
            "old report"
        );
    }
}

#[test]
fn missing_delivery_receipt_never_adopts_existing_artifacts_or_user_files() {
    let mut f = fixture("report.md", true);
    assert!(writer_worktree(&f.core, &mut f.tasks[1], &f.specs[1], &f.project).is_err());
    f.specs[0].outputs.push("user.md".into());
    f.tasks[0].spec = serde_json::to_value(&f.specs[0]).unwrap();
    f.core.store.update_task(&f.tasks[0]).unwrap();
    f.specs[1].outputs = vec!["user.md".into()];
    f.tasks[1].spec = serde_json::to_value(&f.specs[1]).unwrap();
    f.core.store.update_task(&f.tasks[1]).unwrap();
    assert!(writer_worktree(&f.core, &mut f.tasks[1], &f.specs[1], &f.project).is_err());
    assert!(
        f.core
            .store
            .artifact_receipt(&f.conversation, "user.md")
            .unwrap()
            .is_none()
    );
    assert_eq!(
        std::fs::read_to_string(f.project.join("user.md")).unwrap(),
        "user document"
    );
}

#[test]
fn a_previously_unknown_owned_report_can_be_recovered_without_a_dependency_edge() {
    let mut f = fixture("report.md", false);
    f.specs[1].depends_on.clear();
    f.core
        .store
        .freeze_artifact_input(&f.tasks[1], "report.md", None)
        .unwrap();
    let mut task = f.tasks[1].clone();
    let path = writer_worktree(&f.core, &mut task, &f.specs[1], &f.project)
        .unwrap()
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(path.join("report.md")).unwrap(),
        "old report"
    );
    assert!(
        f.core
            .store
            .artifact_input(&task, "report.md")
            .unwrap()
            .unwrap()
            .is_some()
    );
    assert_eq!(git::head(&f.project).unwrap(), f.head);
    assert_eq!(
        std::fs::read(f.project.join(".git/index")).unwrap(),
        f.index
    );
}

#[test]
fn recovered_ownership_never_adopts_user_edits_or_symlink_targets() {
    use std::os::unix::fs::symlink;
    for link in [false, true] {
        let mut f = fixture("report.md", false);
        f.specs[1].depends_on.clear();
        f.core
            .store
            .freeze_artifact_input(&f.tasks[1], "report.md", None)
            .unwrap();
        if link {
            std::fs::remove_file(f.project.join("report.md")).unwrap();
            symlink("user.md", f.project.join("report.md")).unwrap();
        } else {
            std::fs::write(f.project.join("report.md"), "user correction").unwrap();
        }
        let mut task = f.tasks[1].clone();
        assert!(writer_worktree(&f.core, &mut task, &f.specs[1], &f.project).is_err());
        assert!(
            f.core
                .store
                .artifact_input(&task, "report.md")
                .unwrap()
                .unwrap()
                .is_none()
        );
        assert_eq!(
            std::fs::read_to_string(f.project.join("user.md")).unwrap(),
            "user document"
        );
        assert_eq!(git::head(&f.project).unwrap(), f.head);
        assert_eq!(
            std::fs::read(f.project.join(".git/index")).unwrap(),
            f.index
        );
    }
}

#[test]
fn continuation_keeps_integrated_work_and_new_upstream_changes() {
    let mut f = fixture("report.md", false);
    std::fs::write(f.project.join("source"), "new upstream content").unwrap();
    f.tasks[1].base = f.tasks[0].base.clone();
    f.tasks[1].result = f.tasks[0].result.clone();
    f.tasks[1].feedback = Some(json!({"continuation_review":true,"resume_role":"reviewer"}));
    let path = writer_worktree(&f.core, &mut f.tasks[1], &f.specs[1], &f.project)
        .unwrap()
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(path.join("source")).unwrap(),
        "user source"
    );
    assert_eq!(
        std::fs::read_to_string(f.project.join("source")).unwrap(),
        "new upstream content"
    );
    assert_eq!(
        std::fs::read_to_string(path.join("report.md")).unwrap(),
        "old report"
    );
    assert_eq!(
        std::fs::read_to_string(f.project.join("report.md")).unwrap(),
        "old report"
    );
    assert_eq!(git::head(&f.project).unwrap(), f.head);
    assert_eq!(
        std::fs::read(f.project.join(".git/index")).unwrap(),
        f.index
    );
}

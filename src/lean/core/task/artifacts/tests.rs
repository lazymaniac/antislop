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
    let checkout_lease = core.lease_worktree(&report_worktree);
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

    drop(checkout_lease);
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
    for continuation_review in [false, true] {
        let (_temporary, root) = checkpoint_test_root();
        let root = root.join(format!("conflict-{continuation_review}"));
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
        tasks[1].feedback = continuation_review.then(|| json!({"continuation_review":true}));
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

#[test]
fn repeated_artifact_preparation_uses_current_producers_without_review_feedback() {
    mod cli_fixture {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/support/fixture_cli.rs"
        ));
    }
    use crate::lean::core::task::execution::task_execution::worker::commit_round;

    let (_temporary, root) = checkpoint_test_root();
    let root = root.join("multi-stage");
    let (project, mut core, conversation, initial, mut config) = checkpoint_project(&root);
    let cli = root.join("agy");
    std::fs::write(
        &cli,
        format!(
            r#"#!/usr/bin/env python3
import json, os, sys, pathlib
prompt = sys.argv[-1]
root = pathlib.Path({root:?})
assert pathlib.Path('docs/tui-capture-adapter.md').read_text() == (root / 'expected-doc.txt').read_text()
expected = json.loads((root / 'expected-targets.json').read_text())
assert all(pathlib.Path(path).is_file() for path in expected)
role = 'reviewer' if 'Task role: reviewer' in prompt else 'worker'
wave = (root / 'expected-wave.txt').read_text()
for output in ['artifacts/tui-redesign/reference-renders.json', 'reports/tui-redesign/readiness.md']:
    content = pathlib.Path(output).read_text()
    assert content.startswith('saved author delta: ') or content.startswith('refreshed report ')
    expected_output = 'refreshed report ' + wave + ': ' + output + '\n'
    if role == 'worker':
        pathlib.Path(output).write_text(expected_output)
    else:
        assert content == expected_output
with (root / 'cli-inspections.jsonl').open('a') as log:
    log.write(json.dumps({{'role':role, 'cwd':os.getcwd(), 'targets':expected}}) + '\n')
answer = 'Inspected current inputs and preserved both report outputs.'
if role == 'reviewer':
    answer += '\n```verdict\n' + json.dumps({{'approved':True, 'findings':[]}}) + '\n```'
print(json.dumps({{'event':'result', 'result':{{'conversation_id':role, 'status':'SUCCESS', 'response':answer, 'usage':{{}}}}}}))
"#,
            root = root.to_string_lossy()
        ),
    ).unwrap();
    cli_fixture::interactive(&cli);
    for profile in config.profiles.values_mut() {
        profile.provider = crate::config::ProviderKind::Agy;
        profile.binary = None;
    }
    config
        .providers
        .get_mut(&crate::config::ProviderKind::Agy)
        .unwrap()
        .binary = cli;
    let targets = [
        "tests/lean_tui_navigation_sequence.rs",
        "tests/lean_tui_recovery_sequence.rs",
        "tests/lean_tui_message_sequences.rs",
        "tests/lean_tui_rigor_semantics.rs",
        "tests/lean_tui_plan_consent.rs",
    ];
    let mut specs: Vec<TaskContract> = (0..=targets.len())
        .map(|index| {
            serde_json::from_value(json!({
                "id":format!("producer-{index}"), "title":"Approved source input",
                "acceptance_criteria":[{"id":"C1","text":"The source input is present"}],
                "goal":"Provide current source", "kind":"implementation", "change_kind":"code"
            }))
            .unwrap()
        })
        .collect();
    let report: TaskContract = serde_json::from_value(json!({
        "id":"readiness", "title":"Continue report",
        "acceptance_criteria":[{"id":"C1","text":"The report is delivered"}],
        "goal":"Refresh report", "kind":"analysis", "change_kind":"artifact",
        "delivery":"untracked",
        "outputs":["artifacts/tui-redesign/reference-renders.json","reports/tui-redesign/readiness.md"],
        "depends_on":specs.iter().map(|spec| spec.id.clone()).collect::<Vec<_>>()
    })).unwrap();
    specs.push(report.clone());
    let mut tasks = core
        .store
        .add_plan(
            &conversation,
            &specs
                .iter()
                .map(serde_json::to_value)
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap(),
        )
        .unwrap();
    let consumer = tasks.len() - 1;
    let worktree = writer_worktree(&core, &mut tasks[consumer], &report, &project)
        .unwrap()
        .unwrap();
    for output in &report.outputs {
        std::fs::create_dir_all(worktree.join(output).parent().unwrap()).unwrap();
        std::fs::write(
            worktree.join(output),
            format!("saved author delta: {output}\n"),
        )
        .unwrap();
    }
    let checkpoint = git::commit_artifacts(
        &worktree,
        &report.outputs,
        git::CommitPurpose::TestFixture,
        true,
    )
    .unwrap();
    tasks[consumer].result = Some(checkpoint.clone());
    core.store.update_task(&tasks[consumer]).unwrap();
    let plan = tasks[consumer].plan_id.clone();
    let mut snapshots = Vec::new();
    // Approvals arrive in two waves; preparation must work without transient
    // continuation_review feedback, including after a reopened Core.
    for wave in 0..2 {
        for index in 0..6 {
            if wave > 0 {
                core.invalidate_task_dependents(&conversation, &tasks[index])
                    .unwrap();
            }
            let producer = writer_worktree(&core, &mut tasks[index], &specs[index], &project)
                .unwrap()
                .unwrap();
            let paths = if index == 0 {
                vec!["src/capture_adapter.rs", "docs/tui-capture-adapter.md"]
            } else {
                vec![targets[index - 1]]
            };
            for path in paths {
                std::fs::create_dir_all(producer.join(path).parent().unwrap()).unwrap();
                std::fs::write(
                    producer.join(path),
                    format!("approved producer {index} wave {wave}\n"),
                )
                .unwrap();
            }
            tasks[index].result =
                Some(git::commit_all(&producer, git::CommitPurpose::TestFixture).unwrap());
            Fixture::approve_task_with_completion(
                &core,
                &conversation,
                &mut tasks[index],
                "Approved input",
                "Current producer input",
            );
        }
        tasks = core.store.plan(&plan).unwrap();
        assert_eq!(
            tasks[consumer].input_generation,
            if wave == 0 { 0 } else { 6 }
        );
        if wave > 0 {
            tasks[consumer].round += 1;
            // Reproduce the actual legacy defect: current source base paired
            // with a result still parented by the older writer checkout.
            tasks[consumer].base = Some(core.writer_base(&plan, &report, &project).unwrap());
            assert!(
                report
                    .validate_change_paths(
                        &git::changed_files(
                            &project,
                            tasks[consumer].base.as_deref().unwrap(),
                            tasks[consumer].result.as_deref().unwrap()
                        )
                        .unwrap()
                    )
                    .is_err()
            );
        }
        tasks[consumer].feedback = None;
        core.store.update_task(&tasks[consumer]).unwrap();
        std::fs::write(root.join("expected-wave.txt"), wave.to_string()).unwrap();
        drop(core);
        core = Core::open(&root.join("state"), config.clone()).unwrap();
        tasks = core.store.plan(&plan).unwrap();
        std::fs::write(
            root.join("expected-doc.txt"),
            format!("approved producer 0 wave {wave}\n"),
        )
        .unwrap();
        std::fs::write(
            root.join("expected-targets.json"),
            serde_json::to_vec(&targets).unwrap(),
        )
        .unwrap();
        tasks[consumer].verdict = None;
        tasks[consumer].report = None;
        tasks[consumer].status = status::WORKING.into();
        core.store.update_task(&tasks[consumer]).unwrap();
        core.run_task(&conversation, &mut tasks[consumer], &AtomicBool::new(false))
            .unwrap();
        let checkout_lease = core.lease_worktree(&worktree);
        writer_worktree(&core, &mut tasks[consumer], &report, &project).unwrap();
        let pair = (tasks[consumer].base.clone(), tasks[consumer].result.clone());
        let persisted = core
            .store
            .task_in_plan_by_id(&plan, "readiness")
            .unwrap()
            .unwrap();
        assert_eq!((persisted.base, persisted.result), pair);
        assert_eq!(
            std::fs::read_to_string(worktree.join("docs/tui-capture-adapter.md")).unwrap(),
            format!("approved producer 0 wave {wave}\n")
        );
        for target in &targets {
            assert!(
                worktree.join(target).is_file(),
                "missing current input {target}"
            );
        }
        commit_round(
            &mut tasks[consumer],
            &report,
            Some(&worktree),
            wave + 1,
            "Report reviewed against current inputs",
            "Refreshed report",
            "docs: refresh readiness report",
        )
        .unwrap();
        assert_eq!(
            git::changed_files(
                &project,
                tasks[consumer].base.as_deref().unwrap(),
                tasks[consumer].result.as_deref().unwrap()
            )
            .unwrap(),
            report.outputs
        );
        // Inspect the saved result independently before the real CLI review.
        let review = root.join(format!("review-{wave}"));
        git::add_worktree(
            &project,
            &review,
            &format!("review-{wave}"),
            tasks[consumer].result.as_deref().unwrap(),
        )
        .unwrap();
        assert!(review.join("docs/tui-capture-adapter.md").is_file());
        assert_eq!(tasks[consumer].status, status::APPROVED);
        let result = tasks[consumer].result.clone().unwrap();
        snapshots.push((tasks[consumer].base.clone().unwrap(), result.clone()));
        for output in &report.outputs {
            let receipt = core
                .store
                .artifact_receipt(&conversation, output)
                .unwrap()
                .unwrap();
            assert_eq!(receipt.task, tasks[consumer].id);
            assert_eq!(receipt.generation, tasks[consumer].input_generation);
            assert_eq!(
                std::fs::read_to_string(project.join(output)).unwrap(),
                format!("refreshed report {wave}: {output}\n")
            );
        }
        core.store.update_task(&tasks[consumer]).unwrap();
        writer_worktree(&core, &mut tasks[consumer], &report, &project).unwrap();
        assert_eq!(
            git::tree_identity(&project, tasks[consumer].result.as_deref().unwrap()).unwrap(),
            git::tree_identity(&project, &result).unwrap()
        );
        if wave == 0 {
            // Composition may save an equivalent base on a sibling history.
            // The persisted pair, not its merge-base, defines the author delta.
            let tree =
                git::tree_identity(&project, tasks[consumer].base.as_deref().unwrap()).unwrap();
            let sibling = git::ok(
                &project,
                &[
                    "commit-tree",
                    &tree,
                    "-p",
                    &initial,
                    "-m",
                    "Equivalent composed input",
                ],
            )
            .unwrap();
            assert_ne!(
                git::common_base(&project, &sibling, &result).unwrap(),
                sibling
            );
            tasks[consumer].round += 1;
            tasks[consumer].base = Some(sibling);
            core.store.update_task(&tasks[consumer]).unwrap();
            if worktree.exists() {
                let branch = git::worktree_branch(&worktree).unwrap();
                git::remove_clean_writer(&project, &worktree, &branch, &report.outputs).unwrap();
            }
            assert!(!worktree.exists());
        }
        drop(checkout_lease);
    }
    drop(core);
    let core = Core::open(&root.join("state"), config).unwrap();
    for (base, result) in &snapshots {
        assert_eq!(
            git::changed_files(&project, base, result).unwrap(),
            report.outputs
        );
    }
    let recovery = core
        .store
        .events_since(&conversation, 0, 1000)
        .unwrap()
        .into_iter()
        .find(|event| event.kind == "artifact_checkpoint_recovery")
        .expect("durable recovery provenance after reopen");
    assert_eq!(
        git::tree_identity(&project, recovery.payload["result"].as_str().unwrap()).unwrap(),
        git::tree_identity(&project, &snapshots[0].1).unwrap()
    );
    assert_eq!(recovery.payload["input_generation"], 6);
    let inspections = std::fs::read_to_string(root.join("cli-inspections.jsonl")).unwrap();
    assert_eq!(
        inspections
            .lines()
            .filter(|line| line.contains("\"role\": \"reviewer\""))
            .count(),
        2,
        "{inspections}"
    );
    // A genuine unauthorized committed edit must survive reconciliation and
    // still be refused, rather than disappearing behind an outputs filter.
    let _checkout_lease = core.lease_worktree(&worktree);
    writer_worktree(&core, &mut tasks[consumer], &report, &project).unwrap();
    std::fs::write(worktree.join("docs/forbidden.md"), "unauthorized\n").unwrap();
    let forbidden = git::commit_all(&worktree, git::CommitPurpose::TestFixture).unwrap();
    let saved_pair = (tasks[consumer].base.clone(), tasks[consumer].result.clone());
    tasks[consumer].feedback = None;
    writer_worktree(&core, &mut tasks[consumer], &report, &project).unwrap();
    assert_eq!(
        (tasks[consumer].base.clone(), tasks[consumer].result.clone()),
        saved_pair
    );
    assert_eq!(git::head(&worktree).unwrap(), forbidden);
    assert!(
        tasks[consumer].feedback.as_ref().unwrap()["checkpoint_merge"]["detail"]
            .as_str()
            .unwrap()
            .contains("branch differs")
    );
    tasks[consumer].result = Some(forbidden);
    tasks[consumer].feedback = None;
    core.store.update_task(&tasks[consumer]).unwrap();
    let preparation_error = writer_worktree(&core, &mut tasks[consumer], &report, &project)
        .unwrap_err()
        .to_string();
    assert!(
        preparation_error.contains("artifact change outside outputs: docs/forbidden.md"),
        "{preparation_error}"
    );
    let error = commit_round(
        &mut tasks[consumer],
        &report,
        Some(&worktree),
        3,
        "Invalid report",
        "Invalid change",
        "docs: attempt forbidden change",
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("artifact change outside outputs: docs/forbidden.md"),
        "{error}"
    );
    let delivery_error = core
        .deliver_artifact_change(
            &tasks[consumer],
            &report,
            &project,
            tasks[consumer].base.as_deref().unwrap(),
            tasks[consumer].result.as_deref().unwrap(),
            &AtomicBool::new(false),
        )
        .unwrap_err()
        .to_string();
    assert!(
        delivery_error.contains("artifact change outside outputs: docs/forbidden.md"),
        "{delivery_error}"
    );
    assert_eq!(git::head(&project).unwrap(), initial);
    std::fs::write(root.join("multi-stage-evidence.txt"), format!("checkpoint={checkpoint}\npairs={snapshots:?}\nforbidden={error}\nsource_inputs=adapter,docs,five targets\nCore=reopened before each preparation and after delivery\n")).unwrap();
}

#[test]
fn inconsistent_checkpoint_recovery_requires_unambiguous_task_history() {
    for scenario in ["missing", "unrelated", "ambiguous"] {
        let (_temporary, root) = checkpoint_test_root();
        let root = root.join(format!("provenance-{scenario}"));
        let (project, core, conversation, initial, _config) = checkpoint_project(&root);
        let spec: TaskContract = serde_json::from_value(json!({
            "id":"report", "title":"Recover report",
            "acceptance_criteria":[{"id":"C1","text":"Preserve proven checkpoint"}],
            "goal":"Recover report", "kind":"analysis", "change_kind":"artifact",
            "outputs":["report.md"]
        }))
        .unwrap();
        let mut task = core
            .store
            .add_plan(&conversation, &[serde_json::to_value(&spec).unwrap()])
            .unwrap()
            .remove(0);
        let writer = writer_worktree(&core, &mut task, &spec, &project)
            .unwrap()
            .unwrap();
        std::fs::write(writer.join("report.md"), "earlier report\n").unwrap();
        let earlier = git::commit_artifacts(
            &writer,
            &spec.outputs,
            git::CommitPurpose::TestFixture,
            true,
        )
        .unwrap();
        std::fs::write(writer.join("report.md"), "current report\n").unwrap();
        let result = git::commit_artifacts(
            &writer,
            &spec.outputs,
            git::CommitPurpose::TestFixture,
            true,
        )
        .unwrap();
        if scenario == "ambiguous" {
            task.round = 1;
            task.base = Some(initial.clone());
            task.result = Some(earlier.clone());
            core.store.update_task(&task).unwrap();
            task.round = 2;
            task.base = Some(earlier);
            task.result = Some(result.clone());
            core.store.update_task(&task).unwrap();
        } else if scenario == "unrelated" {
            let other = root.join("unrelated");
            git::add_worktree(&project, &other, "unrelated", &initial).unwrap();
            std::fs::write(other.join("report.md"), "unrelated report\n").unwrap();
            task.round = 1;
            task.base = Some(initial.clone());
            task.result = Some(
                git::commit_artifacts(&other, &spec.outputs, git::CommitPurpose::TestFixture, true)
                    .unwrap(),
            );
            core.store.update_task(&task).unwrap();
        }
        std::fs::write(project.join("source.txt"), "new approved input\n").unwrap();
        let current_base = git::commit_all(&project, git::CommitPurpose::TestFixture).unwrap();
        // A new plan anchor models the legacy update of base alone.
        core.record_plan_landing(&task.plan_id, "producer", &current_base)
            .unwrap();
        task.round = 3;
        task.base = Some(current_base);
        task.result = Some(result.clone());
        task.feedback = None;
        core.store.update_task(&task).unwrap();
        let pair = (task.base.clone(), task.result.clone());
        let error = writer_worktree(&core, &mut task, &spec, &project)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(if scenario == "ambiguous" {
                "ambiguous saved artifact checkpoint bases"
            } else {
                "no proven saved artifact checkpoint base"
            }),
            "{scenario}: {error}"
        );
        assert_eq!((task.base.clone(), task.result.clone()), pair);
        let saved = core.store.task_by_id(&task.id).unwrap().unwrap();
        assert_eq!((saved.base, saved.result), pair);
        assert_eq!(git::head(&writer).unwrap(), result);
        assert_eq!(
            std::fs::read_to_string(writer.join("report.md")).unwrap(),
            "current report\n"
        );
    }
}

/// Opt-in replay of a recorded pair in a private clone. The JSON file named by
/// ANTISLOP_CHECKPOINT_REPLAY supplies repository, base, result, outputs, and
/// history [{round, base, result}]. No live runtime state is read or changed.
#[test]
#[ignore = "requires an explicit recorded-checkpoint JSON file"]
fn replay_recorded_artifact_checkpoint_through_core() {
    let input: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            std::env::var_os("ANTISLOP_CHECKPOINT_REPLAY").expect("recorded-checkpoint JSON path"),
        )
        .unwrap(),
    )
    .unwrap();
    let (_temporary, root) = checkpoint_test_root();
    let root = root.join("recorded-replay");
    std::fs::create_dir_all(&root).unwrap();
    let project = root.join("project");
    let cloned = std::process::Command::new("git")
        .args([
            "clone",
            "--shared",
            "--no-checkout",
            input["repository"].as_str().unwrap(),
        ])
        .arg(&project)
        .output()
        .unwrap();
    assert!(
        cloned.status.success(),
        "{}",
        String::from_utf8_lossy(&cloned.stderr)
    );
    let base = input["base"].as_str().unwrap();
    let result = input["result"].as_str().unwrap();
    git::ok(&project, &["reset", "--hard", base]).unwrap();
    git::ok(&project, &["branch", "-M", "main"]).unwrap();
    let config = Config::from_toml(include_str!("../../../../../examples/antislop.toml")).unwrap();
    let state = root.join("state");
    let core = Core::open(&state, config.clone()).unwrap();
    let conversation = core.start(&project).unwrap().id;
    let spec: TaskContract = serde_json::from_value(json!({
        "id":"recorded-report", "title":"Replay recorded report",
        "acceptance_criteria":[{"id":"C1","text":"Deliver only recorded report outputs"}],
        "goal":"Replay recorded checkpoint", "kind":"analysis", "change_kind":"artifact",
        "delivery":"untracked", "outputs":input["outputs"]
    }))
    .unwrap();
    let mut task = core
        .store
        .add_plan(&conversation, &[serde_json::to_value(&spec).unwrap()])
        .unwrap()
        .remove(0);
    for record in input["history"].as_array().unwrap() {
        task.round = serde_json::from_value(record["round"].clone()).unwrap();
        task.base = Some(record["base"].as_str().unwrap().into());
        task.result = Some(record["result"].as_str().unwrap().into());
        core.store.update_task(&task).unwrap();
    }
    task.round += 1;
    task.base = Some(base.into());
    task.result = Some(result.into());
    task.verdict = Some(json!({"approved":true,"findings":[]}));
    task.status = status::WORKING.into();
    core.store.update_task(&task).unwrap();
    drop(core);
    let core = Core::open(&state, config).unwrap();
    core.run_task(&conversation, &mut task, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(task.status, status::APPROVED);
    let files = git::changed_files(
        &project,
        task.base.as_deref().unwrap(),
        task.result.as_deref().unwrap(),
    )
    .unwrap();
    assert_eq!(files, spec.outputs);
    let writer = core.lease_worktree(&core.task_worktree_path(&task).unwrap().unwrap());
    let worktree = writer_worktree(&core, &mut task, &spec, &project)
        .unwrap()
        .unwrap();
    assert_eq!(
        git::patch(&project, base, task.base.as_deref()).unwrap(),
        ""
    );
    assert_eq!(
        git::patch(&worktree, base, task.base.as_deref()).unwrap(),
        ""
    );
    std::fs::write(root.join("recorded-replay-evidence.txt"), format!("recorded_base={base}\nrecorded_result={result}\nprepared_base={}\nprepared_result={}\nfiles={files:?}\nstatus={}\n", task.base.as_deref().unwrap(), task.result.as_deref().unwrap(), task.status)).unwrap();
    drop(writer);
}

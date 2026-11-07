use super::*;

fn fixture() -> (tempfile::TempDir, Arc<Core>, String, Vec<Task>, String) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    assert!(
        std::process::Command::new("git")
            .args(["init", "--quiet", "-b", "main"])
            .current_dir(&root)
            .status()
            .unwrap()
            .success()
    );
    std::fs::write(root.join(".gitignore"), ".antislop/\ncache/\n").unwrap();
    std::fs::write(root.join("initial"), "initial").unwrap();
    let commit = git::commit_all(&root, git::CommitPurpose::TestFixture).unwrap();
    let config = Config::from_toml(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/examples/antislop.toml"
    )))
    .unwrap();
    let core = Core::open(&root.join(".antislop"), config).unwrap();
    let conversation = core
        .store
        .create_conversation(root.to_str().unwrap())
        .unwrap()
        .id;
    let tasks = core
        .store
        .add_plan(
            &conversation,
            &[
                json!({"id":"analysis","title":"Analysis","goal":"Report","change_kind": "none", "kind":"analysis"}),
                json!({"id":"other","title":"Other","goal":"Other","change_kind": "none", "kind":"analysis"}),
            ],
        )
        .unwrap();
    (directory, core, conversation, tasks, commit)
}

fn drain_until(core: &Core, done: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        core.drain_worktree_cleanup();
        if done() {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "worktree cleanup did not finish"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn checkout(core: &Core, task: &Task, suffix: &str, commit: &str) -> PathBuf {
    let root = PathBuf::from(
        core.store
            .conversation(&core.store.plan_conversation(&task.plan_id).unwrap())
            .unwrap()
            .project,
    );
    let name = format!("{}-{suffix}", task.id);
    let path = core.state.join("worktrees").join(&name);
    git::add_worktree(&root, &path, &format!("antislop/{name}"), commit).unwrap();
    path
}

#[test]
fn approved_tasks_release_clean_views_and_keep_dirty_and_other_task_views() {
    let (_directory, core, conversation, tasks, commit) = fixture();
    let root = PathBuf::from(core.store.conversation(&conversation).unwrap().project);
    let mut task = tasks[0].clone();
    let clean = checkout(&core, &task, "observation-g0", &commit);
    let dirty = checkout(&core, &task, "review-g0-1", &commit);
    let other = checkout(&core, &tasks[1], "observation-g0", &commit);
    std::fs::write(dirty.join("unsaved"), "preserve this work").unwrap();
    let head = git::head(&root).unwrap();
    let index = std::fs::read(root.join(".git/index")).unwrap();
    let pin = core.pin_worktree(&clean);
    core.set_status(&conversation, &mut task, status::APPROVED)
        .unwrap();
    core.drain_worktree_cleanup();
    assert!(
        clean.exists(),
        "a restoring native session pins its checkout"
    );
    drop(pin);
    drain_until(&core, || !clean.exists());
    assert!(!clean.exists());
    assert!(dirty.exists() && other.exists());
    assert_eq!(
        std::fs::read_to_string(dirty.join("unsaved")).unwrap(),
        "preserve this work"
    );
    assert!(
        !git::branches(&root, "antislop/")
            .unwrap()
            .contains(&format!("antislop/{}-observation-g0", task.id))
    );
    assert_eq!(git::head(&root).unwrap(), head);
    assert_eq!(std::fs::read(root.join(".git/index")).unwrap(), index);
    // The exact committed view survives cleanup and aggressive Git GC.
    assert!(
        std::process::Command::new("git")
            .args(["gc", "--prune=now", "--quiet"])
            .current_dir(&root)
            .status()
            .unwrap()
            .success()
    );
    git::restore_clean_view(&root, &clean).unwrap();
    assert_eq!(git::head(&clean).unwrap(), commit);
}

#[test]
fn old_generations_are_cleaned_after_execution_without_losing_writer_commits() {
    let (_directory, core, conversation, tasks, commit) = fixture();
    let task = &tasks[0];
    let old = checkout(&core, task, "g0", &commit);
    std::fs::write(old.join("result"), "unique result").unwrap();
    let result = git::commit_all(&old, git::CommitPurpose::TestFixture).unwrap();
    let dirty = checkout(&core, task, "g1", &commit);
    std::fs::write(dirty.join("unsaved"), "unsaved").unwrap();
    let current = checkout(&core, task, "g2", &commit);
    let old_view = checkout(&core, task, "review-g0-1", &commit);
    let current_view = checkout(&core, task, "observation-g2", &commit);
    let run = core
        .store
        .create_run(&conversation, "Changed inputs")
        .unwrap();
    core.store
        .db()
        .execute(
            "INSERT INTO task_inputs(task_id,run_id,generation) VALUES(?1,?2,2)",
            rusqlite::params![task.id, run.id],
        )
        .unwrap();
    core.task_cancels.lock().unwrap().insert(
        task.id.clone(),
        (conversation.clone(), Arc::new(AtomicBool::new(false))),
    );
    let after = core.store.task_by_id(&task.id).unwrap().unwrap();
    core.cancel_changed_task_inputs(std::slice::from_ref(task), std::slice::from_ref(&after));
    core.drain_worktree_cleanup();
    assert!(
        old.exists() && old_view.exists(),
        "active task directories are retained"
    );
    core.task_cancels.lock().unwrap().remove(&task.id);
    drain_until(&core, || !old.exists() && !old_view.exists());
    assert!(!old.exists() && !old_view.exists());
    assert!(dirty.exists() && current.exists() && current_view.exists());
    let root = PathBuf::from(core.store.conversation(&conversation).unwrap().project);
    let branch = format!("antislop/{}-g0", task.id);
    let restored = core.state.join("restored-writer");
    git::add_worktree(&root, &restored, &branch, &result).unwrap();
    assert_eq!(
        std::fs::read_to_string(restored.join("result")).unwrap(),
        "unique result"
    );
}

#[test]
fn cleanup_preserves_a_checkout_reassigned_to_a_user_branch() {
    let (_directory, core, conversation, tasks, commit) = fixture();
    let mut task = tasks[0].clone();
    let path = checkout(&core, &task, "observation-g0", &commit);
    assert!(
        std::process::Command::new("git")
            .args(["switch", "--quiet", "-c", "user-work"])
            .current_dir(&path)
            .status()
            .unwrap()
            .success()
    );
    core.set_status(&conversation, &mut task, status::APPROVED)
        .unwrap();
    core.drain_worktree_cleanup();
    assert!(path.exists());
    assert_eq!(git::worktree_branch(&path).unwrap(), "user-work");
}

#[test]
fn cleanup_does_not_follow_a_worktree_directory_symlink_into_another_repository() {
    let (_directory, core, conversation, tasks, _commit) = fixture();
    let external = tempfile::tempdir().unwrap();
    let root = external.path().canonicalize().unwrap();
    assert!(
        std::process::Command::new("git")
            .args(["init", "--quiet", "-b", "main"])
            .current_dir(&root)
            .status()
            .unwrap()
            .success()
    );
    std::fs::write(root.join("precious"), "foreign data").unwrap();
    let base = git::commit_all(&root, git::CommitPurpose::TestFixture).unwrap();
    let name = format!("{}-observation-g0", tasks[0].id);
    let foreign = root.join("views").join(&name);
    git::add_worktree(&root, &foreign, &format!("antislop/{name}"), &base).unwrap();
    std::os::unix::fs::symlink(root.join("views"), core.state.join("worktrees")).unwrap();
    let mut task = tasks[0].clone();
    core.set_status(&conversation, &mut task, status::APPROVED)
        .unwrap();
    core.drain_worktree_cleanup();
    assert!(foreign.exists());
    assert_eq!(
        std::fs::read_to_string(foreign.join("precious")).unwrap(),
        "foreign data"
    );
}

#[test]
fn old_writer_contracts_protect_uncommitted_ignored_artifacts() {
    let (_directory, core, conversation, tasks, commit) = fixture();
    let task = &tasks[0];
    let writer = checkout(&core, task, "g0", &commit);
    std::fs::create_dir(writer.join(".antislop")).unwrap();
    std::fs::write(writer.join(".antislop/report.md"), "uncommitted report").unwrap();
    assert!(git::is_clean(&writer), "the report is ignored by Git");
    let previous = json!({"id":"analysis","outputs":[".antislop/report.md"]});
    core.store
        .db()
        .execute(
            "INSERT INTO task_contracts(task_id,version,spec) VALUES(?1,17,?2)",
            rusqlite::params![task.id, previous.to_string()],
        )
        .unwrap();
    let run = core.store.create_run(&conversation, "New inputs").unwrap();
    core.store
        .db()
        .execute(
            "INSERT INTO task_inputs(task_id,run_id,generation) VALUES(?1,?2,1)",
            rusqlite::params![task.id, run.id],
        )
        .unwrap();
    core.queue_worktree_cleanup(&task.id);
    core.drain_worktree_cleanup();
    assert_eq!(
        std::fs::read_to_string(writer.join(".antislop/report.md")).unwrap(),
        "uncommitted report"
    );
}

#[test]
fn deferred_old_round_views_are_released_even_when_the_task_stalls() {
    let (_directory, core, conversation, tasks, commit) = fixture();
    let mut task = tasks[0].clone();
    task.round = 2;
    core.store.update_task(&task).unwrap();
    let old = checkout(&core, &task, "review-g0-1", &commit);
    let current = checkout(&core, &task, "review-g0-2", &commit);
    let observation = checkout(&core, &task, "observation-g0", &commit);
    let pin = core.pin_worktree(&old);
    core.set_status(&conversation, &mut task, status::COORDINATING)
        .unwrap();
    core.drain_worktree_cleanup();
    assert!(old.exists());
    drop(pin);
    drain_until(&core, || !old.exists());
    assert!(
        current.exists() && observation.exists(),
        "a stalled task retains its current views"
    );
}

#[test]
fn retired_checkouts_leave_no_ignored_files_behind() {
    let (_directory, core, conversation, tasks, commit) = fixture();
    let mut task = tasks[0].clone();
    let writer = checkout(&core, &task, "g0", &commit);
    let view = checkout(&core, &task, "observation-g0", &commit);
    for path in [&writer, &view] {
        // A tagged build cache, an untagged one and other ignored state.
        let tag = "Signature: 8a477f597d28d172789f06886806bc55\n# cache\n";
        std::fs::create_dir_all(path.join("target/debug")).unwrap();
        std::fs::write(path.join("target/CACHEDIR.TAG"), tag).unwrap();
        std::fs::create_dir_all(path.join("benchmark/__pycache__")).unwrap();
        std::fs::write(path.join("benchmark/__pycache__/run.pyc"), "pyc").unwrap();
        std::fs::create_dir_all(path.join(".antislop/qualification")).unwrap();
        std::fs::write(path.join(".antislop/qualification/journal"), "j").unwrap();
        std::fs::write(
            path.join(".gitignore"),
            "target/\n__pycache__/\n.antislop/\n",
        )
        .unwrap();
        git::git(path, &["add", ".gitignore"]).unwrap();
        git::commit_all(path, git::CommitPurpose::TestFixture).unwrap();
        assert!(git::is_clean(path), "Git ignores all of them");
    }
    core.set_status(&conversation, &mut task, "superseded")
        .unwrap();
    core.drain_worktree_cleanup();
    drain_until(&core, || !writer.exists() && !view.exists());
    super::super::core_path_helpers::release_settled_views(&core.store, &core.state).unwrap();
    assert_eq!(
        core.store.task_by_id(&task.id).unwrap().unwrap().status,
        "superseded"
    );
}

#[test]
fn durable_runtime_survives_checkout_cleanup_and_is_discovered_by_successor() {
    let (_directory, core, conversation, tasks, commit) = fixture();
    let mut source = tasks[0].clone();
    let writer = checkout(&core, &source, "g0", &commit);
    let runtime = super::super::core_task_runtime::directory(&core.state, &source).unwrap();
    for file in ["journal", "snapshot", "training.json"] {
        std::fs::write(runtime.join(file), file).unwrap();
    }
    core.store.db().execute("INSERT INTO task_resume_results(task_id,generation,source_task,result_turn) VALUES(?1,0,?2,NULL)", rusqlite::params![tasks[1].id,source.id]).unwrap();
    core.set_status(&conversation, &mut source, "superseded")
        .unwrap();
    drain_until(&core, || !writer.exists());
    super::super::core_path_helpers::release_settled_views(&core.store, &core.state).unwrap();
    let handoff = super::super::core_task_runtime::sources(&core, &tasks[1]).unwrap();
    assert_eq!(handoff.len(), 1);
    assert_eq!(handoff[0]["task"], source.id);
    assert_eq!(handoff[0]["available"], true);
    assert_eq!(
        handoff[0]["directory"],
        runtime.parent().unwrap().to_string_lossy().as_ref()
    );
    source.input_generation += 1;
    let next = super::super::core_task_runtime::directory(&core.state, &source).unwrap();
    assert_ne!(runtime, next);
    for file in ["journal", "snapshot", "training.json"] {
        assert_eq!(std::fs::read_to_string(runtime.join(file)).unwrap(), file);
    }
}

#[test]
fn approval_keeps_only_the_runtime_generation_that_produced_the_result() {
    let (directory, core, conversation, tasks, _commit) = fixture();
    let mut task = tasks[0].clone();
    let run = core.store.create_run(&conversation, "inputs").unwrap();
    core.store
        .db()
        .execute(
            "INSERT INTO task_inputs(task_id,run_id,generation) VALUES(?1,?2,1)",
            [&task.id, &run.id],
        )
        .unwrap();
    let owner = core.state.join("task-state").join(&task.id);
    let other = super::super::core_task_runtime::directory(&core.state, &tasks[1]).unwrap();
    let outside = directory.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    for name in ["g0/run-1", "g1/run-2", "notes"] {
        std::fs::create_dir_all(owner.join(name)).unwrap();
    }
    std::os::unix::fs::symlink(&outside, owner.join("g2")).unwrap();
    task.input_generation = 1;

    core.set_status(&conversation, &mut task, status::WORKING)
        .unwrap();
    core.drain_worktree_cleanup();
    assert!(
        owner.join("g0/run-1").is_dir(),
        "a running task keeps every generation"
    );

    core.set_status(&conversation, &mut task, status::APPROVED)
        .unwrap();
    drain_until(&core, || !owner.join("g0").exists());
    assert!(owner.join("g1/run-2").is_dir());
    assert!(owner.join("notes").is_dir() && owner.join("g2").is_symlink());
    assert!(outside.is_dir() && other.is_dir());
}

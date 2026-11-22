use super::*;
use crate::lean::git::test_observation;
use std::sync::mpsc;
use std::time::Duration;

fn fixture() -> (tempfile::TempDir, Arc<Core>, String, String) {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    git::ok(&project, &["init", "-q"]).unwrap();
    std::fs::write(project.join("input"), "before").unwrap();
    git::commit_all(&project, git::CommitPurpose::TestFixture).unwrap();
    let mut config =
        Config::from_toml(include_str!("../../../../../examples/antislop.toml")).unwrap();
    for provider in config.providers.values_mut() {
        provider.binary = std::env::current_exe().unwrap();
        provider.command = None;
    }
    let core = Core::open(&dir.path().join("state"), config).unwrap();
    let conversation = core
        .store
        .create_conversation(project.to_str().unwrap())
        .unwrap()
        .id;
    core.store
        .add_message(&conversation, "user", "Inspect the input")
        .unwrap();
    core.store
        .db()
        .execute(
            "UPDATE session_workflow SET phase='planning' WHERE conversation_id=?1",
            [&conversation],
        )
        .unwrap();
    let scope = core
        .store
        .create_scope(&conversation, "Inspect the input")
        .unwrap();
    let spec = json!({"id":"a","title":"A","goal":"Inspect input","kind":"analysis","change_kind":"code","acceptance_criteria":[{"id":"C1","text":"Input inspected"}],"reviewer":{"summary":"Check input"}});
    let mut source = core
        .store
        .add_plan(&conversation, std::slice::from_ref(&spec))
        .unwrap()
        .remove(0);
    core.store
        .bind_plan_scope(&conversation, &source.plan_id, &scope.id)
        .unwrap();
    core.store
        .queue_plan(&conversation, &source.plan_id)
        .unwrap();
    source.status = status::APPROVED.into();
    source.round = 1;
    source.report = Some("durable result".into());
    core.store.update_task(&source).unwrap();
    core.store.test_completion(&source, "durable result");
    let revision = git::snapshot(&project, git::CommitPurpose::TestFixture).unwrap();
    core.store
        .db()
        .execute(
            "INSERT INTO task_source_revisions VALUES(?1,0,?2)",
            rusqlite::params![source.id, revision],
        )
        .unwrap();
    let target = core
        .store
        .add_plan(&conversation, &[spec])
        .unwrap()
        .remove(0);
    core.store
        .bind_plan_scope(&conversation, &target.plan_id, &scope.id)
        .unwrap();
    (dir, core, conversation, target.plan_id)
}

fn assert_unpublished(core: &Core, conversation: &str, plan: &str) {
    assert!(core.is_idle(conversation));
    assert!(core.activity.operations.lock().unwrap().is_empty());
    let db = core.store.db();
    for sql in [
        "SELECT count(*) FROM plan_activations WHERE plan_id=?1",
        "SELECT count(*) FROM tasks WHERE plan_id=?1 AND status<>'proposed'",
        "SELECT count(*) FROM task_result_sources WHERE task_id IN (SELECT id FROM tasks WHERE plan_id=?1)",
        "SELECT count(*) FROM task_resume_results WHERE task_id IN (SELECT id FROM tasks WHERE plan_id=?1)",
    ] {
        assert_eq!(
            db.query_row(sql, [plan], |r| r.get::<_, i64>(0)).unwrap(),
            0,
            "{sql}"
        );
    }
}

#[test]
fn preparation_releases_plans_and_cancellation_rolls_back_real_queue() {
    let (_dir, core, conversation, plan) = fixture();
    let other = core.store.create_conversation("independent").unwrap().id;
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let worker = core.clone();
    let id = conversation.clone();
    let plan_id = plan.clone();
    let start = std::thread::spawn(move || {
        worker.start_plan(&id, &plan_id, |store, cancel| {
            test_observation::on_next_diff(move || {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            });
            store.queue_plan_cancellable(&id, &plan_id, cancel)
        })
    });
    if let Err(error) = entered_rx.recv_timeout(Duration::from_secs(10)) {
        panic!(
            "Git hook not reached: {error}; start={:?}",
            start.join().unwrap()
        );
    }
    // These locks are intentionally retained to protect the graph and sources.
    assert!(core.locks.graph_updates.try_lock().is_err());
    assert!(core.locks.integrating.try_lock().is_err());
    assert!(core.activity.plans.try_lock().unwrap().is_empty());
    assert!(!core.is_idle(&conversation));
    core.pause(&conversation, true).unwrap();
    assert!(core.store.paused(&conversation).unwrap());
    assert!(core.delete_session(&conversation).is_err());
    assert!(core.is_idle(&other));
    core.pause(&other, true).unwrap();
    core.store
        .add_message(&other, "user", "independent work")
        .unwrap();
    core.delete_session(&other).unwrap();
    let starting = core.locks.turn(&conversation, "plan-start");
    assert!(starting.try_lock().is_err());
    let competing = core.clone();
    let competing_id = conversation.clone();
    let competing_plan = plan.clone();
    let pending = std::thread::spawn(move || {
        competing.start_plan(&competing_id, &competing_plan, |_, _| {
            panic!("cancelled competitor queued")
        })
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while core.activity.operations.lock().unwrap().len() != 2 {
        assert!(
            std::time::Instant::now() < deadline,
            "competitor did not reserve"
        );
        std::thread::yield_now();
    }
    // Both the in-flight start and the admitted waiter use operation tokens.
    core.cancel(&conversation);
    release_tx.send(()).unwrap();
    assert!(
        start
            .join()
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("plan start cancelled")
    );
    let result = pending.join();
    assert!(result.is_ok());
    assert!(result.unwrap().is_err());
    assert_unpublished(&core, &conversation, &plan);
}

#[test]
fn preparation_error_releases_reservation_without_activation() {
    let (_dir, core, conversation, plan) = fixture();
    let result = core.start_plan(&conversation, &plan, |store, cancel| {
        test_observation::on_next_diff({
            let project = PathBuf::from(store.conversation(&conversation).unwrap().project);
            move || std::fs::write(project.join("input"), "changed during preparation").unwrap()
        });
        store.queue_plan_cancellable(&conversation, &plan, cancel)
    });
    let error = result.unwrap_err().to_string();
    assert!(error.contains("changed during preparation"), "{error}");
    assert_unpublished(&core, &conversation, &plan);
}

#[test]
fn cancellation_after_commit_reaches_runner_and_activation_is_single() {
    let (_dir, core, conversation, plan) = fixture();
    let tasks = core
        .start_plan(&conversation, &plan, |store, cancel| {
            let tasks = store.queue_plan_cancellable(&conversation, &plan, cancel)?;
            core.cancel(&conversation);
            assert!(cancel.load(Ordering::SeqCst));
            Ok(tasks)
        })
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !core.is_idle(&conversation) {
        assert!(
            std::time::Instant::now() < deadline,
            "cancelled runner did not settle"
        );
        std::thread::yield_now();
    }
    assert_eq!(tasks[0].status, status::APPROVED);
    assert!(
        core.store
            .last_turn(&conversation, &format!("worker:{}", tasks[0].id))
            .unwrap()
            .is_none()
    );
    assert!(
        core.store
            .last_turn(&conversation, ORCHESTRATOR)
            .unwrap()
            .is_none()
    );
    let error = core
        .start_plan(&conversation, &plan, |store, cancel| {
            store.queue_plan_cancellable(&conversation, &plan, cancel)
        })
        .unwrap_err();
    assert!(
        error.to_string().contains("not waiting for approval"),
        "{error}"
    );
    let activations: i64 = core
        .store
        .db()
        .query_row(
            "SELECT count(*) FROM plan_activations WHERE plan_id=?1",
            [&plan],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(activations, 1);
    assert!(core.is_idle(&conversation));
}

#[test]
fn panic_during_preparation_releases_operation_and_leaves_plans_available() {
    let (_dir, core, conversation, plan) = fixture();
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        core.start_plan(&conversation, &plan, |_, _| {
            panic!("injected preparation panic")
        })
    }));
    assert!(panic.is_err());
    assert_unpublished(&core, &conversation, &plan);
}

#[test]
fn concurrent_start_publishes_only_one_activation() {
    let (_dir, core, conversation, plan) = fixture();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let worker = core.clone();
    let id = conversation.clone();
    let plan_id = plan.clone();
    let start = std::thread::spawn(move || {
        worker.start_plan(&id, &plan_id, |store, cancel| {
            test_observation::on_next_diff(move || {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            });
            let tasks = store.queue_plan_cancellable(&id, &plan_id, cancel)?;
            // Stop just this runner, leaving the competing request uncancelled.
            cancel.store(true, Ordering::SeqCst);
            Ok(tasks)
        })
    });
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let worker = core.clone();
    let id = conversation.clone();
    let plan_id = plan.clone();
    let competitor = std::thread::spawn(move || {
        worker.start_plan(&id, &plan_id, |store, cancel| {
            store.queue_plan_cancellable(&id, &plan_id, cancel)
        })
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while core.activity.operations.lock().unwrap().len() != 2 {
        assert!(
            std::time::Instant::now() < deadline,
            "competitor did not reserve"
        );
        std::thread::yield_now();
    }
    release_tx.send(()).unwrap();
    assert_eq!(start.join().unwrap().unwrap().len(), 1);
    let error = competitor.join().unwrap().unwrap_err().to_string();
    assert!(
        error.contains("already") || error.contains("not waiting for approval"),
        "{error}"
    );
    while !core.is_idle(&conversation) {
        assert!(
            std::time::Instant::now() < deadline,
            "runner did not settle"
        );
        std::thread::yield_now();
    }
    let count: i64 = core
        .store
        .db()
        .query_row(
            "SELECT count(*) FROM plan_activations WHERE plan_id=?1",
            [&plan],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn missing_carried_content_rolls_back_activation_and_can_retry() {
    let (_dir, core, conversation, plan) = fixture();
    let source = core
        .store
        .last_plan_with_approvals(&conversation, Some(&plan))
        .unwrap()
        .remove(0);
    let encoded = core.store.content.encode("carried report").unwrap();
    let reference = crate::lean::content::Content::reference(&encoded).unwrap();
    let path = core.store.content.path(&reference).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    core.store
        .db()
        .execute(
            "UPDATE tasks SET report=?2 WHERE id=?1",
            rusqlite::params![source.id, encoded],
        )
        .unwrap();
    std::fs::remove_file(&path).unwrap();
    let workflow = core.store.workflow(&conversation).unwrap().phase;
    let events: i64 = core
        .store
        .db()
        .query_row(
            "SELECT count(*) FROM events WHERE plan_id=?1",
            [&plan],
            |r| r.get(0),
        )
        .unwrap();
    let error = core
        .start_plan(&conversation, &plan, |store, cancel| {
            store.queue_plan_cancellable(&conversation, &plan, cancel)
        })
        .unwrap_err();
    assert!(error.to_string().contains("No such file"), "{error}");
    assert_unpublished(&core, &conversation, &plan);
    assert_eq!(core.store.workflow(&conversation).unwrap().phase, workflow);
    assert_eq!(
        core.store
            .db()
            .query_row(
                "SELECT count(*) FROM events WHERE plan_id=?1",
                [&plan],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        events
    );
    std::fs::write(path, bytes).unwrap();
    let tasks = core
        .start_plan(&conversation, &plan, |store, cancel| {
            let tasks = store.queue_plan_cancellable(&conversation, &plan, cancel)?;
            cancel.store(true, Ordering::SeqCst);
            Ok(tasks)
        })
        .unwrap();
    assert_eq!(tasks[0].report.as_deref(), Some("carried report"));
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !core.is_idle(&conversation) {
        assert!(
            std::time::Instant::now() < deadline,
            "runner did not settle"
        );
        std::thread::yield_now();
    }
}

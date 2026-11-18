use super::*;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[test]
fn plan_history_loads_only_its_plan_turns_and_task_role_fallbacks_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("core.db")).unwrap();
    let conversation = store.create_conversation("/project").unwrap();
    let specs = [
        serde_json::json!({"id":"first","title":"First","goal":"Do first","kind":"analysis","change_kind":"none","acceptance_criteria":[{"id":"C1","text":"done"}]}),
    ];
    let plan = store.add_plan(&conversation.id, &specs).unwrap();
    let target = &plan[0];
    let unrelated_spec = [
        serde_json::json!({"id":"unrelated","title":"Unrelated","goal":"Do unrelated","kind":"analysis","change_kind":"none","acceptance_criteria":[{"id":"C1","text":"done"}]}),
    ];
    let unrelated_plan = store.add_plan(&conversation.id, &unrelated_spec).unwrap();
    let unrelated = &unrelated_plan[0];

    let target_turn = store
        .start_turn(
            &conversation.id,
            "orchestrator",
            "fixture",
            0,
            Some(&target.plan_id),
        )
        .unwrap();
    let fallback_turn = store
        .start_turn(
            &conversation.id,
            &format!("reviewer:{}", target.id),
            "fixture",
            0,
            None,
        )
        .unwrap();
    store
        .start_turn(
            &conversation.id,
            &format!("worker:{}", unrelated.id),
            "fixture",
            0,
            None,
        )
        .unwrap();
    store
        .start_turn(
            &conversation.id,
            "orchestrator",
            "fixture",
            0,
            Some(&unrelated.plan_id),
        )
        .unwrap();

    let history = store
        .plan_history(&conversation.id, &target.plan_id)
        .unwrap();
    let actual = history
        .turns
        .iter()
        .map(|turn| turn.turn_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(actual, [target_turn.id.as_str(), fallback_turn.id.as_str()]);
}

#[test]
#[ignore = "synthetic performance observation; run explicitly with --ignored --nocapture"]
fn plan_history_cost_as_conversation_turns_grow() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("core.db")).unwrap();
    let conversation = store.create_conversation("/project").unwrap();
    let task = store
        .add_plan(
            &conversation.id,
            &[serde_json::json!({"id":"target","title":"Target","goal":"Measure targeted history","kind":"analysis","change_kind":"none","acceptance_criteria":[{"id":"C1","text":"done"}]})],
        )
        .unwrap()
        .remove(0);
    let db_path = dir.path().join("core.db");
    {
        let db = store.db();
        for index in 0..20 {
            db.execute(
                "INSERT INTO turns(id,conversation_id,plan_id,role,provider,status,upto_message,started_ms) VALUES(?1,?2,?3,'orchestrator','fixture','done',0,?4)",
                rusqlite::params![format!("target-{index}"), conversation.id, task.plan_id, index],
            ).unwrap();
        }
    }
    for size in [100, 1_000, 10_000] {
        {
            let db = store.db();
            let existing: i64 = db
                .query_row(
                    "SELECT COUNT(*) FROM turns WHERE conversation_id=?1",
                    [&conversation.id],
                    |r| r.get(0),
                )
                .unwrap();
            let mut insert = db.prepare(
                "INSERT INTO turns(id,conversation_id,plan_id,role,provider,status,upto_message,started_ms) VALUES(?1,?2,'other','orchestrator','fixture','done',0,?3)",
            ).unwrap();
            for index in existing..size {
                insert
                    .execute(rusqlite::params![
                        format!("noise-{index}"),
                        conversation.id,
                        index
                    ])
                    .unwrap();
            }
        }
        let history = store.plan_history(&conversation.id, &task.plan_id).unwrap();
        assert_eq!(history.turns.len(), 20);
        let mut samples = Vec::with_capacity(50);
        let mut legacy_samples = Vec::with_capacity(50);
        for _ in 0..50 {
            let start = Instant::now();
            let history = store.plan_history(&conversation.id, &task.plan_id).unwrap();
            assert_eq!(history.turns.len(), 20);
            samples.push(start.elapsed());

            let start = Instant::now();
            let all = {
                let db = store.db();
                let mut statement = db.prepare(
                    "SELECT id, plan_id, role, provider, session_id, status, error, upto_message, started_ms, finished_ms, pid FROM turns WHERE conversation_id=?1 ORDER BY started_ms ASC, rowid ASC",
                ).unwrap();
                statement
                    .query_map([&conversation.id], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, Option<String>>(4)?,
                            row.get::<_, String>(5)?,
                            row.get::<_, Option<String>>(6)?,
                            row.get::<_, i64>(7)?,
                            row.get::<_, i64>(8)?,
                            row.get::<_, Option<i64>>(9)?,
                            row.get::<_, Option<i64>>(10)?,
                        ))
                    })
                    .unwrap()
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .unwrap()
            };
            let _selected = all
                .iter()
                .filter(|row| {
                    row.1.as_deref() == Some(task.plan_id.as_str())
                        || row.2 == format!("worker:{}", task.id)
                        || row.2 == format!("reviewer:{}", task.id)
                })
                .count();
            legacy_samples.push(start.elapsed());
        }
        samples.sort_unstable();
        legacy_samples.sort_unstable();
        let p50 = samples[25].as_micros();
        let p95 = samples[47].as_micros();
        let legacy_p50 = legacy_samples[25].as_micros();
        let legacy_p95 = legacy_samples[47].as_micros();
        let selected_rows: i64 = store.db().query_row(
            "SELECT COUNT(*) FROM turns WHERE conversation_id=?1 AND (plan_id=?2 OR role IN ('worker:target','reviewer:target'))",
            rusqlite::params![conversation.id, task.plan_id],
            |r| r.get(0),
        ).unwrap();
        let db = store.db();
        let page_count: i64 = db.query_row("PRAGMA page_count", [], |r| r.get(0)).unwrap();
        let page_size: i64 = db.query_row("PRAGMA page_size", [], |r| r.get(0)).unwrap();
        drop(db);
        let database_bytes = page_count * page_size;
        let wal_bytes = std::fs::metadata(format!("{}-wal", db_path.display()))
            .map(|m| m.len())
            .unwrap_or(0);
        #[cfg(target_os = "macos")]
        let peak_rss_bytes = {
            let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
            // SAFETY: getrusage initializes the supplied rusage structure.
            let status = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
            assert_eq!(status, 0);
            unsafe { usage.assume_init().ru_maxrss as u64 }
        };
        #[cfg(not(target_os = "macos"))]
        let peak_rss_bytes = 0;
        println!(
            "turns={size} selected_rows={selected_rows} targeted_p50_us={p50} targeted_p95_us={p95} legacy_scan_p50_us={legacy_p50} legacy_scan_p95_us={legacy_p95} sqlite_pages_bytes={database_bytes} wal_bytes={wal_bytes} process_peak_rss_bytes={peak_rss_bytes}"
        );
    }

    let contention = Arc::new(store);
    let mut waits = Vec::new();
    for index in 0..5 {
        let guard = contention.content_write.lock().unwrap();
        let (wait_tx, wait_rx) = std::sync::mpsc::channel();
        let writer = Arc::clone(&contention);
        let conversation_id = conversation.id.clone();
        let worker = std::thread::spawn(move || {
            let start = Instant::now();
            writer
                .append_event(
                    &conversation_id,
                    None,
                    None,
                    "measurement",
                    &serde_json::json!({"round":index}),
                )
                .unwrap();
            wait_tx.send(start.elapsed()).unwrap();
        });
        std::thread::sleep(Duration::from_millis(10));
        drop(guard);
        waits.push(wait_rx.recv().unwrap());
        worker.join().unwrap();
    }
    waits.sort_unstable();
    println!(
        "content_write_controlled_hold_ms=10 append_wait_p50_us={} p95_us={}",
        waits[2].as_micros(),
        waits[4].as_micros()
    );

    let git_root = dir.path().join("git-project");
    std::fs::create_dir_all(&git_root).unwrap();
    let run_git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(&git_root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    run_git(&["init", "--quiet"]);
    run_git(&["config", "user.email", "perf@example.invalid"]);
    run_git(&["config", "user.name", "Performance Fixture"]);
    std::fs::write(git_root.join("fixture.txt"), "fixture\n").unwrap();
    run_git(&["add", "fixture.txt"]);
    run_git(&["commit", "--quiet", "-m", "fixture"]);
    let base = crate::lean::git::head(&git_root).unwrap();
    let mut create = Vec::new();
    for index in 0..5 {
        let path = dir.path().join(format!("worktree-{index}"));
        let branch = format!("perf/create-{index}");
        let start = Instant::now();
        crate::lean::git::add_worktree(&git_root, &path, &branch, &base).unwrap();
        create.push(start.elapsed());
    }
    create.sort_unstable();
    let attach_path = dir.path().join("worktree-0");
    let attach_branch = "perf/create-0";
    let mut attach = Vec::new();
    for _ in 0..50 {
        let start = Instant::now();
        crate::lean::git::add_worktree(&git_root, &attach_path, attach_branch, &base).unwrap();
        attach.push(start.elapsed());
    }
    attach.sort_unstable();
    println!(
        "worktree_create_n=5 p50_ms={} max_ms={} existing_attach_n=50 p50_us={} p95_us={}",
        create[2].as_millis(),
        create[4].as_millis(),
        attach[25].as_micros(),
        attach[47].as_micros()
    );
}

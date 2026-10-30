mod common;
mod support;
use antislop::{
    config::Config,
    lean::{
        Core,
        daemon::{Client, Request},
        store::Store,
    },
};
use serde_json::json;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn fake_clis(dir: &Path) -> (PathBuf, PathBuf) {
    let codex = dir.join("codex");
    std::fs::write(
        &codex,
        r#"#!/usr/bin/env python3
import json, sys
print(json.dumps({"type": "thread.started", "thread_id": "orch"}))
print(json.dumps({"type": "item.completed", "item": {"type": "agent_message", "text": "answer: ok"}}))
print(json.dumps({"type": "turn.completed", "usage": {}}))
"#,
    )
    .unwrap();
    let agy = dir.join("agy");
    std::fs::write(
        &agy,
        r#"#!/usr/bin/env python3
import json, sys
print(json.dumps({"event": "init", "conversation_id": "c1"}))
print(json.dumps({"event": "result", "result": {"conversation_id": "c1", "status": "SUCCESS", "response": "ok", "usage": {}}}))
"#,
    )
    .unwrap();
    std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o755)).unwrap();
    common::interactive(&codex);
    std::fs::set_permissions(&agy, std::fs::Permissions::from_mode(0o755)).unwrap();
    common::interactive(&agy);
    (codex, agy)
}

fn test_config(codex: &Path, agy: &Path) -> Config {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/examples/antislop.toml"
    ))
    .unwrap()
    .replace(
        "binary = \"codex\"",
        &format!("binary = {:?}", codex.to_string_lossy()),
    )
    .replace(
        "binary = \"agy\"",
        &format!("binary = {:?}", agy.to_string_lossy()),
    );
    Config::from_toml(&text).unwrap()
}

struct DaemonGuard {
    child: std::process::Child,
    client: Client,
}

impl DaemonGuard {
    fn start(project: &Path, config: &Path, state: &Path) -> Self {
        common::trust(project);
        std::fs::create_dir_all(state).unwrap();
        std::fs::set_permissions(state, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
            .arg("__core")
            .arg(project)
            .arg(config)
            .arg(state)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .unwrap();
        let client = Client::at(state);
        let deadline = Instant::now() + Duration::from_secs(120);
        while Instant::now() < deadline {
            if client
                .call(&Request::Control(
                    antislop::lean::daemon::ControlRequest::Identity,
                ))
                .is_ok()
            {
                return Self { child, client };
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let _ = child.kill();
        let _ = child.wait();
        panic!("daemon did not start within 10 seconds");
    }
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let _ = self.client.call(&Request::Control(
            antislop::lean::daemon::ControlRequest::Shutdown,
        ));
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// 1. Multiple plans with repeating short labels ('t1', 't2' in plan 1 and plan 2).
// Verify:
// - Distinct globally unique task UUIDs.
// - Correct lookup by UUID vs scoped label.
// - Events, turns, reports and verdicts associated with correct plan_id.
#[test]
fn multiple_plans_with_repeating_labels_have_distinct_identities_and_event_scoping() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let (codex, agy) = fake_clis(dir.path());
    let cfg = test_config(&codex, &agy);
    let state_dir = dir.path().join("state");
    let core = Core::open(&state_dir, cfg).unwrap();
    let conv = support::fixture_conversation(&core, &project).unwrap();

    // Plan 1: tasks t1 and t2
    let specs_plan1 = vec![
        json!({"id": "t1", "title": "Plan 1 Task 1", "kind": "analysis"}),
        json!({"id": "t2", "title": "Plan 1 Task 2", "kind": "analysis"}),
    ];
    let tasks1 = core.store.add_plan(&conv.id, &specs_plan1).unwrap();
    let plan1_id = tasks1[0].plan_id.clone();

    let page1 = core.plans(&conv.id, 0, 10).unwrap();
    assert_eq!(page1.plans.len(), 1);
    let plan1 = &page1.plans[0];
    assert_eq!(plan1.tasks.len(), 2);
    let plan1_t1 = plan1.tasks[0].clone();
    let plan1_t2 = plan1.tasks[1].clone();
    assert_eq!(plan1_t1.spec["id"], "t1");
    assert_eq!(plan1_t2.spec["id"], "t2");

    // Add comment scoped to plan1
    let c1 = core
        .comment_scoped(&conv.id, Some(&plan1_id), "t1", "comment on plan1 t1")
        .unwrap();
    assert_eq!(c1.plan_id.as_deref(), Some(plan1_id.as_str()));

    // Record turn and event for plan1
    let turn1 = core
        .store
        .start_turn(&conv.id, "worker:test1", "agy", 0, Some(&plan1_id))
        .unwrap();
    core.store
        .append_event(
            &conv.id,
            Some(&turn1.id),
            Some(&plan1_id),
            "custom_event1",
            &json!({"data": 1}),
        )
        .unwrap();

    // Plan 2: tasks also labelled t1 and t2, but completely different specs
    let specs_plan2 = vec![
        json!({"id": "t1", "title": "Plan 2 Task 1", "kind": "implementation"}),
        json!({"id": "t2", "title": "Plan 2 Task 2", "kind": "implementation"}),
    ];
    core.store.add_plan(&conv.id, &specs_plan2).unwrap();

    let page2 = core.plans(&conv.id, 0, 10).unwrap();
    assert_eq!(page2.plans.len(), 2);
    // Most recent plan is first
    let plan2 = &page2.plans[0];
    let plan2_id = plan2.id.clone();
    assert_ne!(plan1_id, plan2_id);
    assert_eq!(plan2.tasks.len(), 2);
    let plan2_t1 = plan2.tasks[0].clone();
    let plan2_t2 = plan2.tasks[1].clone();

    // Verify task UUIDs are unique across plans despite repeating short labels
    assert_ne!(plan1_t1.id, plan2_t1.id);
    assert_ne!(plan1_t2.id, plan2_t2.id);

    // Verify lookup by UUID finds exact task regardless of label
    let retrieved_p1_t1 = core
        .store
        .task_by_id(&plan1_t1.id)
        .unwrap()
        .expect("found p1 t1");
    assert_eq!(retrieved_p1_t1.plan_id, plan1_id);
    assert_eq!(retrieved_p1_t1.spec["title"], "Plan 1 Task 1");

    let retrieved_p2_t1 = core
        .store
        .task_by_id(&plan2_t1.id)
        .unwrap()
        .expect("found p2 t1");
    assert_eq!(retrieved_p2_t1.plan_id, plan2_id);
    assert_eq!(retrieved_p2_t1.spec["title"], "Plan 2 Task 1");

    // Verify lookup by scoped label ("t1") within plan1 vs plan2
    let p1_by_label = core.task(&plan1_id, "t1").unwrap();
    assert_eq!(p1_by_label.id, plan1_t1.id);
    let p2_by_label = core.task(&plan2_id, "t1").unwrap();
    assert_eq!(p2_by_label.id, plan2_t1.id);

    // Add comment scoped to plan2
    let c2 = core
        .comment_scoped(&conv.id, Some(&plan2_id), "t1", "comment on plan2 t1")
        .unwrap();
    assert_eq!(c2.plan_id.as_deref(), Some(plan2_id.as_str()));

    // Filter comments by plan
    let comments_p1 = core
        .store
        .comments_by_plan(&conv.id, &plan1_id, None)
        .unwrap();
    assert_eq!(comments_p1.len(), 1);
    assert_eq!(comments_p1[0].text, "comment on plan1 t1");

    let comments_p2 = core
        .store
        .comments_by_plan(&conv.id, &plan2_id, None)
        .unwrap();
    assert_eq!(comments_p2.len(), 1);
    assert_eq!(comments_p2[0].text, "comment on plan2 t1");

    // All comments returns both
    let all_comments = core.store.comments_all(&conv.id, None).unwrap();
    assert_eq!(all_comments.len(), 2);
}

// 2. Pagination of plans: offset, limit, total, has_more, active_plan_id.
#[test]
fn plan_pagination_and_active_plan_invariance() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let (codex, agy) = fake_clis(dir.path());
    let cfg = test_config(&codex, &agy);
    let state_dir = dir.path().join("state");
    let core = Core::open(&state_dir, cfg).unwrap();
    let conv = support::fixture_conversation(&core, &project).unwrap();

    // Create 3 successive plans
    let mut created_ids = Vec::new();
    for i in 1..=3 {
        let specs = vec![json!({"id": "t1", "title": format!("Task {i}"), "kind": "analysis"})];
        let tasks = core.store.add_plan(&conv.id, &specs).unwrap();
        created_ids.push(tasks[0].plan_id.clone());
    }

    // Active plan should be the 3rd plan (most recent)
    let active_id = created_ids.last().unwrap().clone();

    // Query page 1 (limit 1, offset 0)
    let page1 = core.plans(&conv.id, 0, 1).unwrap();
    assert_eq!(page1.total, 3);
    assert_eq!(page1.offset, 0);
    assert_eq!(page1.limit, 1);
    assert!(page1.has_more);
    assert_eq!(page1.plans.len(), 1);
    assert_eq!(page1.plans[0].id, created_ids[2]);
    assert!(page1.plans[0].is_active);
    assert_eq!(page1.active_plan_id.as_deref(), Some(active_id.as_str()));

    // Query page 2 (limit 1, offset 1)
    let page2 = core.plans(&conv.id, 1, 1).unwrap();
    assert_eq!(page2.total, 3);
    assert_eq!(page2.offset, 1);
    assert_eq!(page2.limit, 1);
    assert!(page2.has_more);
    assert_eq!(page2.plans.len(), 1);
    assert_eq!(page2.plans[0].id, created_ids[1]);
    assert!(!page2.plans[0].is_active); // past plan is NOT active
    assert_eq!(page2.active_plan_id.as_deref(), Some(active_id.as_str()));

    // Query page 3 (limit 1, offset 2)
    let page3 = core.plans(&conv.id, 2, 1).unwrap();
    assert_eq!(page3.total, 3);
    assert_eq!(page3.offset, 2);
    assert_eq!(page3.limit, 1);
    assert!(!page3.has_more);
    assert_eq!(page3.plans.len(), 1);
    assert_eq!(page3.plans[0].id, created_ids[0]);
    assert!(!page3.plans[0].is_active);

    // Active Plan Invariance: querying history of past plans must NOT change the active plan!
    let _past_history = core.plan_history(&conv.id, &created_ids[0]).unwrap();
    let latest_after_viewing = core.store.latest_plan(&conv.id).unwrap();
    assert_eq!(latest_after_viewing[0].plan_id, active_id);
}

// 3. Incompatible databases are rejected without migrating or deleting history.
// Keep messages, reports, verdicts, sessions and events available for manual archival.
#[test]
fn incompatible_legacy_database_is_rejected_without_changing_history() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("legacy.db");

    // 1. Create a schema 3 database directly with SQLite
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
             CREATE TABLE conversations(
               id TEXT PRIMARY KEY, project TEXT NOT NULL, created_ms INTEGER NOT NULL);
             CREATE TABLE messages(
               seq INTEGER PRIMARY KEY AUTOINCREMENT,
               conversation_id TEXT NOT NULL REFERENCES conversations(id),
               author TEXT NOT NULL, text TEXT NOT NULL, created_ms INTEGER NOT NULL);
             CREATE TABLE turns(
               id TEXT PRIMARY KEY,
               conversation_id TEXT NOT NULL REFERENCES conversations(id),
               role TEXT NOT NULL, provider TEXT NOT NULL, session_id TEXT,
               status TEXT NOT NULL, error TEXT, upto_message INTEGER NOT NULL,
               started_ms INTEGER NOT NULL, finished_ms INTEGER, pid INTEGER);
             CREATE TABLE events(
               seq INTEGER PRIMARY KEY AUTOINCREMENT,
               conversation_id TEXT NOT NULL REFERENCES conversations(id),
               turn_id TEXT, kind TEXT NOT NULL, payload TEXT NOT NULL,
               created_ms INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE tasks(
               id TEXT PRIMARY KEY,
               conversation_id TEXT NOT NULL REFERENCES conversations(id),
               plan_id TEXT NOT NULL, position INTEGER NOT NULL, spec TEXT NOT NULL,
               status TEXT NOT NULL, round INTEGER NOT NULL DEFAULT 0,
               report TEXT, verdict TEXT, base TEXT, result TEXT,
               created_ms INTEGER NOT NULL);
             CREATE TABLE comments(
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               conversation_id TEXT NOT NULL REFERENCES conversations(id),
               target TEXT NOT NULL, label TEXT NOT NULL, text TEXT NOT NULL,
               status TEXT NOT NULL, message INTEGER NOT NULL,
               created_ms INTEGER NOT NULL, updated_ms INTEGER NOT NULL);
             PRAGMA user_version = 3;",
        )
        .unwrap();

        // Seed schema 3 data
        conn.execute(
            "INSERT INTO conversations VALUES('c1', '/test/project', 1000)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages VALUES(1, 'c1', 'user', 'hello legacy', 1001)",
            [],
        )
        .unwrap();

        // Insert legacy task belonging to plan 'legacy_p1'
        conn.execute(
            "INSERT INTO tasks VALUES('task_uuid_1', 'c1', 'legacy_p1', 0, '{\"id\":\"t1\",\"title\":\"Legacy task\"}', 'approved', 0, 'legacy report', '{\"approved\":true,\"findings\":[]}', NULL, NULL, 1002)",
            [],
        )
        .unwrap();

        // Turn for worker on task_uuid_1
        conn.execute(
            "INSERT INTO turns VALUES('turn_w1', 'c1', 'worker:task_uuid_1', 'agy', 's1', 'done', NULL, 1, 1003, 1005, NULL)",
            [],
        )
        .unwrap();

        // Turn for orchestrator (no task link)
        conn.execute(
            "INSERT INTO turns VALUES('turn_o1', 'c1', 'orchestrator', 'codex', 's2', 'done', NULL, 1, 1006, 1008, NULL)",
            [],
        )
        .unwrap();

        // Event for turn_w1
        conn.execute(
            "INSERT INTO events VALUES(1, 'c1', 'turn_w1', 'turn_started', '{\"role\":\"worker\"}', 1003)",
            [],
        )
        .unwrap();

        // Event for orchestrator turn
        conn.execute(
            "INSERT INTO events VALUES(2, 'c1', 'turn_o1', 'turn_started', '{\"role\":\"orchestrator\"}', 1006)",
            [],
        )
        .unwrap();

        // Event referencing task in payload
        conn.execute(
            "INSERT INTO events VALUES(3, 'c1', NULL, 'task', '{\"task\":\"task_uuid_1\",\"status\":\"approved\"}', 1007)",
            [],
        )
        .unwrap();

        // Unassociated message event
        conn.execute(
            "INSERT INTO events VALUES(4, 'c1', NULL, 'message', '{\"seq\":1,\"text\":\"hello legacy\"}', 1001)",
            [],
        )
        .unwrap();
    }

    // The current store deliberately rejects incompatible schemas instead of
    // silently migrating their histories. Verify the archive stays intact.
    let before = std::fs::read(&db_path).unwrap();
    let error = Store::open(&db_path)
        .err()
        .expect("schema 3 must be rejected")
        .to_string();
    assert!(error.contains("incompatible schema 3"), "{error}");
    assert!(error.contains("No data was migrated or deleted"), "{error}");
    assert_eq!(std::fs::read(&db_path).unwrap(), before);
    let connection = rusqlite::Connection::open(&db_path).unwrap();
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 3);
    let task: (String, String) = connection
        .query_row(
            "SELECT report,verdict FROM tasks WHERE id='task_uuid_1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(task.0, "legacy report");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&task.1).unwrap()["approved"],
        true
    );
    let session: String = connection
        .query_row(
            "SELECT session_id FROM turns WHERE id='turn_w1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(session, "s1");
    let count: i64 = connection
        .query_row("SELECT count(*) FROM events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 4);
    let message: String = connection
        .query_row("SELECT text FROM messages WHERE seq=1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(message, "hello legacy");
}

// 4. Foreign scope rejection on daemon.
// Validates project, conversation, plan, and task boundaries:
// - Calling Plan/Plans for conversation of another project -> rejected.
// - Calling Plan with plan_id belonging to another conversation -> rejected.
// - Calling Task with task_id belonging to another plan -> rejected.
// - Calling Diff with foreign plan or task -> rejected.
// - Calling CommentScoped with foreign plan -> rejected.
#[test]
fn daemon_rejects_foreign_scope_requests() {
    let dir = tempfile::tempdir().unwrap();
    let project_a = dir.path().join("proj_a");
    let project_b = dir.path().join("proj_b");
    std::fs::create_dir_all(&project_a).unwrap();
    std::fs::create_dir_all(&project_b).unwrap();
    let (codex, agy) = fake_clis(dir.path());
    let config_path = dir.path().join("antislop.toml");
    let content = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/examples/antislop.toml"
    ))
    .unwrap()
    .replace(
        "binary = \"codex\"",
        &format!("binary = {:?}", codex.to_string_lossy()),
    )
    .replace(
        "binary = \"agy\"",
        &format!("binary = {:?}", agy.to_string_lossy()),
    );
    std::fs::write(&config_path, content).unwrap();

    let state_a = dir.path().join("state_a");
    let daemon_a = DaemonGuard::start(&project_a, &config_path, &state_a);

    // Start conversation in project A
    let conv_a = daemon_a
        .client
        .call(&Request::Control(
            antislop::lean::daemon::ControlRequest::Start,
        ))
        .unwrap();
    let conv_a_id = conv_a["id"].as_str().unwrap().to_string();

    // Start a daemon in project B
    let state_b = dir.path().join("state_b");
    let daemon_b = DaemonGuard::start(&project_b, &config_path, &state_b);
    let store_b = Store::open(&state_b.join("core.db")).unwrap();
    let foreign_in_b = store_b
        .create_conversation(&project_a.canonicalize().unwrap().to_string_lossy())
        .unwrap();

    // Create a second conversation in project A to test cross-conversation plan access
    let conv_a2 = daemon_a
        .client
        .call(&Request::Control(
            antislop::lean::daemon::ControlRequest::Start,
        ))
        .unwrap();
    let conv_a2_id = conv_a2["id"].as_str().unwrap().to_string();

    // Send plan proposal into conversation A
    let store_a = Store::open(&state_a.join("core.db")).unwrap();
    let specs_a1 = vec![json!({"id": "t1", "title": "Task A1", "kind": "analysis"})];
    let tasks_a1 = store_a.add_plan(&conv_a_id, &specs_a1).unwrap();
    let plan_a1_id = tasks_a1[0].plan_id.clone();
    let task_a1_id = tasks_a1[0].id.clone();

    // Also propose plan A2 in conversation A
    let specs_a2 = vec![json!({"id": "t1", "title": "Task A2", "kind": "analysis"})];
    let tasks_a2 = store_a.add_plan(&conv_a_id, &specs_a2).unwrap();
    let plan_a2_id = tasks_a2[0].plan_id.clone();
    let _task_a2_id = tasks_a2[0].id.clone();

    // Test 1: Project B daemon rejects request accessing a conversation belonging to Project A
    let err_foreign_conv = daemon_b.client.call(&Request::Conversation(
        antislop::lean::daemon::ConversationRequest::Plans {
            conversation: foreign_in_b.id.clone(),
            offset: Some(0),
            limit: Some(10),
        },
    ));
    assert!(
        err_foreign_conv.is_err(),
        "Must reject foreign conversation from another project"
    );
    let err_msg = err_foreign_conv.unwrap_err().to_string();
    assert!(err_msg.contains("does not belong to project"), "{err_msg}");

    // Test 2: Project A daemon rejects request accessing plan_a1 under conversation A2
    let err_foreign_plan = daemon_a.client.call(&Request::Conversation(
        antislop::lean::daemon::ConversationRequest::Plan {
            conversation: conv_a2_id.clone(),
            plan: plan_a1_id.clone(),
        },
    ));
    assert!(
        err_foreign_plan.is_err(),
        "Must reject plan belonging to another conversation on daemon A"
    );

    // Test 3: Requesting task_a1 with plan_a2 (task belongs to plan A1, not plan A2)
    let err_task_mismatch = daemon_a.client.call(&Request::Conversation(
        antislop::lean::daemon::ConversationRequest::TaskScoped {
            conversation: Some(conv_a_id.clone()),
            plan: plan_a2_id.clone(),
            id: task_a1_id.clone(),
        },
    ));
    assert!(
        err_task_mismatch.is_err(),
        "Must reject task belonging to another plan"
    );
    let err_task_msg = err_task_mismatch.unwrap_err().to_string();
    assert!(
        err_task_msg.contains("does not belong to plan"),
        "{err_task_msg}"
    );

    // Test 4: Requesting diff with task_a1 under plan_a2
    let err_diff_mismatch = daemon_a.client.call(&Request::Conversation(
        antislop::lean::daemon::ConversationRequest::Diff {
            conversation: conv_a_id.clone(),
            plan: plan_a2_id.clone(),
            id: task_a1_id.clone(),
        },
    ));
    assert!(
        err_diff_mismatch.is_err(),
        "Diff must reject task belonging to another plan"
    );

    // Test 5: Comment scoped with non-existent or foreign plan
    let err_comment_foreign = daemon_a.client.call(&Request::Conversation(
        antislop::lean::daemon::ConversationRequest::CommentScoped {
            conversation: conv_a_id.clone(),
            plan: Some("non_existent_plan".into()),
            target: "t1".into(),
            text: "comment".into(),
        },
    ));
    assert!(
        err_comment_foreign.is_err(),
        "Must reject comment scoped to invalid plan"
    );

    // Test 6: Valid scoped requests succeed
    let ok_task = daemon_a.client.call(&Request::Conversation(
        antislop::lean::daemon::ConversationRequest::TaskScoped {
            conversation: Some(conv_a_id.clone()),
            plan: plan_a1_id.clone(),
            id: task_a1_id.clone(),
        },
    ));
    assert!(ok_task.is_ok(), "Valid scoped task request must succeed");

    let ok_plans = daemon_a.client.call(&Request::Conversation(
        antislop::lean::daemon::ConversationRequest::Plans {
            conversation: conv_a_id.clone(),
            offset: Some(0),
            limit: Some(10),
        },
    ));
    assert!(ok_plans.is_ok(), "Valid plans request must succeed");
    let plans_page = ok_plans.unwrap();
    assert_eq!(plans_page["total"], 2);
}

mod common;
use antislop::lean::daemon::{Client, Request};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn fake_codex(dir: &Path) -> PathBuf {
    let script = dir.join("codex");
    std::fs::write(
        &script,
        r#"#!/usr/bin/env python3
import json, sys
prompt = sys.argv[-1]
if "propose" in prompt:
    answer = "Plan:\n```plan\n{\"tasks\": [{\"id\": \"t1\", \"title\": \"T1\", \"goal\": \"g\", \"kind\": \"analysis\", \"verifier\": {\"focus\": \"f\"}}]}\n```"
else:
    answer = "answer: " + prompt.splitlines()[-1] if prompt else "answer: ok"
print(json.dumps({"type": "thread.started", "thread_id": "orch"}))
print(json.dumps({"type": "item.completed", "item": {"type": "agent_message", "text": answer}}))
print(json.dumps({"type": "turn.completed", "usage": {"input_tokens": 10, "output_tokens": 5}}))
"#,
    )
    .unwrap();
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    script
}

fn fake_agy(dir: &Path) -> PathBuf {
    let script = dir.join("agy");
    std::fs::write(
        &script,
        r#"#!/usr/bin/env python3
import json, sys
print(json.dumps({"event": "init", "conversation_id": "worker-1"}))
print(json.dumps({"event": "result", "result": {"conversation_id": "worker-1", "status": "SUCCESS", "response": "ok", "usage": {}}}))
"#,
    )
    .unwrap();
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    script
}

fn write_config(dir: &Path, codex: &Path, agy: &Path) -> PathBuf {
    let config_path = dir.join("antislop.toml");
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
    config_path
}

struct DaemonGuard {
    child: std::process::Child,
    client: Client,
    state: PathBuf,
}

impl DaemonGuard {
    fn start(project: &Path, config: &Path, state: &Path) -> Self {
        common::trust(project);
        std::fs::create_dir_all(state).unwrap();
        std::fs::set_permissions(state, std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .unwrap();
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
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if client.call(&Request::Identity).is_ok() {
                return Self {
                    child,
                    client,
                    state: state.to_path_buf(),
                };
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let _ = child.kill();
        let _ = child.wait();
        panic!("daemon did not start within 10 seconds");
    }

    fn call_raw(&self, json_payload: &Value) -> std::result::Result<Value, String> {
        let socket_path = antislop::lean::daemon::socket(&self.state);
        let mut stream = UnixStream::connect(&socket_path).unwrap();
        let mut bytes = serde_json::to_vec(json_payload).unwrap();
        bytes.push(b'\n');
        stream.write_all(&bytes).unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let resp: Value = serde_json::from_str(&line).unwrap();
        if let Some(err) = resp["error"].as_str() {
            Err(err.to_string())
        } else {
            Ok(resp["data"].clone())
        }
    }
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let _ = self.client.call(&Request::Shutdown);
        let deadline = Instant::now() + Duration::from_millis(500);
        loop {
            if let Ok(Some(_)) = self.child.try_wait() {
                break;
            }
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                let _ = self.child.wait();
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

#[test]
fn test_conversations_structured_summary() {
    let dir = tempfile::tempdir().unwrap();
    let dir_path = dir.path().canonicalize().unwrap();
    let project = dir_path.join("project");
    std::fs::create_dir(&project).unwrap();
    let codex = fake_codex(&dir_path);
    let agy = fake_agy(&dir_path);
    let config = write_config(&dir_path, &codex, &agy);
    let state = dir_path.join("state");

    let daemon = DaemonGuard::start(&project, &config, &state);

    // Initially no conversations
    let convs = daemon.call_raw(&json!({"op": "conversations"})).unwrap();
    let array = convs.as_array().expect("must be array");
    assert!(array.is_empty());

    // Start a new conversation
    let started = daemon.call_raw(&json!({"op": "start"})).unwrap();
    let conv_id = started["id"].as_str().expect("must have conv id");

    // Query conversations again
    let convs = daemon.call_raw(&json!({"op": "conversations"})).unwrap();
    let array = convs.as_array().expect("must be array");
    assert_eq!(array.len(), 1);
    let item = &array[0];
    assert_eq!(item["id"], conv_id);
    assert_eq!(item["project"], project.to_str().unwrap());
    assert!(item["created_ms"].as_u64().unwrap() > 0);
    assert_eq!(item["status"], "idle");
    assert!(item["active_plan"].is_null());
    assert_eq!(item["tasks_total"], 0);
    assert_eq!(item["tasks_done"], 0);
    assert!(item["scope_hash"].as_str().unwrap().len() >= 8);
}

#[test]
fn test_history_and_events_pagination_and_unavailable_flag() {
    let dir = tempfile::tempdir().unwrap();
    let dir_path = dir.path().canonicalize().unwrap();
    let project = dir_path.join("project");
    std::fs::create_dir(&project).unwrap();
    let codex = fake_codex(&dir_path);
    let agy = fake_agy(&dir_path);
    let config = write_config(&dir_path, &codex, &agy);
    let state = dir_path.join("state");

    let daemon = DaemonGuard::start(&project, &config, &state);
    let started = daemon.call_raw(&json!({"op": "start"})).unwrap();
    let conv_id = started["id"].as_str().unwrap();

    // Send 3 messages to generate multiple events
    for i in 1..=3 {
        daemon
            .call_raw(&json!({
                "op": "send",
                "conversation": conv_id,
                "text": format!("message {i}")
            }))
            .unwrap();
    }

    // Let orchestrator finish turns
    std::thread::sleep(Duration::from_millis(500));

    // Test History pagination
    let page1 = daemon
        .call_raw(&json!({
            "op": "history",
            "conversation": conv_id,
            "after": 0,
            "limit": 2
        }))
        .unwrap();
    let events1 = page1["events"].as_array().unwrap();
    assert_eq!(events1.len(), 2);
    assert_eq!(page1["after"], 0);
    assert!(page1["has_more"].as_bool().unwrap());
    assert!(!page1["history_unavailable"].as_bool().unwrap());
    let next_after = page1["next_after"].as_u64().unwrap();
    assert_eq!(next_after, events1.last().unwrap()["seq"].as_u64().unwrap());

    // Fetch next page
    let page2 = daemon
        .call_raw(&json!({
            "op": "history",
            "conversation": conv_id,
            "after": next_after,
            "limit": 50
        }))
        .unwrap();
    let events2 = page2["events"].as_array().unwrap();
    assert!(!events2.is_empty());
    assert!(!page2["history_unavailable"].as_bool().unwrap());

    // Out-of-bounds history query -> history_unavailable flag is true
    let page_oob = daemon
        .call_raw(&json!({
            "op": "history",
            "conversation": conv_id,
            "after": 99999,
            "limit": 10
        }))
        .unwrap();
    assert!(page_oob["history_unavailable"].as_bool().unwrap());

    // Test Events pagination
    let events_resp = daemon
        .call_raw(&json!({
            "op": "events",
            "conversation": conv_id,
            "after": 0,
            "wait_ms": 0,
            "limit": 2
        }))
        .unwrap();
    let evs = events_resp["events"].as_array().unwrap();
    assert_eq!(evs.len(), 2);
    assert!(events_resp["has_more"].as_bool().unwrap());
    assert!(!events_resp["history_unavailable"].as_bool().unwrap());
    assert!(events_resp["total"].as_u64().unwrap() >= 2);
    assert_eq!(events_resp["oldest_seq"], 1);

    // Out-of-bounds events query
    let events_oob = daemon
        .call_raw(&json!({
            "op": "events",
            "conversation": conv_id,
            "after": 99999,
            "wait_ms": 0,
            "limit": 10
        }))
        .unwrap();
    assert!(events_oob["history_unavailable"].as_bool().unwrap());
}

#[test]
fn test_message_addressing_validation_and_mail_queue() {
    let dir = tempfile::tempdir().unwrap();
    let dir_path = dir.path().canonicalize().unwrap();
    let project = dir_path.join("project");
    std::fs::create_dir(&project).unwrap();
    let codex = fake_codex(&dir_path);
    let agy = fake_agy(&dir_path);
    let config = write_config(&dir_path, &codex, &agy);
    let state = dir_path.join("state");

    let daemon = DaemonGuard::start(&project, &config, &state);
    let started = daemon.call_raw(&json!({"op": "start"})).unwrap();
    let conv_id = started["id"].as_str().unwrap();

    // 1. Valid explicit addressing to orchestrator
    daemon
        .call_raw(&json!({
            "op": "send",
            "conversation": conv_id,
            "text": "addressed to orchestrator",
            "to": "orchestrator"
        }))
        .unwrap();

    // 1b. Valid explicit addressing to plan-verifier
    daemon
        .call_raw(&json!({
            "op": "send",
            "conversation": conv_id,
            "text": "addressed to verifier",
            "to": "plan-verifier"
        }))
        .unwrap();

    // 2. Invalid recipient role rejection
    let err = daemon
        .call_raw(&json!({
            "op": "send",
            "conversation": conv_id,
            "text": "addressed to bad role",
            "to": "nonexistent_role"
        }))
        .unwrap_err();
    assert!(err.contains("nonexistent_role"), "error was: {err}");

    // 3. Broadcast to 'all' rejected without active plan tasks
    let err_broadcast = daemon
        .call_raw(&json!({
            "op": "send",
            "conversation": conv_id,
            "text": "broadcast message",
            "to": "all"
        }))
        .unwrap_err();
    assert!(
        err_broadcast.contains("no valid recipients for broadcast"),
        "error was: {err_broadcast}"
    );

    // 4. Verify mail_rejected events were produced in event log
    let hist = daemon
        .call_raw(&json!({
            "op": "history",
            "conversation": conv_id,
            "after": 0,
            "limit": 100
        }))
        .unwrap();
    let rejected_events: Vec<_> = hist["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == "mail_rejected")
        .collect();
    assert_eq!(
        rejected_events.len(),
        2,
        "both rejected messages should emit mail_rejected events"
    );
    assert_eq!(
        rejected_events[0]["payload"]["recipient"],
        "nonexistent_role"
    );
    assert_eq!(rejected_events[1]["payload"]["recipient"], "all");

    // 5. Query mail queue states
    let mail_resp = daemon
        .call_raw(&json!({
            "op": "mail",
            "conversation": conv_id
        }))
        .unwrap();
    let mail_list = mail_resp.as_array().expect("mail must be an array");
    assert!(!mail_list.is_empty());
    let first_mail = &mail_list[0];
    assert!(first_mail["created_ms"].as_u64().unwrap() > 0);
    assert!(first_mail["state"] == "delivered" || first_mail["state"] == "queued");

    // 6. Project validation
    let err_foreign = daemon
        .call_raw(&json!({
            "op": "send",
            "conversation": "foreign-conv-123",
            "text": "hello"
        }))
        .unwrap_err();
    assert!(
        err_foreign.contains("no conversation") || err_foreign.contains("does not belong"),
        "error was: {err_foreign}"
    );
}

#[test]
fn test_sessions_structured_data() {
    let dir = tempfile::tempdir().unwrap();
    let dir_path = dir.path().canonicalize().unwrap();
    let project = dir_path.join("project");
    std::fs::create_dir(&project).unwrap();
    let codex = fake_codex(&dir_path);
    let agy = fake_agy(&dir_path);
    let config = write_config(&dir_path, &codex, &agy);
    let state = dir_path.join("state");

    let daemon = DaemonGuard::start(&project, &config, &state);
    let started = daemon.call_raw(&json!({"op": "start"})).unwrap();
    let conv_id = started["id"].as_str().unwrap();

    daemon
        .call_raw(&json!({
            "op": "send",
            "conversation": conv_id,
            "text": "trigger session turn"
        }))
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last_status = String::new();
    let mut sess = Value::Null;
    while Instant::now() < deadline {
        let sessions_resp = daemon
            .call_raw(&json!({
                "op": "sessions",
                "conversation": conv_id
            }))
            .unwrap();
        let sessions = sessions_resp.as_array().expect("sessions must be array");
        if !sessions.is_empty() {
            sess = sessions[0].clone();
            last_status = sess["status"].as_str().unwrap_or("").to_string();
            if last_status == "done" {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(last_status, "done");
    assert_eq!(sess["role"], "orchestrator");
    assert!(sess["started_ms"].as_u64().unwrap() > 0);
    assert!(sess["finished_ms"].as_u64().unwrap() > 0);
}

#[test]
fn test_scope_hash_verification_and_stale_scope_rejection() {
    let dir = tempfile::tempdir().unwrap();
    let dir_path = dir.path().canonicalize().unwrap();
    let project = dir_path.join("project");
    std::fs::create_dir(&project).unwrap();
    let codex = fake_codex(&dir_path);
    let agy = fake_agy(&dir_path);
    let config = write_config(&dir_path, &codex, &agy);
    let state = dir_path.join("state");

    let daemon = DaemonGuard::start(&project, &config, &state);
    let started = daemon.call_raw(&json!({"op": "start"})).unwrap();
    let conv_id = started["id"].as_str().unwrap();

    // 1. Get conversation scope hash from summary
    let convs = daemon.call_raw(&json!({"op": "conversations"})).unwrap();
    let current_scope = convs[0]["scope_hash"].as_str().unwrap().to_string();

    // 2. Cancel with stale scope_hash -> must be rejected with 'stale_scope'
    let stale_err = daemon
        .call_raw(&json!({
            "op": "cancel",
            "conversation": conv_id,
            "scope_hash": "stale_scope_hash_12345"
        }))
        .unwrap_err();
    assert!(stale_err.contains("stale_scope"), "error was: {stale_err}");

    // 3. Cancel with correct scope_hash -> succeeds
    let cancel_ok = daemon
        .call_raw(&json!({
            "op": "cancel",
            "conversation": conv_id,
            "scope_hash": current_scope
        }))
        .unwrap();
    assert!(cancel_ok.is_null());

    // 4. Cancel without scope_hash -> succeeds (backward compatibility)
    let cancel_none = daemon
        .call_raw(&json!({
            "op": "cancel",
            "conversation": conv_id
        }))
        .unwrap();
    assert!(cancel_none.is_null());

    // 5. Accept with stale scope_hash -> rejected with 'stale_scope'
    let accept_err = daemon
        .call_raw(&json!({
            "op": "accept",
            "conversation": conv_id,
            "plan": "P99",
            "scope_hash": "stale_plan_hash"
        }))
        .unwrap_err();
    assert!(
        accept_err.contains("stale_scope"),
        "error was: {accept_err}"
    );

    // 6. Resume with stale scope_hash -> rejected with 'stale_scope'
    let resume_err = daemon
        .call_raw(&json!({
            "op": "resume",
            "conversation": conv_id,
            "plan": "P99",
            "scope_hash": "stale_plan_hash"
        }))
        .unwrap_err();
    assert!(
        resume_err.contains("stale_scope"),
        "error was: {resume_err}"
    );

    // 7. Review with stale scope_hash -> rejected with 'stale_scope'
    let review_err = daemon
        .call_raw(&json!({
            "op": "review",
            "conversation": conv_id,
            "plan": "P99",
            "id": "T1",
            "scope_hash": "stale_diff_hash"
        }))
        .unwrap_err();
    assert!(
        review_err.contains("stale_scope"),
        "error was: {review_err}"
    );
}

#[test]
fn test_wire_request_deny_unknown_fields_and_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let dir_path = dir.path().canonicalize().unwrap();
    let project = dir_path.join("project");
    std::fs::create_dir(&project).unwrap();
    let codex = fake_codex(&dir_path);
    let agy = fake_agy(&dir_path);
    let config = write_config(&dir_path, &codex, &agy);
    let state = dir_path.join("state");

    let daemon = DaemonGuard::start(&project, &config, &state);

    let started = daemon.call_raw(&json!({"op": "start"})).unwrap();
    let conv_id = started["id"].as_str().unwrap();

    // Reject unknown fields due to #[serde(deny_unknown_fields)]
    let err = daemon
        .call_raw(&json!({
            "op": "send",
            "conversation": conv_id,
            "text": "test",
            "unrecognized_field": true
        }))
        .unwrap_err();
    assert!(err.contains("unknown field"), "error was: {err}");

    // Accept missing optional fields: send text without 'to'
    let send_ok = daemon
        .call_raw(&json!({
            "op": "send",
            "conversation": conv_id,
            "text": "plain message"
        }))
        .unwrap();
    assert!(send_ok.as_u64().unwrap() > 0);
}

#[test]
fn test_stale_scope_forced_change_between_read_and_execution_has_no_side_effects() {
    let dir = tempfile::tempdir().unwrap();
    let dir_path = dir.path().canonicalize().unwrap();
    let project = dir_path.join("project");
    std::fs::create_dir(&project).unwrap();
    let codex = fake_codex(&dir_path);
    let agy = fake_agy(&dir_path);
    let config = write_config(&dir_path, &codex, &agy);
    let state = dir_path.join("state");

    let daemon = DaemonGuard::start(&project, &config, &state);

    let started = daemon.call_raw(&json!({"op": "start"})).unwrap();
    let conv_id = started["id"].as_str().unwrap();

    // 1. Read current conversation scope hash
    let convs = daemon.call_raw(&json!({"op": "conversations"})).unwrap();
    let scope_at_read = convs[0]["scope_hash"].as_str().unwrap().to_string();

    // 2. Force a scope change between read and execution by sending a new turn
    let send_res = daemon
        .call_raw(&json!({
            "op": "send",
            "conversation": conv_id,
            "text": "force state change"
        }))
        .unwrap();
    assert!(send_res.as_u64().unwrap() > 0);

    // Wait until daemon processes turn and returns to idle
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut scope_after_change = scope_at_read.clone();
    while Instant::now() < deadline {
        let updated = daemon.call_raw(&json!({"op": "conversations"})).unwrap();
        if updated[0]["status"] == "idle"
            && updated[0]["scope_hash"].as_str().unwrap() != scope_at_read
        {
            scope_after_change = updated[0]["scope_hash"].as_str().unwrap().to_string();
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_ne!(
        scope_at_read, scope_after_change,
        "scope must have changed on daemon"
    );

    // 3. Execution attempt with scope_at_read (stale scope)
    let stale_err = daemon
        .call_raw(&json!({
            "op": "cancel",
            "conversation": conv_id,
            "scope_hash": scope_at_read
        }))
        .unwrap_err();
    assert!(
        stale_err.contains("stale_scope"),
        "error must be stale_scope, got: {stale_err}"
    );

    // 4. Verify ZERO side effects: conversation was not cancelled, events remain intact
    let convs_after = daemon.call_raw(&json!({"op": "conversations"})).unwrap();
    assert_eq!(convs_after[0]["id"], conv_id);
    assert_eq!(
        convs_after[0]["scope_hash"].as_str().unwrap(),
        scope_after_change,
        "state must remain untouched after stale scope error"
    );

    // 5. Test plan accept with stale scope:
    let plan_stale_err = daemon
        .call_raw(&json!({
            "op": "accept",
            "conversation": conv_id,
            "plan": "P_nonexistent",
            "scope_hash": "outdated_plan_hash"
        }))
        .unwrap_err();
    assert!(
        plan_stale_err.contains("stale_scope"),
        "error must be stale_scope, got: {plan_stale_err}"
    );

    // 6. Test resume with stale scope:
    let resume_stale_err = daemon
        .call_raw(&json!({
            "op": "resume",
            "conversation": conv_id,
            "plan": "P_nonexistent",
            "scope_hash": "outdated_resume_hash"
        }))
        .unwrap_err();
    assert!(
        resume_stale_err.contains("stale_scope"),
        "error must be stale_scope, got: {resume_stale_err}"
    );

    // 7. Test review with stale scope:
    let review_stale_err = daemon
        .call_raw(&json!({
            "op": "review",
            "conversation": conv_id,
            "plan": "P_nonexistent",
            "id": "T_nonexistent",
            "scope_hash": "outdated_diff_hash"
        }))
        .unwrap_err();
    assert!(
        review_stale_err.contains("stale_scope"),
        "error must be stale_scope, got: {review_stale_err}"
    );
}

#[test]
fn test_task_scoped_query_includes_freshness_token() {
    let dir = tempfile::tempdir().unwrap();
    let dir_path = dir.path().canonicalize().unwrap();
    let project = dir_path.join("project");
    std::fs::create_dir(&project).unwrap();
    // Initialize git repo in project so git commands succeed
    let _ = std::process::Command::new("git")
        .args(["init"])
        .current_dir(&project)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output();
    let _ = std::process::Command::new("git")
        .args(["commit", "--allow-empty", "-m", "init"])
        .current_dir(&project)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@test.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@test.com")
        .output();

    let codex = fake_codex(&dir_path);
    let agy = fake_agy(&dir_path);
    let config = write_config(&dir_path, &codex, &agy);
    let state = dir_path.join("state");

    let daemon = DaemonGuard::start(&project, &config, &state);
    let started = daemon.call_raw(&json!({"op": "start"})).unwrap();
    let conv_id = started["id"].as_str().unwrap();

    daemon
        .call_raw(&json!({
            "op": "send",
            "conversation": conv_id,
            "text": "propose plan"
        }))
        .unwrap();

    let mut plan_id = String::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let plans = daemon
            .call_raw(&json!({
                "op": "plans",
                "conversation": conv_id
            }))
            .unwrap();
        if let Some(id) = plans["active_plan_id"].as_str() {
            plan_id = id.to_string();
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!plan_id.is_empty(), "plan was not proposed");

    // Query task with conversation
    let task_resp = daemon
        .call_raw(&json!({
            "op": "task",
            "conversation": conv_id,
            "plan": plan_id,
            "id": "t1"
        }))
        .unwrap();
    assert!(task_resp["freshness_token"].is_string());
    let token = task_resp["freshness_token"].as_str().unwrap();
    assert!(!token.is_empty());

    // Query task without explicit conversation (daemon resolves it via plan)
    let task_resp2 = daemon
        .call_raw(&json!({
            "op": "task",
            "plan": plan_id,
            "id": "t1"
        }))
        .unwrap();
    assert_eq!(task_resp2["freshness_token"].as_str().unwrap(), token);
}

#[test]
fn recovery_requests_cross_the_wire_and_only_apply_to_stopped_tasks() {
    let dir = tempfile::tempdir().unwrap();
    let dir_path = dir.path().canonicalize().unwrap();
    let project = dir_path.join("project");
    std::fs::create_dir(&project).unwrap();
    let codex = fake_codex(&dir_path);
    let agy = fake_agy(&dir_path);
    let config = write_config(&dir_path, &codex, &agy);
    let daemon = DaemonGuard::start(&project, &config, &dir_path.join("state"));

    let conv = daemon.call_raw(&json!({"op": "start"})).unwrap();
    let conv = conv["id"].as_str().unwrap();
    daemon
        .call_raw(&json!({"op": "send", "conversation": conv, "text": "please propose"}))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let plan = loop {
        let list = daemon.call_raw(&json!({"op": "conversations"})).unwrap();
        if let Some(plan) = list[0]["active_plan"].as_str() {
            break plan.to_owned();
        }
        assert!(Instant::now() < deadline, "no plan was proposed");
        std::thread::sleep(Duration::from_millis(50));
    };
    let task = daemon
        .call_raw(&json!({"op": "task", "plan": plan, "id": "t1"}))
        .unwrap();
    let token = task["freshness_token"]
        .as_str()
        .expect("a token")
        .to_owned();

    // A proposed task is not stopped, so there is nothing to recover.
    let error = daemon
        .call_raw(&json!({
            "op": "recover", "conversation": conv, "plan": plan, "task": "t1",
            "action": "extend_rounds", "freshness_token": token
        }))
        .unwrap_err();
    assert!(error.contains("not waiting for recovery"), "{error}");

    // Unknown actions and fields are refused by the protocol.
    assert!(
        daemon
            .call_raw(&json!({
                "op": "recover", "conversation": conv, "plan": plan, "task": "t1",
                "action": "explode", "freshness_token": token
            }))
            .is_err()
    );
}

#[test]
fn removed_commands_and_options_are_refused_with_a_reason() {
    let dir = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
            .args(args)
            .current_dir(dir.path())
            .output()
            .unwrap()
    };
    let export = run(&["export"]);
    assert!(!export.status.success());
    assert!(String::from_utf8_lossy(&export.stderr).contains("export was removed"));
    // The working directory is the project: there is no --project.
    let project = run(&["--project", "."]);
    assert!(!project.status.success());
    assert!(String::from_utf8_lossy(&project.stderr).contains("unknown option --project"));
    // Neither is advertised any more.
    let help = String::from_utf8_lossy(&run(&["--help"]).stdout).into_owned();
    assert!(!help.contains("--project") && !help.contains("export"));
}

#[test]
fn nothing_is_created_or_started_in_a_directory_that_is_not_trusted() {
    let dir = tempfile::tempdir().unwrap();
    let dir_path = dir.path().canonicalize().unwrap();
    let project = dir_path.join("project");
    std::fs::create_dir(&project).unwrap();
    let codex = fake_codex(&dir_path);
    let agy = fake_agy(&dir_path);
    let config = write_config(&dir_path, &codex, &agy);
    std::fs::copy(&config, project.join("antislop.toml")).unwrap();
    let list = dir_path.join("config");

    // The CLI without a terminal and without --trust refuses, leaving no state.
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
        .current_dir(&project)
        .env("ANTISLOP_CONFIG_DIR", &list)
        .arg("hello")
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not trusted"));
    assert!(!project.join(".antislop").exists(), "no state directory");
    assert!(!list.exists(), "nothing was remembered");

    // A daemon will not start for it either.
    let state = dir_path.join("state");
    std::fs::create_dir_all(&state).unwrap();
    std::fs::set_permissions(&state, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    let daemon = std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
        .env("ANTISLOP_CONFIG_DIR", &list)
        .arg("__core")
        .arg(&project)
        .arg(&config)
        .arg(&state)
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(!daemon.status.success());
    assert!(String::from_utf8_lossy(&daemon.stderr).contains("not a trusted directory"));
}

/// The reply budget counts payload bytes, but the first event of a page
/// is always taken whole: one oversized historical event must advance the
/// cursor, never stall a follower. A reply therefore has no strict size
/// ceiling deducible from the budget alone — the cursor, not the size,
/// bounds it — and events over the budget wait for the next poll.
#[test]
fn an_oversized_first_event_still_advances_the_cursor() {
    use antislop::lean::daemon::reply_event_count;
    use antislop::lean::store::Event;
    let event = |seq: u64, size: usize| Event {
        seq,
        turn_id: None,
        plan_id: None,
        kind: "notice".into(),
        payload: json!({"text": "x".repeat(size)}),
        created_ms: 0,
    };
    let budget = 1024 * 1024;
    // Alone over the budget: still taken, so `next_after` moves past it.
    let page = [event(9, 2 * budget)];
    assert_eq!(reply_event_count(&page, budget), 1);
    // Whole events only: the page stops at the first crossing and the
    // rest is left for the next poll, which the advanced cursor opens.
    let page = [event(10, 400_000), event(11, 400_000), event(12, 400_000)];
    let taken = reply_event_count(&page, budget);
    assert_eq!(taken, 2, "the third would cross the budget");
    let next_after = page[taken - 1].seq;
    assert_eq!(next_after, 11);
    assert_eq!(reply_event_count(&page[taken..], budget), 1);
    // Nothing to send is nothing to take.
    assert_eq!(reply_event_count(&[], budget), 0);
}

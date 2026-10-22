mod support;
use antislop::{
    config::Config,
    lean::{Core, RecoveryAction, TaskRecoveryRequest},
};
use serde_json::json;
use std::path::Path;
use std::time::{Duration, Instant};

/// Orchestrator: proposes a plan named by the message. Agents: a worker on a
/// task titled "Fail" dies without a result; every other turn succeeds.
fn fake_clis(dir: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let codex = dir.join("codex");
    std::fs::write(
        &codex,
        r#"#!/usr/bin/env python3
import json, sys
prompt = sys.argv[-1]
def task(id, title, depends=(), writes=False):
    return {"id": id, "title": title, "goal": "g", "depends_on": list(depends),
            "writes": writes, "scope": ["."], "kind": "implementation" if writes else "analysis",
            "verifier": {"focus": "f"}}
plans = {
  "chain": [task("t1", "Fail"), task("t2", "Second", ["t1"]), task("t3", "Third", ["t2"])],
  "first": [task("a1", "First plan")],
  "second": [task("b1", "Second plan")],
  "write_chain": [task("w1", "Write file", writes=True), task("w2", "Follow up", ["w1"])],
  "manual_fix": [task("m1", "Manual fix", writes=True)],
}
name = next((name for name in sorted(plans, key=len, reverse=True) if name in prompt.splitlines()[-1]), None)
answer = "Plan.\n```plan\n" + json.dumps({"tasks": plans[name]}) + "\n```" if name else "ok"
# The user has decided (supervised) or nobody watches (autonomous): unblock t1,
# which only works once the agent is logged in.
if "The user is not watching" in prompt or prompt.splitlines()[-1] == "retry t1":
    import os
    open(os.path.join(os.path.dirname(sys.argv[0]), "logged-in"), "w").close()
    answer = "Retrying.\n```recover\n" + json.dumps({"actions": [
        {"task": "t1", "action": "retry_with_guidance", "message": "logged in now"}]}) + "\n```"
print(json.dumps({"type": "thread.started", "thread_id": "orch"}))
print(json.dumps({"type": "item.completed", "item": {"type": "agent_message", "text": answer}}))
print(json.dumps({"type": "turn.completed", "usage": {}}))
"#,
    )
    .unwrap();
    let agy = dir.join("agy");
    std::fs::write(
        &agy,
        r#"#!/usr/bin/env python3
import json, sys
prompt = sys.argv[-1]
import os
print(json.dumps({"event": "init", "conversation_id": "s"}), flush=True)
here = os.path.dirname(sys.argv[0])
with open(os.path.join(here, "agy.log"), "a") as log:
    log.write(json.dumps(sys.argv[1:]) + "\n")
worker = "Task: Fail" in prompt or "was interrupted" in prompt
if worker and "You verify" not in prompt and "You audit" not in prompt \
        and not os.path.exists(os.path.join(here, "logged-in")):
    print("Error: not logged in, run `agy login`", file=sys.stderr)
    sys.exit(1)
if "Task: Write file" in prompt and "You verify" not in prompt and "You audit" not in prompt:
    if os.path.exists(os.path.join(here, "concurrent-user-edit")):
        with open(os.path.join(here, "project", "hello.txt"), "w") as primary:
            primary.write("user edit\n")
    with open("hello.txt", "w") as f:
        f.write("from worker\n")
if "Task: Manual fix" in prompt and "You verify" not in prompt and "You audit" not in prompt:
    with open("hello.txt", "w") as f:
        f.write("from worker manual\n")
def cover(answer):
    import re
    ids = re.findall(r"^- ([RCV][0-9]+): ", prompt, re.M)
    if not ids or "```verdict" not in answer:
        return answer
    head, body = answer.rsplit("```verdict", 1)
    data = json.loads(body.split("```")[0])
    data["coverage"] = {i: {"status": "ok", "note": "checked"} for i in ids}
    return head + "```verdict\n" + json.dumps(data) + "\n```"
REVIEW = ("You review a plan", "new plan:", "You audit", "next plan:", "You verify")
answer = "```verdict\n{\"approved\": true, \"findings\": []}\n```" if any(m in prompt for m in REVIEW) else "report"
if "Automatic integration conflict" in prompt:
    answer = "```integration_question\nKeep the concurrent user edit or apply the worker change?\n```"
answer = cover(answer)
print(json.dumps({"event": "result", "result": {"conversation_id": "s", "status": "SUCCESS", "response": answer, "usage": {}}}))
"#,
    )
    .unwrap();
    for script in [&codex, &agy] {
        support::install_ledger_protocol(script);
        support::install_task_options_protocol(script);
        std::fs::set_permissions(script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
    }
    (codex, agy)
}

fn open(dir: &Path) -> std::sync::Arc<Core> {
    open_in_mode(dir, "supervised")
}

fn open_in_mode(dir: &Path, mode: &str) -> std::sync::Arc<Core> {
    let (codex, agy) = fake_clis(dir);
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
    )
    .replace("required_gates = true", "required_gates = false");
    let text = text.replace("mode = \"supervised\"", &format!("mode = {mode:?}"));
    Core::open(&dir.join("state"), Config::from_toml(&text).unwrap()).unwrap()
}

fn git(dir: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn init_git_project(dir: &Path) -> std::path::PathBuf {
    let project = dir.join("project");
    std::fs::create_dir_all(&project).unwrap();
    git(&project, &["init", "-b", "main"]);
    std::fs::write(project.join("README"), "readme\n").unwrap();
    git(&project, &["add", "."]);
    git(&project, &["commit", "-m", "init"]);
    project.canonicalize().unwrap()
}

fn wait_idle(core: &Core, conversation: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !core.is_idle(conversation) {
        assert!(Instant::now() < deadline, "conversation never went idle");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn propose(core: &std::sync::Arc<Core>, conversation: &str, name: &str) -> String {
    core.send(conversation, name).unwrap();
    wait_idle(core, conversation);
    core.store.latest_plan(conversation).unwrap()[0]
        .plan_id
        .clone()
}

fn statuses(core: &Core, plan: &str) -> Vec<String> {
    core.store
        .plan(plan)
        .unwrap()
        .into_iter()
        .filter(|task| task.spec["kind"] != "options")
        .map(|task| task.status)
        .collect()
}

#[test]
fn a_failure_keeps_every_dependent_task_pending_and_says_why() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("project")).unwrap();
    let core = open(dir.path());
    let conversation = support::legacy_conversation(&core, &dir.path().join("project"))
        .unwrap()
        .id;
    let plan = propose(&core, &conversation, "chain");
    core.accept_plan(&conversation, &plan).unwrap();
    wait_idle(&core, &conversation);

    assert_eq!(statuses(&core, &plan), ["waiting", "queued", "queued"]);
    let waiting = &core.store.plan(&plan).unwrap()[0];
    let report = waiting.report.as_deref().unwrap();
    assert!(report.contains("not logged in"), "{report}");
}

#[test]
fn after_a_login_resuming_finishes_the_plan_in_the_same_sessions() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("project")).unwrap();
    let core = open(dir.path());
    let conversation = support::legacy_conversation(&core, &dir.path().join("project"))
        .unwrap()
        .id;
    let plan = propose(&core, &conversation, "chain");
    core.accept_plan(&conversation, &plan).unwrap();
    wait_idle(&core, &conversation);
    assert_eq!(statuses(&core, &plan), ["waiting", "queued", "queued"]);

    std::fs::write(dir.path().join("logged-in"), "").unwrap();
    core.resume_plan(&conversation, &plan).unwrap();
    wait_idle(&core, &conversation);
    assert_eq!(statuses(&core, &plan), ["approved", "approved", "approved"]);
    // The failed turn already had a session: the worker resumes it and is
    // told what happened instead of being handed the task anew.
    let log = std::fs::read_to_string(dir.path().join("agy.log")).unwrap();
    let resumed = log
        .lines()
        .find(|line| line.contains("was interrupted"))
        .expect("a resumed worker turn");
    assert!(resumed.contains("--conversation"), "{resumed}");
}

#[test]
fn accepting_runs_the_plan_the_user_saw_not_a_newer_one() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("project")).unwrap();
    let core = open(dir.path());
    let conversation = support::legacy_conversation(&core, &dir.path().join("project"))
        .unwrap()
        .id;
    let first = propose(&core, &conversation, "first");
    let second = propose(&core, &conversation, "second");
    assert_ne!(first, second);

    core.accept_plan(&conversation, &first).unwrap();
    wait_idle(&core, &conversation);
    assert_eq!(statuses(&core, &first), ["approved"]);
    assert_eq!(statuses(&core, &second), ["proposed"]);
    // The results the orchestrator receives are the accepted plan's.
    let messages = core.store.messages_after(&conversation, 0).unwrap();
    let results = messages
        .iter()
        .rfind(|message| message.author == "runtime")
        .unwrap();
    assert!(results.text.contains("First plan") && !results.text.contains("Second plan"));
    // A plan already run cannot be started again.
    assert!(core.accept_plan(&conversation, &first).is_err());
}

#[test]
fn legacy_supervised_tasks_require_ledger_extraction_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("project")).unwrap();
    let core = open(dir.path());
    let conversation = support::legacy_conversation(&core, &dir.path().join("project"))
        .unwrap()
        .id;
    let spec = json!({"id": "t1", "title": "T", "goal": "g", "kind": "analysis",
                      "verifier": {"focus": "f"}});
    let mut second = spec.clone();
    second["id"] = json!("t2");
    second["depends_on"] = json!(["t1"]);
    let tasks = core.store.add_plan(&conversation, &[spec, second]).unwrap();
    core.store
        .queue_plan(&conversation, &tasks[0].plan_id)
        .unwrap();
    let mut working = tasks[0].clone();
    working.status = "working".into();
    core.store.update_task(&working).unwrap();
    drop(core);

    let core = open(dir.path());
    wait_idle(&core, &conversation);
    let after = core.store.plan(&tasks[0].plan_id).unwrap();
    assert_eq!(statuses(&core, &tasks[0].plan_id), ["queued", "queued"]);
    assert!(after.iter().all(|task| task.round == 0));
    assert!(
        core.resume_plan(&conversation, &tasks[0].plan_id)
            .unwrap_err()
            .to_string()
            .contains("legacy plan")
    );
}

#[test]
fn legacy_autonomous_plan_cannot_resume_without_ledger_extraction() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("project")).unwrap();
    std::fs::write(dir.path().join("logged-in"), "").unwrap();
    let core = open_in_mode(dir.path(), "autonomous");
    let conversation = support::legacy_conversation(&core, &dir.path().join("project"))
        .unwrap()
        .id;
    let spec = json!({"id": "t1", "title": "T", "goal": "g", "kind": "analysis",
                      "verifier": {"focus": "f"}});
    let tasks = core.store.add_plan(&conversation, &[spec]).unwrap();
    core.store
        .queue_plan(&conversation, &tasks[0].plan_id)
        .unwrap();
    let mut working = tasks[0].clone();
    working.status = "working".into();
    working.round = 1;
    core.store.update_task(&working).unwrap();
    drop(core);

    let core = open_in_mode(dir.path(), "autonomous");
    wait_idle(&core, &conversation);
    assert_eq!(statuses(&core, &tasks[0].plan_id), ["queued"]);
    assert!(
        core.resume_plan(&conversation, &tasks[0].plan_id)
            .unwrap_err()
            .to_string()
            .contains("legacy plan")
    );
}

#[test]
fn test_freshness_token_staleness_and_rejection() {
    let dir = tempfile::tempdir().unwrap();
    let project = init_git_project(dir.path());
    let core = open(dir.path());
    let conversation = support::legacy_conversation(&core, &project).unwrap().id;
    let spec = json!({
        "id": "t1",
        "title": "T1",
        "goal": "g",
        "kind": "analysis",
        "scope": ["README"],
        "verifier": {"focus": "f"}
    });
    let tasks = core.store.add_plan(&conversation, &[spec]).unwrap();
    let plan = &tasks[0].plan_id;
    let task_id = &tasks[0].id;

    // 1. Initial freshness token
    let token1 = core
        .task_freshness_token(&conversation, plan, task_id)
        .unwrap();
    assert_eq!(token1.len(), 64, "token should be sha256 hex string");

    // 2. Querying via core.task returns the same freshness token
    let task_obj = core.task(plan, task_id).unwrap();
    assert_eq!(task_obj.freshness_token.as_deref(), Some(token1.as_str()));

    // 3. Dirty working tree changes the token
    std::fs::write(project.join("README"), "modified content\n").unwrap();
    let token2 = core
        .task_freshness_token(&conversation, plan, task_id)
        .unwrap();
    assert_ne!(
        token1, token2,
        "freshness token must change when working tree is dirty"
    );

    // 4. Stale token rejection with zero side-effects
    let mut task_waiting = tasks[0].clone();
    task_waiting.status = "needs_user".into();
    core.store.update_task(&task_waiting).unwrap();

    let res = core.recover_task(TaskRecoveryRequest {
        conversation: conversation.clone(),
        plan: plan.clone(),
        task: task_id.clone(),
        action: RecoveryAction::CancelTask,
        consent_files: None,
        freshness_token: token1, // stale!
        message: None,
    });
    assert!(res.is_err());
    assert!(res.unwrap_err().to_string().contains("stale_token"));

    // Task remains in needs_user, not cancelled
    let current_task = core.task_unscoped(plan, task_id).unwrap();
    assert_eq!(current_task.status, "needs_user");
}

#[test]
fn test_blocked_task_prevents_premature_audit() {
    let dir = tempfile::tempdir().unwrap();
    let project = init_git_project(dir.path());
    let core = open(dir.path());
    let conversation = support::legacy_conversation(&core, &project).unwrap().id;
    let plan = propose(&core, &conversation, "chain");
    core.accept_plan(&conversation, &plan).unwrap();
    wait_idle(&core, &conversation);

    assert_eq!(statuses(&core, &plan), ["waiting", "queued", "queued"]);

    // Verify no audit event was emitted
    let events = core.store.events_since(&conversation, 0, 1000).unwrap();
    assert!(
        !events.iter().any(|e| e.kind == "plan_audit"),
        "blocked task must not trigger plan audit"
    );

    // Verify messages do not contain plan audit / report
    let messages = core.store.messages_after(&conversation, 0).unwrap();
    assert!(
        !messages
            .iter()
            .any(|m| m.text.contains("Plan Audit") || m.text.contains("All tasks approved")),
        "no final audit message should be sent while tasks are blocked"
    );

    // The orchestrator is told, once, which step is blocked and how to go on.
    let reports: Vec<_> = messages
        .iter()
        .filter(|m| m.author == "runtime" && m.text.contains("cannot go on"))
        .collect();
    assert_eq!(reports.len(), 1, "{messages:?}");
    let text = &reports[0].text;
    assert!(text.contains("is waiting after"), "{text}");
    assert!(text.contains("Waiting for dependencies; these tasks remain pending"));
    assert!(text.contains("/unblock"));
}

#[test]
fn test_recover_task_integrate_with_consent_and_dag_cascade() {
    let dir = tempfile::tempdir().unwrap();
    let project = init_git_project(dir.path());
    // Edit the primary file after the writer takes its base. Starting edits
    // are inherited by the writer; concurrent conflicting edits require consent.
    std::fs::write(dir.path().join("concurrent-user-edit"), "").unwrap();

    let core = open(dir.path());
    let conversation = support::legacy_conversation(&core, &project).unwrap().id;
    let plan = propose(&core, &conversation, "write_chain");
    core.accept_plan(&conversation, &plan).unwrap();
    wait_idle(&core, &conversation);

    // w1 stopped because of user edit, w2 was skipped
    assert_eq!(statuses(&core, &plan), ["needs_user", "queued"]);
    assert_eq!(
        std::fs::read_to_string(project.join("hello.txt")).unwrap(),
        "user edit\n"
    );

    let tasks = core.store.plan(&plan).unwrap();
    let w1_id = &tasks
        .iter()
        .find(|task| task.spec["id"] == "w1")
        .unwrap()
        .id;
    let token = core
        .task_freshness_token(&conversation, &plan, w1_id)
        .unwrap();

    // Consent scopes integration; it does not silently overwrite an unresolved
    // conflicting edit. Reconcile the chosen contents before retrying recovery.
    let error = core
        .recover_task(TaskRecoveryRequest {
            conversation: conversation.clone(),
            plan: plan.clone(),
            task: w1_id.clone(),
            action: RecoveryAction::IntegrateWithConsent,
            consent_files: Some(vec!["hello.txt".into()]),
            freshness_token: token,
            message: None,
        })
        .unwrap_err();
    assert!(
        error.to_string().contains("uncommitted edits remain"),
        "{error}"
    );
    assert_eq!(
        std::fs::read_to_string(project.join("hello.txt")).unwrap(),
        "user edit\n"
    );
    std::fs::write(project.join("hello.txt"), "from worker\n").unwrap();
    let token = core
        .task_freshness_token(&conversation, &plan, w1_id)
        .unwrap();
    let resp = core
        .recover_task(TaskRecoveryRequest {
            conversation: conversation.clone(),
            plan: plan.clone(),
            task: w1_id.clone(),
            action: RecoveryAction::IntegrateWithConsent,
            consent_files: Some(vec!["hello.txt".into()]),
            freshness_token: token,
            message: None,
        })
        .unwrap();

    assert_eq!(resp.status, "approved");
    assert_eq!(
        std::fs::read_to_string(project.join("hello.txt")).unwrap(),
        "from worker\n"
    );

    // Wait for the cascade: w2 was unblocked to queued, runs, and completes
    wait_idle(&core, &conversation);
    assert_eq!(statuses(&core, &plan), ["approved", "approved"]);
}

#[test]
fn preserving_a_branch_keeps_integration_and_dependents_pending() {
    let dir = tempfile::tempdir().unwrap();
    let project = init_git_project(dir.path());
    std::fs::write(dir.path().join("concurrent-user-edit"), "").unwrap();

    let core = open(dir.path());
    let conversation = support::legacy_conversation(&core, &project).unwrap().id;
    let plan = propose(&core, &conversation, "write_chain");
    core.accept_plan(&conversation, &plan).unwrap();
    wait_idle(&core, &conversation);

    assert_eq!(statuses(&core, &plan), ["needs_user", "queued"]);
    let tasks = core.store.plan(&plan).unwrap();
    let w1_id = &tasks
        .iter()
        .find(|task| task.spec["id"] == "w1")
        .unwrap()
        .id;
    let branch = format!("antislop/{}", &w1_id[..8]);
    let token = core
        .task_freshness_token(&conversation, &plan, w1_id)
        .unwrap();

    // Recover with KeepOnBranch
    let resp = core
        .recover_task(TaskRecoveryRequest {
            conversation: conversation.clone(),
            plan: plan.clone(),
            task: w1_id.clone(),
            action: RecoveryAction::KeepOnBranch,
            consent_files: None,
            freshness_token: token,
            message: None,
        })
        .unwrap();

    assert_eq!(resp.status, "needs_user");
    // Project file must NOT be overwritten
    assert_eq!(
        std::fs::read_to_string(project.join("hello.txt")).unwrap(),
        "user edit\n"
    );
    // Branch still exists and contains worker change
    assert_eq!(
        git(&project, &["show", &format!("{branch}:hello.txt")]),
        "from worker"
    );

    // DAG cascades and w2 completes
    wait_idle(&core, &conversation);
    assert_eq!(statuses(&core, &plan), ["needs_user", "queued"]);
}

#[test]
fn test_recover_task_verify_manual_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let project = init_git_project(dir.path());
    let core = open(dir.path());
    let conversation = support::legacy_conversation(&core, &project).unwrap().id;
    let plan = propose(&core, &conversation, "manual_fix");
    core.accept_plan(&conversation, &plan).unwrap();
    wait_idle(&core, &conversation);

    // Set task to needs_user
    let mut tasks = core.store.plan(&plan).unwrap();
    tasks[0].status = "needs_user".into();
    core.store.update_task(&tasks[0]).unwrap();

    // User makes a manual fix in the project
    std::fs::write(project.join("hello.txt"), "manual user fix\n").unwrap();
    let head_before = git(&project, &["rev-parse", "HEAD"]);

    let token = core
        .task_freshness_token(&conversation, &plan, &tasks[0].id)
        .unwrap();
    let resp = core
        .recover_task(TaskRecoveryRequest {
            conversation: conversation.clone(),
            plan: plan.clone(),
            task: tasks[0].id.clone(),
            action: RecoveryAction::VerifyManualSnapshot,
            consent_files: None,
            freshness_token: token,
            message: None,
        })
        .unwrap();

    assert_eq!(resp.status, "approved");
    let updated = core.task_unscoped(&plan, &tasks[0].id).unwrap();
    assert_eq!(updated.status, "approved");

    // Commit policy applies to manually verified work, too.
    let head_after = git(&project, &["rev-parse", "HEAD"]);
    assert_ne!(head_before, head_after);
    assert_eq!(
        git(&project, &["show", "HEAD:hello.txt"]),
        "manual user fix"
    );

    // Verify task result points to the snapshot commit
    assert!(updated.result.is_some());
    let snap_commit = updated.result.unwrap();
    assert_ne!(snap_commit, head_after);
    assert_eq!(
        git(&project, &["show", &format!("{snap_commit}:hello.txt")]),
        "manual user fix"
    );
}

#[test]
fn test_recover_task_retry_with_guidance() {
    let dir = tempfile::tempdir().unwrap();
    let project = init_git_project(dir.path());
    let core = open(dir.path());
    let conversation = support::legacy_conversation(&core, &project).unwrap().id;
    let plan = propose(&core, &conversation, "chain");
    core.accept_plan(&conversation, &plan).unwrap();
    wait_idle(&core, &conversation);
    assert_eq!(statuses(&core, &plan), ["waiting", "queued", "queued"]);

    let tasks = core.store.plan(&plan).unwrap();
    let t1_id = &tasks[0].id;
    let token = core
        .task_freshness_token(&conversation, &plan, t1_id)
        .unwrap();

    // Now enable login so worker succeeds on retry
    std::fs::write(dir.path().join("logged-in"), "").unwrap();

    let resp = core
        .recover_task(TaskRecoveryRequest {
            conversation: conversation.clone(),
            plan: plan.clone(),
            task: t1_id.clone(),
            action: RecoveryAction::RetryWithGuidance,
            consent_files: None,
            freshness_token: token,
            message: Some("retrying after login".into()),
        })
        .unwrap();
    assert_eq!(resp.status, "queued");

    wait_idle(&core, &conversation);
    assert_eq!(statuses(&core, &plan), ["approved", "approved", "approved"]);
}

#[test]
fn an_unperformed_task_cannot_be_approved_or_discarded() {
    let dir = tempfile::tempdir().unwrap();
    let project = init_git_project(dir.path());
    let core = open(dir.path());
    let conversation = support::legacy_conversation(&core, &project).unwrap().id;
    let plan = propose(&core, &conversation, "chain");
    core.accept_plan(&conversation, &plan).unwrap();
    wait_idle(&core, &conversation);
    for action in [RecoveryAction::ForceApprove, RecoveryAction::CancelTask] {
        let task = core.store.plan(&plan).unwrap()[0].clone();
        let error = core
            .recover_task(TaskRecoveryRequest {
                conversation: conversation.clone(),
                plan: plan.clone(),
                task: task.id.clone(),
                action,
                consent_files: None,
                freshness_token: core
                    .task_freshness_token(&conversation, &plan, &task.id)
                    .unwrap(),
                message: None,
            })
            .unwrap_err();
        assert!(!error.to_string().is_empty());
        assert_eq!(statuses(&core, &plan), ["waiting", "queued", "queued"]);
    }
}

#[test]
fn test_mechanical_audit_gate_prevents_false_success() {
    let dir = tempfile::tempdir().unwrap();
    let project = init_git_project(dir.path());
    let core = open(dir.path());
    let conversation = support::legacy_conversation(&core, &project).unwrap().id;
    let spec1 = json!({"id": "t1", "title": "Done", "goal": "g", "kind": "analysis", "verifier": {"focus": "f"}});
    let spec2 = json!({"id": "t2", "title": "Blocked", "goal": "g", "kind": "analysis", "verifier": {"focus": "f"}});
    let tasks = core.store.add_plan(&conversation, &[spec1, spec2]).unwrap();
    let plan = &tasks[0].plan_id;

    // t1 approved, t2 needs_user
    let mut t1 = tasks[0].clone();
    t1.status = "approved".into();
    core.store.update_task(&t1).unwrap();

    let mut t2 = tasks[1].clone();
    t2.status = "needs_user".into();
    core.store.update_task(&t2).unwrap();

    // Call plan_report directly (where fake agy returns approved: true)
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let report_output = core.plan_report(&conversation, plan, &cancel).unwrap();

    // The mechanical gate in plan_report must have overridden verdict.approved to false
    let events = core.store.events_since(&conversation, 0, 1000).unwrap();
    let audit_event = events
        .iter()
        .rfind(|e| e.kind == "audit")
        .expect("audit event");
    assert_eq!(
        audit_event.payload["approved"], false,
        "mechanical gate must override to false"
    );
    assert!(
        audit_event.payload["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["issue"]
                .as_str()
                .is_some_and(|issue| issue.contains("Blocked") || issue.contains("not approved"))),
        "findings must note unapproved tasks"
    );
    assert!(
        report_output.contains("not fully delivered"),
        "plan report text must reflect audit rejection"
    );
}

#[test]
fn supervised_orchestrator_unblocks_only_after_the_user_decides() {
    let dir = tempfile::tempdir().unwrap();
    let project = init_git_project(dir.path());
    let core = open(dir.path());
    let conversation = support::legacy_conversation(&core, &project).unwrap().id;
    let plan = propose(&core, &conversation, "chain");
    core.accept_plan(&conversation, &plan).unwrap();
    wait_idle(&core, &conversation);
    assert_eq!(statuses(&core, &plan), ["waiting", "queued", "queued"]);

    core.send(&conversation, "retry t1").unwrap();
    wait_idle(&core, &conversation);
    assert_eq!(statuses(&core, &plan), ["approved", "approved", "approved"]);
    let events = core.store.events_since(&conversation, 0, 1000).unwrap();
    assert!(events.iter().any(|e| {
        e.kind == "notice"
            && e.payload["text"]
                .as_str()
                .is_some_and(|text| text.contains("orchestrator unblocked t1"))
    }));
}

#[test]
fn autonomous_orchestrator_unblocks_a_blocked_task_by_itself() {
    let dir = tempfile::tempdir().unwrap();
    let project = init_git_project(dir.path());
    let core = open_in_mode(dir.path(), "autonomous");
    let conversation = support::legacy_conversation(&core, &project).unwrap().id;
    core.send(&conversation, "chain").unwrap();
    wait_idle(&core, &conversation);
    let plan = core.store.latest_plan(&conversation).unwrap()[0]
        .plan_id
        .clone();
    assert_eq!(statuses(&core, &plan), ["approved", "approved", "approved"]);
}

/// Recovery announcements share the card's byte limits: the `task_recovery`
/// event carries the bounded line, while the whole reason stays readable —
/// guidance reaches the worker as mail in full, and a force-approve note
/// stays whole in the task's report.
#[test]
fn recovery_events_carry_bounded_details_and_keep_the_whole_reason() {
    let limit = antislop::lean::tasks::DETAIL_LIMIT;
    let dir = tempfile::tempdir().unwrap();
    let project = init_git_project(dir.path());
    let core = open(dir.path());
    let conversation = support::legacy_conversation(&core, &project).unwrap().id;
    let plan = propose(&core, &conversation, "chain");
    core.accept_plan(&conversation, &plan).unwrap();
    wait_idle(&core, &conversation);
    assert_eq!(statuses(&core, &plan), ["waiting", "queued", "queued"]);

    let t1_id = core.store.plan(&plan).unwrap()[0].id.clone();
    let token = core
        .task_freshness_token(&conversation, &plan, &t1_id)
        .unwrap();
    let guidance = format!("{} — tail", "g".repeat(4 * limit));
    std::fs::write(dir.path().join("logged-in"), "").unwrap();
    core.recover_task(TaskRecoveryRequest {
        conversation: conversation.clone(),
        plan: plan.clone(),
        task: t1_id.clone(),
        action: RecoveryAction::RetryWithGuidance,
        consent_files: None,
        freshness_token: token,
        message: Some(guidance.clone()),
    })
    .unwrap();
    wait_idle(&core, &conversation);

    let events = core.store.events_since(&conversation, 0, 10_000).unwrap();
    let retry = events
        .iter()
        .rev()
        .find(|event| {
            event.kind == "task_recovery"
                && event.payload["action"] == antislop::lean::tasks::action::RETRY_WITH_GUIDANCE
        })
        .expect("a retry recovery event");
    let message = retry.payload["message"].as_str().expect("the message");
    assert!(message.len() <= limit, "{} bytes", message.len());
    let mail = core.store.mail(&conversation).unwrap();
    assert!(
        mail.iter().any(|mail| mail.text == guidance),
        "the worker gets the whole guidance as mail"
    );

    // A second chain ends waiting again; a force approve with a huge note
    // keeps the whole note in the report and a bounded line in the event.
    std::fs::remove_file(dir.path().join("logged-in")).unwrap();
    let plan2 = propose(&core, &conversation, "chain");
    core.accept_plan(&conversation, &plan2).unwrap();
    wait_idle(&core, &conversation);
    let tasks2 = core.store.plan(&plan2).unwrap();
    let token2 = core
        .task_freshness_token(&conversation, &plan2, &tasks2[0].id)
        .unwrap();
    let note = "f".repeat(4 * limit);
    let mut completed = tasks2[0].clone();
    completed.report = Some("completed read-only analysis".into());
    core.store.update_task(&completed).unwrap();
    let turn = core
        .store
        .start_turn(
            &conversation,
            &format!("worker:{}", completed.id),
            "agy",
            0,
            Some(&plan2),
        )
        .unwrap();
    core.store
        .finish_turn(&turn.id, antislop::lean::store::TurnStatus::Done, None)
        .unwrap();
    core.recover_task(TaskRecoveryRequest {
        conversation: conversation.clone(),
        plan: plan2.clone(),
        task: tasks2[0].id.clone(),
        action: RecoveryAction::ForceApprove,
        consent_files: None,
        freshness_token: token2,
        message: Some(note.clone()),
    })
    .unwrap();
    let events = core.store.events_since(&conversation, 0, 10_000).unwrap();
    let force = events
        .iter()
        .rev()
        .find(|event| {
            event.kind == "task_recovery"
                && event.payload["action"] == antislop::lean::tasks::action::FORCE_APPROVE
        })
        .expect("a force approve recovery event");
    assert_eq!(force.payload["actor"], "user");
    let detail = force.payload["detail"].as_str().expect("the detail");
    assert!(detail.len() <= limit, "{} bytes", detail.len());
    let forced = core.store.task_by_id(&tasks2[0].id).unwrap().expect("task");
    assert!(
        forced
            .report
            .as_deref()
            .is_some_and(|report| report.contains(&note)),
        "the report keeps the whole force-approve reason"
    );
}

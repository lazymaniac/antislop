mod common;
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
prompt = prompt.split("\nCurrent task contracts for plan ",1)[0].rstrip()
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
# A plan the ledger mapping sends back for regeneration names no scenario: repeat the last one.
import os
last = os.path.join(os.path.dirname(sys.argv[0]), "last-plan")
if name:
    with open(last, "w") as f:
        f.write(name)
elif "Regenerate the task plan from the verified ledger" in prompt and os.path.exists(last):
    name = open(last).read()
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
    ids = re.findall(r"^- ([A-Za-z0-9_.-]+): ", prompt, re.M)
    if not ids or "```verdict" not in answer:
        return answer
    head, body = answer.rsplit("```verdict", 1)
    data = json.loads(body.split("```")[0])
    data["coverage"] = {i: {"status": "ok", "note": "checked"} for i in ids}
    return head + "```verdict\n" + json.dumps(data) + "\n```"
REVIEW = ("You review a plan", "new plan:", "You audit", "next plan:", "You verify", "account again for your obligations")
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
        common::interactive(script);
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
    let deadline = Instant::now() + Duration::from_secs(120);
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
    let conversation = support::fixture_conversation(&core, &dir.path().join("project"))
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
fn after_a_login_resuming_finishes_the_plan_in_fresh_round_sessions() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("project")).unwrap();
    let core = open(dir.path());
    let conversation = support::fixture_conversation(&core, &dir.path().join("project"))
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
    // A new round receives the full task and interruption context without
    // resuming the provider session of the failed round.
    let log = std::fs::read_to_string(dir.path().join("agy.log")).unwrap();
    let resumed = log
        .lines()
        .find(|line| line.contains("was interrupted"))
        .expect("a resumed worker turn");
    assert!(!resumed.contains("--conversation"), "{resumed}");
    assert!(resumed.contains("Task: Fail"), "{resumed}");
    assert!(resumed.contains("Round: 2"), "{resumed}");
    assert!(
        resumed.contains("Interrupted before a report: true"),
        "{resumed}"
    );
    assert!(
        resumed.contains("No completed worker report is available"),
        "{resumed}"
    );
}

#[test]
fn accepting_runs_the_plan_the_user_saw_not_a_newer_one() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("project")).unwrap();
    let core = open(dir.path());
    let conversation = support::fixture_conversation(&core, &dir.path().join("project"))
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
    let conversation = support::fixture_conversation(&core, &dir.path().join("project"))
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
            .contains("integrity error: plan has no requirements run")
    );
}

#[test]
fn autonomous_plan_without_requirements_cannot_resume() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("project")).unwrap();
    std::fs::write(dir.path().join("logged-in"), "").unwrap();
    let core = open_in_mode(dir.path(), "autonomous");
    let conversation = support::fixture_conversation(&core, &dir.path().join("project"))
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
            .contains("integrity error: plan has no requirements run")
    );
}

#[test]
fn test_freshness_token_staleness_and_rejection() {
    let dir = tempfile::tempdir().unwrap();
    let project = init_git_project(dir.path());
    let core = open(dir.path());
    let conversation = support::fixture_conversation(&core, &project).unwrap().id;
    let message = core
        .store
        .add_message(&conversation, "user", "Check freshness token")
        .unwrap();
    let run = core
        .store
        .create_run(&conversation, "Check freshness token")
        .unwrap();
    let source = core
        .store
        .register_message_source(&conversation, &run.id, message.seq)
        .unwrap();
    let change = serde_json::from_value(json!({"conversation":conversation,"run_id":run.id,"expected_spec_revision":0,"operation_id":"freshness-scope","reason":"test scope","entries":[{"id":"R1","source":{"source_id":source.id,"start":0,"end":message.text.len()},"interpretation":"Check freshness token","criteria":[{"id":"R1.accept","text":"Reject stale tokens without changes"}]}]})).unwrap();
    core.store
        .apply_ledger_change(&change, &dir.path().join("state"), true)
        .unwrap();
    let spec = json!({
        "requirement_ids": ["R1"], "criterion_ids": ["R1.accept"],
        "id": "t1",
        "title": "T1",
        "goal": "g",
        "kind": "analysis",
        "scope": ["README"],
        "verifier": {"focus": "f"}
    });
    let tasks = core.store.add_plan(&conversation, &[spec]).unwrap();
    core.store
        .link_plan_run(&conversation, &tasks[0].plan_id, &run.id)
        .unwrap();
    core.store
        .bind_task_ledger(&conversation, &run.id, &tasks[0].plan_id)
        .unwrap();
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
    let conversation = support::fixture_conversation(&core, &project).unwrap().id;
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
        .filter(|m| m.author == "runtime" && m.text.contains("Tasks need coordination"))
        .collect();
    assert_eq!(reports.len(), 1, "{messages:?}");
    let text = &reports[0].text;
    assert!(text.contains("reply_to_agent"));
    assert!(text.contains("submit_plan_revision"));
}

#[test]
fn test_recover_task_integrate_with_consent_and_dag_cascade() {
    let dir = tempfile::tempdir().unwrap();
    let project = init_git_project(dir.path());
    // Edit the primary file after the writer takes its base. Starting edits
    // are inherited by the writer; concurrent conflicting edits require consent.
    std::fs::write(dir.path().join("concurrent-user-edit"), "").unwrap();

    let core = open(dir.path());
    let conversation = support::fixture_conversation(&core, &project).unwrap().id;
    let plan = propose(&core, &conversation, "write_chain");
    core.accept_plan(&conversation, &plan).unwrap();
    wait_idle(&core, &conversation);

    // w1 stopped because of user edit, w2 was skipped
    assert_eq!(statuses(&core, &plan), ["waiting", "queued"]);
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
        error
            .to_string()
            .contains("independent review must approve"),
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
            action: RecoveryAction::VerifyManualSnapshot,
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
    let conversation = support::fixture_conversation(&core, &project).unwrap().id;
    let plan = propose(&core, &conversation, "write_chain");
    core.accept_plan(&conversation, &plan).unwrap();
    wait_idle(&core, &conversation);

    assert_eq!(statuses(&core, &plan), ["waiting", "queued"]);
    let tasks = core.store.plan(&plan).unwrap();
    let w1_id = &tasks
        .iter()
        .find(|task| task.spec["id"] == "w1")
        .unwrap()
        .id;
    let branch = format!("antislop/{}-g0", w1_id);
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
    let conversation = support::fixture_conversation(&core, &project).unwrap().id;
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
    let conversation = support::fixture_conversation(&core, &project).unwrap().id;
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
fn test_mechanical_audit_gate_prevents_false_success() {
    let dir = tempfile::tempdir().unwrap();
    let project = init_git_project(dir.path());
    let core = open(dir.path());
    let conversation = support::fixture_conversation(&core, &project).unwrap().id;
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
fn manual_snapshot_delivers_options_and_rejects_a_silent_variant_switch() {
    let dir = tempfile::tempdir().unwrap();
    let project = init_git_project(dir.path());
    let core = open(dir.path());
    let conversation = support::fixture_conversation(&core, &project).unwrap().id;
    let plan = propose(&core, &conversation, "manual_fix");
    core.accept_plan(&conversation, &plan).unwrap();
    wait_idle(&core, &conversation);
    let mut task = core
        .store
        .plan(&plan)
        .unwrap()
        .into_iter()
        .find(|task| task.spec["id"] == "m1")
        .unwrap();
    task.status = "needs_user".into();
    core.store.update_task(&task).unwrap();
    std::fs::write(
        project.join("hello.txt"),
        "unjustified alternative variant\n",
    )
    .unwrap();

    // The oracle rejects a departure only when it receives both the selected
    // variant and the independent comparison obligation. No new report claims
    // that the user supplied evidence or justification for the alternative.
    let agy = dir.path().join("agy");
    let script = std::fs::read_to_string(&agy).unwrap().replace(
        "answer = cover(answer)",
        r#"if "selected_approach" in prompt and "Independently compare the implementation" in prompt and open("hello.txt").read() == "unjustified alternative variant\n":
    answer = "```verdict\n" + json.dumps({"approved": False, "findings": [{"severity": "high", "issue": "Silent variant switch: alternative lacks evidence and justification"}]}) + "\n```"
answer = cover(answer)"#,
    );
    std::fs::write(&agy, script).unwrap();
    let token = core
        .task_freshness_token(&conversation, &plan, &task.id)
        .unwrap();
    let response = core
        .recover_task(TaskRecoveryRequest {
            conversation: conversation.clone(),
            plan: plan.clone(),
            task: task.id.clone(),
            action: RecoveryAction::VerifyManualSnapshot,
            consent_files: None,
            freshness_token: token,
            message: None,
        })
        .unwrap();
    assert_eq!(response.status, "needs_user");
    let updated = core.task_unscoped(&plan, &task.id).unwrap();
    assert!(
        updated
            .verdict
            .as_ref()
            .unwrap()
            .to_string()
            .contains("Silent variant switch")
    );
    let log = std::fs::read_to_string(dir.path().join("agy.log")).unwrap();
    let args: Vec<String> = serde_json::from_str(log.lines().last().unwrap()).unwrap();
    let prompt = args.last().unwrap();
    assert!(prompt.contains("selected_approach"));
    assert!(prompt.contains("Independently compare the implementation"));
    assert!(prompt.contains("reviewed options"));
    assert!(prompt.contains("git diff"));
}

#[test]
fn a_running_agent_can_be_restarted_without_blocking_the_task() {
    let dir = tempfile::tempdir().unwrap();
    let project = init_git_project(dir.path());
    let core = open(dir.path());
    let script = dir.path().join("agy");
    let source = std::fs::read_to_string(&script).unwrap();
    std::fs::write(
        &script,
        source.replace(
            "worker = \"Task: Fail\" in prompt or \"was interrupted\" in prompt",
            r#"if "Task: First plan" in prompt and "You verify" not in prompt:
    marker = os.path.join(here, "restart-started")
    if not os.path.exists(marker):
        open(marker, "w").close()
        import time
        time.sleep(20)
worker = "Task: Fail" in prompt or "was interrupted" in prompt"#,
        ),
    )
    .unwrap();
    let conversation = support::fixture_conversation(&core, &project).unwrap().id;
    let message = core
        .store
        .add_message(&conversation, "user", "Check restart")
        .unwrap();
    let run = core
        .store
        .create_run(&conversation, "Check restart")
        .unwrap();
    let source = core
        .store
        .register_message_source(&conversation, &run.id, message.seq)
        .unwrap();
    let change = serde_json::from_value(json!({"conversation":conversation,"run_id":run.id,"expected_spec_revision":0,"operation_id":"restart-scope","reason":"test scope","entries":[{"id":"R1","source":{"source_id":source.id,"start":0,"end":message.text.len()},"interpretation":"Check restart","criteria":[{"id":"R1.accept","text":"Restart a running agent"}]}]})).unwrap();
    core.store
        .apply_ledger_change(&change, &dir.path().join("state"), true)
        .unwrap();
    let revision = core
        .store
        .run(&conversation, &run.id)
        .unwrap()
        .spec_revision;
    let mut review_turns = Vec::new();
    for role in [
        "requirements-extraction-analyst",
        "requirements-extraction-verifier",
    ] {
        let turn = core
            .store
            .start_turn(&conversation, role, "mock", 0, None)
            .unwrap();
        core.store.set_session(&turn.id, &antislop::id()).unwrap();
        core.store
            .finish_turn(&turn.id, antislop::lean::store::TurnStatus::Done, None)
            .unwrap();
        core.store
            .bind_ledger_turn(
                &conversation,
                &run.id,
                &turn.id,
                revision,
                "extraction",
                std::slice::from_ref(&source.id),
            )
            .unwrap();
        review_turns.push(turn.id);
    }
    core.store
        .record_ledger_review(
            &conversation,
            &run.id,
            revision,
            "extraction",
            &review_turns[0],
            &review_turns[1],
            "",
            true,
            "checked restart requirement",
        )
        .unwrap();
    std::fs::write(dir.path().join("logged-in"), "").unwrap();
    let tasks = core
        .store
        .add_plan(
            &conversation,
            &[json!({
                "id": "a1", "title": "First plan", "goal": "Check restart", "kind": "analysis",
                "scope": ["README"], "requirement_ids": ["R1"], "criterion_ids": ["R1.accept"],
                "verifier": {"focus": "Check restart"}
            })],
        )
        .unwrap();
    let plan = tasks[0].plan_id.clone();
    core.store
        .link_plan_run(&conversation, &plan, &run.id)
        .unwrap();
    core.store
        .bind_task_ledger(&conversation, &run.id, &plan)
        .unwrap();
    core.accept_plan(&conversation, &plan).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while !dir.path().join("restart-started").exists() {
        assert!(Instant::now() < deadline, "worker did not start");
        std::thread::sleep(Duration::from_millis(20));
    }
    let task = core
        .store
        .plan(&plan)
        .unwrap()
        .into_iter()
        .find(|task| task.spec["id"] == "a1")
        .unwrap();
    assert_eq!(task.status, "working");
    let reply = core
        .recover_task(TaskRecoveryRequest {
            conversation: conversation.clone(),
            plan: plan.clone(),
            task: task.id.clone(),
            action: RecoveryAction::RestartAgent,
            consent_files: None,
            freshness_token: core
                .task_freshness_token(&conversation, &plan, &task.id)
                .unwrap(),
            message: Some("Continue after restart".into()),
        })
        .unwrap();
    assert!(reply.notice.contains("stopping the current round"));
    wait_idle(&core, &conversation);
    let restarted = core.task_unscoped(&plan, &task.id).unwrap();
    assert_eq!(restarted.status, "approved");
    assert!(restarted.round > task.round);
}

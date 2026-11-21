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
import json, sys, os
prompt = sys.argv[-1]
prompt = prompt.split("\nCurrent task contracts for plan ",1)[0].rstrip()
def task(id, title, depends=(), writes=False):
    return {"id": id, "title": title, "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal": "g", "depends_on": list(depends),
              "change_kind": ("code" if writes else "none"), "kind": "implementation" if writes else "analysis",
            "reviewer": {"summary": "f"}}
plans = {
  "chain": [task("t1", "Fail"), task("t2", "Second", ["t1"]), task("t3", "Third", ["t2"])],
  "first": [task("a1", "First plan")],
  "second": [task("b1", "Second plan")],
  "write_chain": [task("w1", "Write file", writes=True), task("w2", "Follow up", ["w1"])],
  "manual_fix": [task("m1", "Manual fix", writes=True)],
  "failed_manual": [task("f1", "Fail", writes=True)],
  "parallel": [task("t1", "Fail"), task("s1", "Slow")],
  "requeue": [task("t1", "Fail"), task("p1", "Patient")],
  "stubborn": [task("x1", "Stubborn")],
}
# A first orchestrator turn can include history after the current request.
import re
requests = re.findall(r'\[user\]\n([^\n]+)', prompt)
request = requests[-1] if requests else prompt.splitlines()[-1]
name = next((name for name in sorted(plans, key=len, reverse=True) if name in request), None)
answer = "Plan.\n```plan\n" + json.dumps({"tasks": plans[name]}) + "\n```" if name else "ok"
if "Tasks need coordination while the rest of the plan keeps running" in prompt:
    open(os.path.join(os.path.dirname(sys.argv[0]), "told-while-running"), "w").close()
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
import os
here = os.path.dirname(sys.argv[0])
if os.path.exists(os.path.join(here, "hold-review")):
    import time
    open(os.path.join(here, "review-held"), "w").close()
    while os.path.exists(os.path.join(here, "hold-review")):
        time.sleep(0.05)
prompt = sys.argv[-1]
import os
print(json.dumps({"event": "init", "conversation_id": "s"}), flush=True)
here = os.path.dirname(sys.argv[0])
with open(os.path.join(here, "agy.log"), "a") as log:
    log.write(json.dumps(sys.argv[1:]) + "\n")
allocation_marker = "Current runtime allocation (for this turn only): "
allocation = (json.loads(prompt.split(allocation_marker, 1)[1].splitlines()[0])
              if allocation_marker in prompt else {})
with open(os.path.join(here, "cwd.log"), "a") as log:
    log.write(json.dumps({"cwd": os.getcwd(), "role": allocation.get("role", ""),
                          "review": "End with review: your report" in prompt}) + "\n")
worker = "Task: Fail" in prompt or "was interrupted" in prompt
if worker and "You review" not in prompt and "You audit" not in prompt \
        and not os.path.exists(os.path.join(here, "logged-in")):
    print("Error: not logged in, run `agy login`", file=sys.stderr)
    sys.exit(1)
if "Task: Write file" in prompt and "You review" not in prompt and "You audit" not in prompt:
    if os.path.exists(os.path.join(here, "concurrent-user-edit")):
        with open(os.path.join(here, "project", "hello.txt"), "w") as primary:
            primary.write("user edit\n")
    with open("hello.txt", "w") as f:
        f.write("from worker\n")
# A slow worker that finishes once the orchestrator has heard of the stopped task.
if "Task: Slow" in prompt and "You review" not in prompt and "You audit" not in prompt:
    import time
    deadline = time.time() + 30
    while time.time() < deadline and not os.path.exists(os.path.join(here, "told-while-running")):
        time.sleep(0.05)
    if os.path.exists(os.path.join(here, "told-while-running")):
        open(os.path.join(here, "slow-saw-report"), "w").close()
if worker and "You review" not in prompt and "You audit" not in prompt:
    open(os.path.join(here, "fail-ran"), "w").close()
# A worker that finishes once the failed task has run again.
if "Task: Patient" in prompt and "You review" not in prompt and "You audit" not in prompt:
    import time
    deadline = time.time() + 30
    while time.time() < deadline and not os.path.exists(os.path.join(here, "fail-ran")):
        time.sleep(0.05)
    if os.path.exists(os.path.join(here, "fail-ran")):
        open(os.path.join(here, "patient-saw-rerun"), "w").close()
if "Task: Manual fix" in prompt and "You review" not in prompt and "You audit" not in prompt:
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
REVIEW = ("You review a plan", "new plan:", "You audit", "next plan:", "You review", "account again for your obligations")
answer = "```verdict\n{\"approved\": true, \"findings\": []}\n```" if any(m in prompt for m in REVIEW) else "report"
if "Automatic integration conflict" in prompt:
    answer = "```integration_question\nKeep the concurrent user edit or apply the worker change?\n```"
answer = cover(answer)
# A reviewer that rejects the same obligation every round.
if "Task: Stubborn" in prompt and "You review" in prompt:
    answer = cover("```verdict\n" + json.dumps({"approved": False, "findings": [
        {"check": "goal", "id": "x", "severity": "medium", "issue": "still incomplete", "where": "README"}]}) + "\n```")
    head, body = answer.rsplit("```verdict", 1)
    data = json.loads(body.split("```")[0])
    data["coverage"] = {**data.get("coverage", {}), "goal": {"status": "issue", "note": "incomplete"}}
    answer = head + "```verdict\n" + json.dumps(data) + "\n```"
print(json.dumps({"event": "result", "result": {"conversation_id": "s", "status": "SUCCESS", "response": answer, "usage": {}}}))
"#,
    )
    .unwrap();
    for script in [&codex, &agy] {
        support::install_task_protocol(script);
        support::install_task_options_protocol(script);
        std::fs::set_permissions(script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        common::interactive(script);
    }
    (codex, agy)
}

fn open(dir: &Path) -> std::sync::Arc<Core> {
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
    let deadline = Instant::now() + Duration::from_secs(120);
    let plan = loop {
        if let Some(task) = core.store.latest_plan(conversation).unwrap().first()
            && core
                .store
                .plan(&task.plan_id)
                .unwrap()
                .iter()
                .all(|task| task.status == "proposed")
            && core
                .store
                .plan_review_approved(conversation, &task.plan_id)
                .unwrap()
        {
            break task.plan_id.clone();
        }
        assert!(Instant::now() < deadline, "reviewed plan was not proposed");
        std::thread::sleep(Duration::from_millis(20));
    };
    core.resume_plan(conversation, &plan).unwrap();
    assert_eq!(
        core.store.accepted_plan(conversation).unwrap().as_deref(),
        Some(plan.as_str())
    );
    plan
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
    let arguments: Vec<String> = serde_json::from_str(resumed).unwrap();
    assert!(
        !arguments[..arguments.len() - 1]
            .iter()
            .any(|arg| arg == "--conversation"),
        "{resumed}"
    );
    let resumed = arguments.last().unwrap();
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
fn reviewed_plans_require_acceptance_and_keep_distinct_results() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("project")).unwrap();
    let core = open(dir.path());
    let conversation = support::fixture_conversation(&core, &dir.path().join("project"))
        .unwrap()
        .id;
    let first = propose(&core, &conversation, "first");
    wait_idle(&core, &conversation);
    // A completed request starts a new interview. Settle that fixture scope
    // before asking for the next independently reviewed plan.
    let request = core
        .store
        .add_message(&conversation, "user", "second")
        .unwrap();
    core.store
        .interview_summary(&conversation, "second", request.seq)
        .unwrap();
    support::confirm_scope(&core, &conversation).unwrap();
    let second = propose(&core, &conversation, "second");
    assert_ne!(first, second);

    wait_idle(&core, &conversation);
    assert_eq!(statuses(&core, &first), ["approved"]);
    assert_eq!(statuses(&core, &second), ["approved"]);
    // The latest result belongs to the independently reviewed second plan.
    let messages = core.store.messages_after(&conversation, 0).unwrap();
    let results = messages
        .iter()
        .rfind(|message| message.author == "runtime")
        .unwrap();
    assert!(results.text.contains("Second plan") && !results.text.contains("First plan"));
    // A plan already run cannot be started again.
    assert!(core.resume_plan(&conversation, &first).is_err());
}

#[test]
fn tasks_without_a_scope_cannot_resume_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("project")).unwrap();
    let core = open(dir.path());
    let conversation = support::fixture_conversation(&core, &dir.path().join("project"))
        .unwrap()
        .id;
    let spec = json!({"id": "t1", "title": "T", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal": "g", "change_kind": "none", "kind": "analysis",
                      "reviewer": {"summary": "f"}});
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
            .contains("integrity error: plan has no scope")
    );
}

#[test]
fn autonomous_plan_without_scope_cannot_resume() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("project")).unwrap();
    std::fs::write(dir.path().join("logged-in"), "").unwrap();
    let core = open(dir.path());
    let conversation = support::fixture_conversation(&core, &dir.path().join("project"))
        .unwrap()
        .id;
    let spec = json!({"id": "t1", "title": "T", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal": "g", "change_kind": "none", "kind": "analysis",
                      "reviewer": {"summary": "f"}});
    let tasks = core.store.add_plan(&conversation, &[spec]).unwrap();
    core.store
        .queue_plan(&conversation, &tasks[0].plan_id)
        .unwrap();
    let mut working = tasks[0].clone();
    working.status = "working".into();
    working.round = 1;
    core.store.update_task(&working).unwrap();
    drop(core);

    let core = open(dir.path());
    wait_idle(&core, &conversation);
    assert_eq!(statuses(&core, &tasks[0].plan_id), ["queued"]);
    assert!(
        core.resume_plan(&conversation, &tasks[0].plan_id)
            .unwrap_err()
            .to_string()
            .contains("integrity error: plan has no scope")
    );
}

#[test]
fn test_freshness_token_staleness_and_rejection() {
    let dir = tempfile::tempdir().unwrap();
    let project = init_git_project(dir.path());
    let core = open(dir.path());
    let conversation = support::fixture_conversation(&core, &project).unwrap().id;
    let _message = core
        .store
        .add_message(&conversation, "user", "Check freshness token")
        .unwrap();
    let run = core
        .store
        .create_scope(&conversation, "Check freshness token")
        .unwrap();
    let spec = json!({

        "id": "t1",
        "title": "T1",
        "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal": "g",
        "change_kind": "none", "kind": "analysis",

        "reviewer": {"summary": "f"}});
    let tasks = core.store.add_plan(&conversation, &[spec]).unwrap();
    core.store
        .bind_plan_scope(&conversation, &tasks[0].plan_id, &run.id)
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

    // Task freshness observes the whole project, ignoring historical path hints.
    std::fs::write(project.join("other-input.txt"), "first input\n").unwrap();
    let token3 = core
        .task_freshness_token(&conversation, plan, task_id)
        .unwrap();
    assert_ne!(token2, token3);
    std::fs::write(project.join("other-input.txt"), "second input\n").unwrap();
    let token4 = core
        .task_freshness_token(&conversation, plan, task_id)
        .unwrap();
    assert_ne!(
        token3, token4,
        "content changes matter even with identical Git status"
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

    // A fresh token does not make a terminal task recoverable.
    let mut task_approved = current_task;
    task_approved.status = "approved".into();
    core.store.update_task(&task_approved).unwrap();
    let fresh_token = core
        .task_freshness_token(&conversation, plan, task_id)
        .unwrap();
    let invalid_state = core
        .recover_task(TaskRecoveryRequest {
            conversation: conversation.clone(),
            plan: plan.clone(),
            task: task_id.clone(),
            action: RecoveryAction::CancelTask,
            consent_files: None,
            freshness_token: fresh_token,
            message: None,
        })
        .unwrap_err();
    assert!(
        invalid_state
            .to_string()
            .contains("task is not waiting for recovery")
    );
    assert_eq!(
        core.task_unscoped(plan, task_id).unwrap().status,
        "approved"
    );
}

#[test]
fn test_blocked_task_prevents_premature_audit() {
    let dir = tempfile::tempdir().unwrap();
    let project = init_git_project(dir.path());
    let core = open(dir.path());
    let conversation = support::fixture_conversation(&core, &project).unwrap().id;
    let plan = propose(&core, &conversation, "chain");
    wait_idle(&core, &conversation);

    assert_eq!(statuses(&core, &plan), ["waiting", "queued", "queued"]);

    // Review no audit event was emitted
    let events = core.store.events_since(&conversation, 0, 1000).unwrap();
    assert!(
        !events.iter().any(|e| e.kind == "plan_audit"),
        "blocked task must not trigger plan audit"
    );

    // Review messages do not contain plan audit / report
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
    assert!(text.contains("answer_agent"));
    assert!(text.contains("revise_plan"));
}

#[test]
fn a_task_that_stops_wakes_the_orchestrator_while_others_still_run() {
    let dir = tempfile::tempdir().unwrap();
    let project = init_git_project(dir.path());
    let core = open(dir.path());
    let conversation = support::fixture_conversation(&core, &project).unwrap().id;
    let plan = propose(&core, &conversation, "parallel");
    wait_idle(&core, &conversation);

    assert_eq!(statuses(&core, &plan), ["waiting", "approved"]);
    assert!(
        dir.path().join("slow-saw-report").exists(),
        "the orchestrator was told of the stopped task only after the slow one finished"
    );
    let messages = core.store.messages_after(&conversation, 0).unwrap();
    let interim: Vec<_> = messages
        .iter()
        .filter(|m| m.author == "runtime" && m.text.contains("keeps running"))
        .collect();
    assert_eq!(interim.len(), 1, "{messages:?}");
    assert!(interim[0].text.contains("\"t1\""));
    assert!(!interim[0].text.contains("\"s1\""));
}

#[test]
fn a_task_requeued_while_others_run_starts_without_waiting_for_them() {
    let dir = tempfile::tempdir().unwrap();
    let project = init_git_project(dir.path());
    let core = open(dir.path());
    let conversation = support::fixture_conversation(&core, &project).unwrap().id;
    let plan = propose(&core, &conversation, "requeue");
    let deadline = Instant::now() + Duration::from_secs(60);
    let failed = loop {
        let tasks = core.store.plan(&plan).unwrap();
        if let Some(task) = tasks
            .into_iter()
            .find(|t| t.spec["id"] == "t1" && t.status == "waiting")
        {
            break task;
        }
        assert!(Instant::now() < deadline, "t1 never stopped");
        std::thread::sleep(Duration::from_millis(20));
    };
    // An explicit user retry lifts the login block while the plan runs on.
    std::fs::write(dir.path().join("logged-in"), "").unwrap();
    core.recover_task(TaskRecoveryRequest {
        conversation: conversation.clone(),
        plan: plan.clone(),
        task: failed.id.clone(),
        action: RecoveryAction::RetryWithGuidance,
        consent_files: None,
        freshness_token: core
            .task_freshness_token(&conversation, &plan, &failed.id)
            .unwrap(),
        message: Some("Retry after logging in".into()),
    })
    .unwrap();
    wait_idle(&core, &conversation);

    assert!(
        dir.path().join("patient-saw-rerun").exists(),
        "the requeued task waited for the running one to finish"
    );
    assert_eq!(statuses(&core, &plan), ["approved", "approved"]);
}

#[test]
fn repeated_findings_keep_running_until_explicit_cancellation() {
    let dir = tempfile::tempdir().unwrap();
    let project = init_git_project(dir.path());
    let core = open(dir.path());
    let conversation = support::fixture_conversation(&core, &project).unwrap().id;
    let plan = propose(&core, &conversation, "stubborn");
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let records = core.store.events_since(&conversation, 0, 100000).unwrap();
        if records.iter().any(|event| {
            event.kind == "task_progress"
                && event.payload["run"]["rejections"].as_u64().unwrap_or(0) >= 4
        }) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "repeated findings stopped the task"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!core.is_idle(&conversation));
    core.cancel(&conversation);
    wait_idle(&core, &conversation);

    let task = core
        .store
        .plan(&plan)
        .unwrap()
        .into_iter()
        .find(|t| t.spec["id"] == "x1")
        .unwrap();
    assert_eq!(
        task.status, "queued",
        "cancellation retains a resumable checkpoint"
    );
    assert!(task.round >= 4);
    let records: Vec<_> = core
        .store
        .events_since(&conversation, 0, 100_000)
        .unwrap()
        .into_iter()
        .filter(|e| e.kind == "task_progress" && e.payload["task"] == task.id.as_str())
        .map(|e| e.payload["run"].clone())
        .collect();
    assert!(records.len() >= 4, "{records:?}");
    assert_eq!(records[0]["rejections"], 1);
    assert_eq!(records[1]["rejections"], 2);
    assert_eq!(records[1]["findings"][0]["check"], "goal");
    assert!(
        records.last().unwrap()["rejections"].as_u64().unwrap() >= 4,
        "cancellation preserves recorded progress for a later explicit resume"
    );
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
    wait_idle(&core, &conversation);

    // The session branch is isolated from edits in the user's checkout, so both
    // writers finish without changing that checkout.
    assert_eq!(statuses(&core, &plan), ["approved", "approved"]);
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
    let session = support::session_worktree(&core, &project, &conversation);
    assert_eq!(
        std::fs::read_to_string(project.join("hello.txt")).unwrap(),
        "user edit\n"
    );
    // Model an approved result awaiting explicit recovery: its commit remains
    // available, while the session integration line is back at the task base.
    let mut task = core
        .store
        .plan(&plan)
        .unwrap()
        .into_iter()
        .find(|task| task.id == *w1_id)
        .unwrap();
    assert!(
        std::process::Command::new("git")
            .args([
                "-C",
                session.to_str().unwrap(),
                "reset",
                "--hard",
                task.base.as_deref().unwrap()
            ])
            .status()
            .unwrap()
            .success()
    );
    task.status = "needs_user".into();
    task.verdict = None;
    core.store.update_task(&task).unwrap();
    let token = core
        .task_freshness_token(&conversation, &plan, w1_id)
        .unwrap();

    // Consent cannot proceed until an independent review approves the result.
    let error = core
        .recover_task(TaskRecoveryRequest {
            conversation: conversation.clone(),
            plan: plan.clone(),
            task: w1_id.clone(),
            action: RecoveryAction::IntegrateWithConsent,
            consent_files: Some(vec!["hello.txt".into()]),
            freshness_token: token.clone(),
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

    // Exercise the approved IntegrateWithConsent recovery path directly. The
    // original checkout still contains the user's concurrent edit; recovery
    // must apply the reviewed commit to the durable session branch instead.
    task.verdict = Some(json!({"approved": true}));
    core.store.update_task(&task).unwrap();
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
            freshness_token: token.clone(),
            message: None,
        })
        .unwrap();

    assert_eq!(resp.status, "approved");
    assert_eq!(
        std::fs::read_to_string(session.join("hello.txt")).unwrap(),
        "from worker\n"
    );
    assert_eq!(
        std::fs::read_to_string(project.join("hello.txt")).unwrap(),
        "user edit\n",
        "consent recovery preserves the user's checkout"
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
    wait_idle(&core, &conversation);

    assert_eq!(statuses(&core, &plan), ["approved", "approved"]);
    let mut task = core
        .store
        .plan(&plan)
        .unwrap()
        .into_iter()
        .find(|task| task.spec["id"] == "w1")
        .unwrap()
        .clone();
    let w1_id = task.id.clone();
    task.status = "needs_user".into();
    core.store.update_task(&task).unwrap();
    let branch = format!("antislop/{w1_id}");
    let token = core
        .task_freshness_token(&conversation, &plan, &w1_id)
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

    // This recovery preserves the task branch without rolling back an already
    // completed dependent task on the session integration line.
    wait_idle(&core, &conversation);
    assert_eq!(statuses(&core, &plan), ["needs_user", "approved"]);
}

#[test]
fn manual_snapshot_cancellation_preserves_snapshot_and_finishes_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    let project = init_git_project(dir.path());
    let core = open(dir.path());
    let conversation = support::fixture_conversation(&core, &project).unwrap().id;
    let plan = propose(&core, &conversation, "manual_fix");
    wait_idle(&core, &conversation);
    let mut task = core.store.plan(&plan).unwrap().remove(0);
    task.status = "needs_user".into();
    core.store.update_task(&task).unwrap();
    std::fs::write(project.join("hello.txt"), "manual user fix\n").unwrap();
    std::fs::write(dir.path().join("hold-review"), "").unwrap();
    let token = core
        .task_freshness_token(&conversation, &plan, &task.id)
        .unwrap();
    let request = TaskRecoveryRequest {
        conversation: conversation.clone(),
        plan: plan.clone(),
        task: task.id.clone(),
        action: RecoveryAction::ReviewManualSnapshot,
        consent_files: None,
        freshness_token: token,
        message: None,
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = core.clone();
    std::thread::spawn(move || {
        let _ = tx.send(worker.recover_task(request));
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    while !dir.path().join("review-held").exists() {
        if let Ok(result) = rx.try_recv() {
            panic!("manual review ended before the hold point: {result:?}");
        }
        assert!(Instant::now() < deadline, "manual reviewer did not start");
        std::thread::sleep(Duration::from_millis(10));
    }
    let snapshot = core.task_unscoped(&plan, &task.id).unwrap().result.unwrap();
    let token = core
        .task_freshness_token(&conversation, &plan, &task.id)
        .unwrap();
    let response = core
        .recover_task(TaskRecoveryRequest {
            conversation: conversation.clone(),
            plan: plan.clone(),
            task: task.id.clone(),
            action: RecoveryAction::CancelTask,
            consent_files: None,
            freshness_token: token,
            message: None,
        })
        .unwrap();
    assert!(response.notice.contains("cancelling"));
    assert!(
        rx.recv_timeout(Duration::from_secs(10))
            .expect("manual cancellation blocked")
            .is_err()
    );
    let updated = core.task_unscoped(&plan, &task.id).unwrap();
    assert_eq!(updated.status, "cancelled");
    assert_eq!(updated.result.as_deref(), Some(snapshot.as_str()));
    assert_eq!(
        git(&project, &["show", &format!("{snapshot}:hello.txt")]),
        "manual user fix"
    );
    assert_eq!(
        std::fs::read_to_string(project.join("hello.txt")).unwrap(),
        "manual user fix\n"
    );
}

#[test]
fn test_recover_task_review_manual_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let project = init_git_project(dir.path());
    let core = open(dir.path());
    let conversation = support::fixture_conversation(&core, &project).unwrap().id;
    let plan = propose(&core, &conversation, "failed_manual");
    wait_idle(&core, &conversation);

    let tasks = core.store.plan(&plan).unwrap();
    let task = tasks
        .iter()
        .find(|task| task.spec["change_kind"] == "code")
        .expect("the manual recovery test needs the source writer, not its options task")
        .clone();
    assert_eq!(task.status, "waiting", "the worker failure must be natural");
    assert!(
        task.report
            .as_deref()
            .is_some_and(|report| report.contains("not logged in"))
    );

    // User makes a manual fix in the project
    std::fs::write(project.join("hello.txt"), "manual user fix\n").unwrap();
    std::fs::write(project.join("notes.txt"), "manual note\n").unwrap();
    let head_before = git(&project, &["rev-parse", "HEAD"]);

    let token = core
        .task_freshness_token(&conversation, &plan, &task.id)
        .unwrap();
    let resp = core
        .recover_task(TaskRecoveryRequest {
            conversation: conversation.clone(),
            plan: plan.clone(),
            task: task.id.clone(),
            action: RecoveryAction::ReviewManualSnapshot,
            consent_files: None,
            freshness_token: token,
            message: None,
        })
        .unwrap();

    assert_eq!(resp.status, "needs_user");
    let updated = core.task_unscoped(&plan, &task.id).unwrap();
    assert_eq!(updated.status, "needs_user");
    assert_eq!(updated.verdict.as_ref().unwrap()["approved"], true);
    assert!(
        updated
            .feedback
            .as_ref()
            .unwrap()
            .get("integration")
            .is_none()
    );
    let reviewed_snapshot = updated.result.clone().unwrap();
    assert_eq!(
        updated.feedback.as_ref().unwrap()["manual_snapshot"]["result"],
        reviewed_snapshot
    );
    let diff = core.diff(&conversation, &plan, &task.id).unwrap();
    assert_eq!(diff["head"], reviewed_snapshot);
    assert_eq!(diff["changed_files"], json!(["hello.txt", "notes.txt"]));
    assert_eq!(
        git(&project, &["rev-parse", "HEAD"]),
        head_before,
        "manual snapshot review must not commit before explicit file consent"
    );

    std::fs::write(project.join("hello.txt"), "newer user edit\n").unwrap();
    let changed_token = core
        .task_freshness_token(&conversation, &plan, &task.id)
        .unwrap();
    let changed = core.recover_task(TaskRecoveryRequest {
        conversation: conversation.clone(),
        plan: plan.clone(),
        task: task.id.clone(),
        action: RecoveryAction::IntegrateWithConsent,
        consent_files: Some(vec!["hello.txt".into()]),
        freshness_token: changed_token,
        message: None,
    });
    assert!(
        changed
            .unwrap_err()
            .to_string()
            .contains("changed after review")
    );
    assert_eq!(
        std::fs::read_to_string(project.join("hello.txt")).unwrap(),
        "newer user edit\n",
        "integration preserves a user edit made after snapshot review"
    );
    std::fs::write(project.join("hello.txt"), "manual user fix\n").unwrap();

    let token = core
        .task_freshness_token(&conversation, &plan, &task.id)
        .unwrap();
    let no_consent = core.recover_task(TaskRecoveryRequest {
        conversation: conversation.clone(),
        plan: plan.clone(),
        task: task.id.clone(),
        action: RecoveryAction::IntegrateWithConsent,
        consent_files: None,
        freshness_token: token.clone(),
        message: None,
    });
    assert!(
        no_consent.is_err(),
        "missing consent cannot integrate reviewed files"
    );
    assert_eq!(
        core.task_unscoped(&plan, &task.id).unwrap().status,
        "needs_user"
    );
    let partial = core
        .recover_task(TaskRecoveryRequest {
            conversation: conversation.clone(),
            plan: plan.clone(),
            task: task.id.clone(),
            action: RecoveryAction::IntegrateWithConsent,
            consent_files: Some(vec!["hello.txt".into()]),
            freshness_token: token.clone(),
            message: None,
        })
        .unwrap();
    assert_eq!(partial.status, "needs_user");
    assert_eq!(partial.integrated_files, ["hello.txt"]);

    std::fs::write(project.join("hello.txt"), "newer user edit\n").unwrap();
    let changed_token = core
        .task_freshness_token(&conversation, &plan, &task.id)
        .unwrap();
    let changed = core.recover_task(TaskRecoveryRequest {
        conversation: conversation.clone(),
        plan: plan.clone(),
        task: task.id.clone(),
        action: RecoveryAction::IntegrateWithConsent,
        consent_files: Some(vec!["notes.txt".into()]),
        freshness_token: changed_token,
        message: None,
    });
    assert!(
        changed
            .unwrap_err()
            .to_string()
            .contains("changed after review")
    );
    assert_eq!(
        std::fs::read_to_string(project.join("hello.txt")).unwrap(),
        "newer user edit\n"
    );
    std::fs::write(project.join("hello.txt"), "manual user fix\n").unwrap();
    let completion_token = core
        .task_freshness_token(&conversation, &plan, &task.id)
        .unwrap();
    let integrated = core
        .recover_task(TaskRecoveryRequest {
            conversation: conversation.clone(),
            plan: plan.clone(),
            task: task.id.clone(),
            action: RecoveryAction::IntegrateWithConsent,
            consent_files: Some(vec!["notes.txt".into()]),
            freshness_token: completion_token,
            message: None,
        })
        .unwrap();
    assert_eq!(integrated.status, "approved");
    assert_eq!(integrated.integrated_files, ["notes.txt"]);
    let replay = core.recover_task(TaskRecoveryRequest {
        conversation: conversation.clone(),
        plan: plan.clone(),
        task: task.id.clone(),
        action: RecoveryAction::IntegrateWithConsent,
        consent_files: Some(vec!["hello.txt".into()]),
        freshness_token: token,
        message: None,
    });
    assert!(
        replay.is_err(),
        "a token from before consent cannot be replayed"
    );

    // Commit policy applies to manually reviewed work, too.
    let head_after = git(&project, &["rev-parse", "HEAD"]);
    assert_ne!(head_before, head_after);
    assert_eq!(
        git(&project, &["show", "HEAD:hello.txt"]),
        "manual user fix"
    );
    assert_eq!(git(&project, &["show", "HEAD:notes.txt"]), "manual note");

    // Review task result points to the snapshot commit
    assert!(updated.result.is_some());
    let updated = core.task_unscoped(&plan, &task.id).unwrap();
    let snap_commit = updated.result.unwrap();
    assert_ne!(snap_commit, head_after);
    assert_eq!(
        git(&project, &["show", &format!("{snap_commit}:hello.txt")]),
        "manual user fix"
    );
    let calls: Vec<serde_json::Value> = std::fs::read_to_string(dir.path().join("cwd.log"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let worker = calls
        .iter()
        .find(|call| call["role"].as_str().unwrap().starts_with("worker:"))
        .expect("task worker call");
    let reviewers: Vec<_> = calls
        .iter()
        .filter(|call| call["role"].as_str().unwrap().starts_with("reviewer:"))
        .collect();
    assert!(!reviewers.is_empty(), "task reviewer call: {calls:?}");
    let logged_calls = std::fs::read_to_string(dir.path().join("agy.log")).unwrap();
    let args: Vec<Vec<String>> = logged_calls
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    for reviewer in reviewers {
        assert_eq!(
            worker["cwd"], reviewer["cwd"],
            "manual recovery reviewer shares the worker task checkout: {calls:?}"
        );
        let review_index = calls.iter().position(|call| call == reviewer).unwrap();
        assert!(
            args[review_index]
                .last()
                .unwrap()
                .contains("do not modify project files"),
            "reviewer remains read-only"
        );
    }
}

#[test]
fn test_recover_task_retry_with_guidance() {
    let dir = tempfile::tempdir().unwrap();
    let project = init_git_project(dir.path());
    let core = open(dir.path());
    let conversation = support::fixture_conversation(&core, &project).unwrap().id;
    let plan = propose(&core, &conversation, "chain");
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
    let recovery = core
        .store
        .events_since(&conversation, 0, 10_000)
        .unwrap()
        .into_iter()
        .find(|event| {
            event.kind == "task_recovery"
                && event.payload["action"] == "retry_with_guidance"
                && event.payload["task"] == *t1_id
        })
        .expect("retry writes its recovery event");
    assert_eq!(recovery.plan_id.as_deref(), Some(plan.as_str()));
    assert_eq!(recovery.payload["status"], "queued");
    assert_eq!(recovery.payload["message"], "retrying after login");

    wait_idle(&core, &conversation);
    assert_eq!(statuses(&core, &plan), ["approved", "approved", "approved"]);
}

#[test]
fn test_mechanical_audit_gate_prevents_false_success() {
    let dir = tempfile::tempdir().unwrap();
    let project = init_git_project(dir.path());
    let core = open(dir.path());
    let conversation = support::fixture_conversation(&core, &project).unwrap().id;
    let spec1 = json!({"id": "t1", "title": "Done", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal": "g", "change_kind": "none", "kind": "analysis", "reviewer": {"summary": "f"}});
    let spec2 = json!({"id": "t2", "title": "Blocked", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal": "g", "change_kind": "none", "kind": "analysis", "reviewer": {"summary": "f"}});
    let tasks = core.store.add_plan(&conversation, &[spec1, spec2]).unwrap();
    let plan = &tasks[0].plan_id;
    let scope = core
        .store
        .create_scope(&conversation, "Verify both tasks")
        .unwrap();
    core.store
        .bind_plan_scope(&conversation, plan, &scope.id)
        .unwrap();

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
            action: RecoveryAction::ReviewManualSnapshot,
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
            r#"if "Task: First plan" in prompt and "You review" not in prompt:
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
    let _message = core
        .store
        .add_message(&conversation, "user", "Check restart")
        .unwrap();
    let run = core
        .store
        .create_scope(&conversation, "Check restart")
        .unwrap();

    std::fs::write(dir.path().join("logged-in"), "").unwrap();
    let tasks = core
        .store
        .add_plan(
            &conversation,
            &[json!({
                "id": "a1", "title": "First plan", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal": "Check restart", "change_kind": "none", "kind": "analysis",

                "reviewer": {"summary": "Check restart"}})],
        )
        .unwrap();
    let plan = tasks[0].plan_id.clone();
    core.store
        .bind_plan_scope(&conversation, &plan, &run.id)
        .unwrap();
    core.store.queue_plan(&conversation, &plan).unwrap();
    core.resume_plan(&conversation, &plan).unwrap();
    let deadline = Instant::now() + Duration::from_secs(120);
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

#[test]
fn missing_cli_during_restart_preserves_checkpoint_and_records_failure_before_retry() {
    let dir = tempfile::tempdir().unwrap();
    let project = init_git_project(dir.path());
    let core = open(dir.path());
    let conversation = support::fixture_conversation(&core, &project).unwrap().id;
    let plan = propose(&core, &conversation, "chain");
    wait_idle(&core, &conversation);
    let mut before = core.store.plan(&plan).unwrap().remove(0);
    before.feedback.get_or_insert_with(|| json!({}))["checkpoint_note"] =
        json!("keep this evidence");
    core.store.update_task(&before).unwrap();
    let turn = core
        .store
        .last_turn(&conversation, &format!("worker:{}", before.id))
        .unwrap()
        .unwrap();
    let cli = dir.path().join("agy");
    let saved = dir.path().join("agy.saved");
    std::fs::rename(&cli, &saved).unwrap();
    let request = |action| TaskRecoveryRequest {
        conversation: conversation.clone(),
        plan: plan.clone(),
        task: before.id.clone(),
        action,
        consent_files: None,
        message: None,
        freshness_token: core
            .task_freshness_token(&conversation, &plan, &before.id)
            .unwrap(),
    };
    let error = core
        .recover_task(request(RecoveryAction::RestartAgent))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("not an executable file or on PATH"),
        "{error}"
    );
    let after = core.task(&plan, &before.id).unwrap();
    assert_eq!(after.status, "failed");
    assert_eq!(after.report, before.report);
    assert_eq!(after.base, before.base);
    assert_eq!(after.result, before.result);
    assert_eq!(
        after.feedback.as_ref().unwrap()["checkpoint_note"],
        "keep this evidence"
    );
    assert_eq!(after.feedback.as_ref().unwrap()["reason"], error);
    assert_eq!(
        core.store
            .last_turn(&conversation, &format!("worker:{}", before.id))
            .unwrap()
            .unwrap()
            .session_id,
        turn.session_id
    );
    assert!(
        core.store
            .events_since(&conversation, 0, 10000)
            .unwrap()
            .iter()
            .any(|e| e.kind == "task_recovery_failed" && e.payload["reason"] == error)
    );
    std::fs::rename(saved, cli).unwrap();
    std::fs::write(dir.path().join("logged-in"), "").unwrap();
    core.recover_task(request(RecoveryAction::RetryWithGuidance))
        .unwrap();
    wait_idle(&core, &conversation);
    assert_eq!(statuses(&core, &plan), ["approved", "approved", "approved"]);
}

#[test]
fn legacy_branch_bootstrap_reopens_and_retries_a_retained_checkpoint() {
    use sha2::{Digest, Sha256};
    let dir = tempfile::tempdir().unwrap();
    let project = init_git_project(dir.path());
    let main = git(&project, &["rev-parse", "main"]);
    std::fs::write(project.join("user-draft.txt"), "unsent user work\n").unwrap();
    let core = open(dir.path());
    let conversation = support::fixture_conversation(&core, &project).unwrap().id;
    let plan = propose(&core, &conversation, "chain");
    wait_idle(&core, &conversation);
    let session = support::session_worktree(&core, &project, &conversation);
    let session_branch = git(&session, &["symbolic-ref", "--short", "HEAD"]);
    core.store
        .append_event(
            &conversation,
            None,
            None,
            "session_started",
            &json!({"branch":"main"}),
        )
        .unwrap();
    let mut task = core.store.plan(&plan).unwrap().remove(0);

    let checkpoint_dir = dir.path().join("checkpoint");
    git(
        &project,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "retained-checkpoint",
            checkpoint_dir.to_str().unwrap(),
            "main",
        ],
    );
    std::fs::write(checkpoint_dir.join("retained.txt"), "checkpoint evidence\n").unwrap();
    git(&checkpoint_dir, &["add", "."]);
    git(&checkpoint_dir, &["commit", "-qm", "retained checkpoint"]);
    task.base = Some(main.clone());
    task.result = Some(git(&checkpoint_dir, &["rev-parse", "HEAD"]));
    task.report = Some("Retained observations before allocation failed".into());
    task.status = "failed".into();
    task.feedback.get_or_insert_with(|| json!({}))["reason"] =
        json!("legacy session has no persisted integration branch");
    core.store.update_task(&task).unwrap();
    let config = (*core.config_snapshot()).clone();
    drop(core);

    // Model a legacy runtime: task evidence exists, but no session mapping or ref.
    let state = dir.path().join("state");
    let record = state.join("session-branches").join(
        Sha256::digest(conversation.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
    );
    std::fs::remove_file(record).unwrap();
    git(&project, &["worktree", "remove", session.to_str().unwrap()]);
    git(&project, &["branch", "-D", &session_branch]);
    let task_branch = format!("antislop/{}", task.id);
    for block in git(&project, &["worktree", "list", "--porcelain"]).split("\n\n") {
        if block
            .lines()
            .any(|line| line == format!("branch refs/heads/{task_branch}"))
        {
            let path = block
                .lines()
                .find_map(|line| line.strip_prefix("worktree "))
                .unwrap();
            git(&project, &["worktree", "remove", path]);
        }
    }
    git(
        &project,
        &[
            "update-ref",
            &format!("refs/heads/{task_branch}"),
            task.result.as_deref().unwrap(),
        ],
    );
    let agy = dir.path().join("agy");
    let source = std::fs::read_to_string(&agy).unwrap();
    let anchor = "# A reviewer that rejects the same obligation every round.";
    assert!(source.contains(anchor));
    std::fs::write(&agy, source.replace(anchor, &format!(r#"
if worker and "You review" not in prompt and os.path.exists(os.path.join(here, "logged-in")):
    assert open("retained.txt").read() == "checkpoint evidence\n", "checkpoint missing from resumed allocation"
    open(os.path.join(here, "retained-checkpoint-seen"), "w").close()
{anchor}
"#))).unwrap();

    let core = Core::open(&state, config).unwrap();
    let restored = core.task(&plan, &task.id).unwrap();
    assert_eq!(restored.status, "failed");
    assert_eq!(restored.report, task.report);
    assert_eq!(restored.base, task.base);
    assert_eq!(restored.result, task.result);
    assert_eq!(
        git(
            &support::session_worktree(&core, &project, &conversation),
            &["rev-parse", "HEAD"]
        ),
        main
    );
    std::fs::write(dir.path().join("logged-in"), "").unwrap();
    core.recover_task(TaskRecoveryRequest {
        conversation: conversation.clone(),
        plan: plan.clone(),
        task: task.id.clone(),
        action: RecoveryAction::RetryWithGuidance,
        consent_files: None,
        freshness_token: core
            .task_freshness_token(&conversation, &plan, &task.id)
            .unwrap(),
        message: Some(
            "Session integration branch restored; continue from the retained checkpoint".into(),
        ),
    })
    .unwrap();
    wait_idle(&core, &conversation);
    assert_eq!(statuses(&core, &plan), ["approved", "approved", "approved"]);
    assert!(dir.path().join("retained-checkpoint-seen").exists());
    assert_eq!(git(&project, &["rev-parse", "main"]), main);
    assert_eq!(
        std::fs::read_to_string(project.join("user-draft.txt")).unwrap(),
        "unsent user work\n"
    );
    assert!(!project.join("retained.txt").exists());
}

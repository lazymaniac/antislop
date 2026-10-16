use architect::{config::Config, lean::Core};
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
def task(id, title, depends=()):
    return {"id": id, "title": title, "goal": "g", "depends_on": list(depends),
            "kind": "analysis", "verifier": {"focus": "f"}}
plans = {
  "chain": [task("t1", "Fail"), task("t2", "Second", ["t1"]), task("t3", "Third", ["t2"])],
  "first": [task("a1", "First plan")],
  "second": [task("b1", "Second plan")],
}
name = next((name for name in plans if name in prompt.splitlines()[-1]), None)
answer = "Plan.\n```plan\n" + json.dumps({"tasks": plans[name]}) + "\n```" if name else "ok"
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
answer = cover(answer)
print(json.dumps({"event": "result", "result": {"conversation_id": "s", "status": "SUCCESS", "response": answer, "usage": {}}}))
"#,
    )
    .unwrap();
    for script in [&codex, &agy] {
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
        "/examples/architect.toml"
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
    let text = text.replace("mode = \"supervised\"", &format!("mode = {mode:?}"));
    Core::open(&dir.join("state"), Config::from_toml(&text).unwrap()).unwrap()
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
        .map(|task| task.status)
        .collect()
}

#[test]
fn a_failure_skips_every_task_that_depends_on_it_and_says_why() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("project")).unwrap();
    let core = open(dir.path());
    let conversation = core.start(&dir.path().join("project")).unwrap().id;
    let plan = propose(&core, &conversation, "chain");
    core.accept_plan(&conversation, &plan).unwrap();
    wait_idle(&core, &conversation);

    assert_eq!(statuses(&core, &plan), ["waiting", "skipped", "skipped"]);
    let waiting = &core.store.plan(&plan).unwrap()[0];
    let report = waiting.report.as_deref().unwrap();
    assert!(report.contains("not logged in"), "{report}");
}

#[test]
fn after_a_login_resuming_finishes_the_plan_in_the_same_sessions() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("project")).unwrap();
    let core = open(dir.path());
    let conversation = core.start(&dir.path().join("project")).unwrap().id;
    let plan = propose(&core, &conversation, "chain");
    core.accept_plan(&conversation, &plan).unwrap();
    wait_idle(&core, &conversation);
    assert_eq!(statuses(&core, &plan), ["waiting", "skipped", "skipped"]);

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
    let conversation = core.start(&dir.path().join("project")).unwrap().id;
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
fn tasks_cut_off_by_a_daemon_restart_end_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("project")).unwrap();
    let core = open(dir.path());
    let conversation = core.start(&dir.path().join("project")).unwrap().id;
    let spec = json!({"id": "t1", "title": "T", "goal": "g", "kind": "analysis",
                      "verifier": {"focus": "f"}});
    let tasks = core
        .store
        .add_plan(&conversation, &[spec.clone(), spec])
        .unwrap();
    core.store
        .queue_plan(&conversation, &tasks[0].plan_id)
        .unwrap();
    let mut working = tasks[0].clone();
    working.status = "working".into();
    core.store.update_task(&working).unwrap();
    drop(core);

    let core = open(dir.path());
    let after = core.store.plan(&tasks[0].plan_id).unwrap();
    assert_eq!(
        statuses(&core, &tasks[0].plan_id),
        ["cancelled", "cancelled"]
    );
    assert!(
        after[0]
            .report
            .as_deref()
            .unwrap()
            .contains("[interrupted]")
    );
}

#[test]
fn an_autonomous_plan_cut_off_by_a_restart_resumes_by_itself() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("project")).unwrap();
    std::fs::write(dir.path().join("logged-in"), "").unwrap();
    let core = open_in_mode(dir.path(), "autonomous");
    let conversation = core.start(&dir.path().join("project")).unwrap().id;
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
    assert_eq!(statuses(&core, &tasks[0].plan_id), ["approved"]);
}

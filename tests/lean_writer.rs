mod common;
mod support;
use antislop::{config::Config, lean::Core};
use serde_json::Value;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

/// Orchestrator proposes one writing task; the worker writes hello.txt in its
/// cwd; the verifier approves. Every call logs its cwd and arguments.
fn fake_clis(dir: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let log = dir.join("calls.jsonl");
    let common = format!(
        r#"#!/usr/bin/env python3
import json, os, sys
prompt = sys.argv[-1]
with open({log:?}, "a") as f:
    f.write(json.dumps({{"bin": sys.argv[0].rsplit("/", 1)[-1], "cwd": os.getcwd(), "args": sys.argv[1:]}}) + "\n")
"#,
        log = log.to_string_lossy()
    );
    let codex = dir.join("codex");
    std::fs::write(
        &codex,
        common.clone()
            + r#"
plan = {"tasks": [{"id": "t1", "title": "Add greeting", "goal": "Create hello.txt",
  "writes": True, "scope": [".", "../shared"], "kind": "implementation", "verifier": {"focus": "file content"}}]}
answer = "done" if "[runtime]" in prompt else "Plan.\n```plan\n" + json.dumps(plan) + "\n```"
print(json.dumps({"type": "thread.started", "thread_id": "orch"}))
print(json.dumps({"type": "item.completed", "item": {"type": "agent_message", "text": answer}}))
print(json.dumps({"type": "turn.completed", "usage": {}}))
"#,
    )
    .unwrap();
    let agy = dir.join("agy");
    std::fs::write(
        &agy,
        common
            + r#"
def cover(answer):
    import re
    ids = re.findall(r"^- ([RCV][0-9]+): ", prompt, re.M)
    if not ids or "```verdict" not in answer:
        return answer
    head, body = answer.rsplit("```verdict", 1)
    data = json.loads(body.split("```")[0])
    data["coverage"] = {i: {"status": "ok", "note": "checked"} for i in ids}
    return head + "```verdict\n" + json.dumps(data) + "\n```"
REVIEW = ("You review a plan", "new plan:", "You audit", "next plan:")
if any(marker in prompt for marker in REVIEW):
    session = "reviewer"
    answer = "```verdict\n{\"approved\": true, \"findings\": []}\n```"
elif "You review a code change" in prompt:
    session = "code-review"
    answer = "```verdict\n" + json.dumps({"approved": False, "findings": [
        {"severity": "medium", "issue": "greeting lacks a name", "where": "hello.txt:1"}]}) + "\n```"
elif "You verify" in prompt:
    session = "verifier"
    answer = "```verdict\n" + json.dumps({"approved": "git diff" in prompt, "findings": []}) + "\n```"
else:
    session = "worker"
    with open("hello.txt", "w") as f:
        f.write("from worker\n")
    answer = "wrote hello.txt"
answer = cover(answer)
print(json.dumps({"event": "init", "conversation_id": session}))
print(json.dumps({"event": "result", "result": {"conversation_id": session, "status": "SUCCESS", "response": answer, "usage": {}}}))
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

/// Simulate a user editing the primary tree after the worker took its base.
fn fake_clis_with_concurrent_edit(
    dir: &Path,
    project: &Path,
) -> (std::path::PathBuf, std::path::PathBuf) {
    let (codex, agy) = fake_clis(dir);
    let script = std::fs::read_to_string(&agy).unwrap();
    let injection = format!(
        "session = \"worker\"\n    if not os.path.exists({marker:?}):\n        open({marker:?}, \"w\").close()\n        with open({file:?}, \"w\") as primary:\n            primary.write(\"user draft\\n\")",
        marker = dir.join("edited-once"),
        file = project.join("hello.txt"),
    );
    let script = script.replace("elif \"You verify\" in prompt:", "elif \"You verify\" in prompt or \"account again\" in prompt:").replace("session = \"worker\"", &injection).replace(
        "answer = cover(answer)",
        "if \"Automatic integration conflict\" in prompt:\n    answer = \"```integration_question\\nuncommitted edits in hello.txt: keep the user draft or accept the worker greeting?\\n```\"\nanswer = cover(answer)",
    );
    std::fs::write(&agy, script).unwrap();
    (codex, agy)
}

fn config(codex: &Path, agy: &Path) -> Config {
    config_with(codex, agy, |text| text)
}

/// The same two fake CLIs, but the worker answers with the Python
/// expression `worker_answer` — used to probe the optional ```effect
/// block the worker may add to the very report it gives.
fn fake_clis_answered(dir: &Path, worker_answer: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let log = dir.join("calls.jsonl");
    let common = format!(
        r#"#!/usr/bin/env python3
import json, os, sys
prompt = sys.argv[-1]
with open({log:?}, "a") as f:
    f.write(json.dumps({{"bin": sys.argv[0].rsplit("/", 1)[-1], "cwd": os.getcwd(), "args": sys.argv[1:]}}) + "\n")
"#,
        log = log.to_string_lossy()
    );
    let codex = dir.join("codex");
    std::fs::write(
        &codex,
        common.clone()
            + r#"
plan = {"tasks": [{"id": "t1", "title": "Add greeting", "goal": "Create hello.txt",
  "writes": True, "scope": [".", "../shared"], "kind": "implementation", "verifier": {"focus": "file content"}}]}
answer = "done" if "[runtime]" in prompt else "Plan.\n```plan\n" + json.dumps(plan) + "\n```"
print(json.dumps({"type": "thread.started", "thread_id": "orch"}))
print(json.dumps({"type": "item.completed", "item": {"type": "agent_message", "text": answer}}))
print(json.dumps({"type": "turn.completed", "usage": {}}))
"#,
    )
    .unwrap();
    let agy = dir.join("agy");
    std::fs::write(
        &agy,
        common
            + format!(
                r#"
def cover(answer):
    import re
    ids = re.findall(r"^- ([RCV][0-9]+): ", prompt, re.M)
    if not ids or "```verdict" not in answer:
        return answer
    head, body = answer.rsplit("```verdict", 1)
    data = json.loads(body.split("```")[0])
    data["coverage"] = {{i: {{"status": "ok", "note": "checked"}} for i in ids}}
    return head + "```verdict\n" + json.dumps(data) + "\n```"
REVIEW = ("You review a plan", "new plan:", "You audit", "next plan:")
if any(marker in prompt for marker in REVIEW):
    session = "reviewer"
    answer = "```verdict\n{{\"approved\": true, \"findings\": []}}\n```"
elif "You review a code change" in prompt:
    session = "code-review"
    answer = "```verdict\n" + json.dumps({{"approved": False, "findings": [
        {{"severity": "medium", "issue": "greeting lacks a name", "where": "hello.txt:1"}}]}}) + "\n```"
elif "You verify" in prompt:
    session = "verifier"
    answer = "```verdict\n" + json.dumps({{"approved": "git diff" in prompt, "findings": []}}) + "\n```"
else:
    session = "worker"
    with open("hello.txt", "w") as f:
        f.write("from worker\n")
    answer = {worker_answer}
answer = cover(answer)
print(json.dumps({{"event": "init", "conversation_id": session}}))
print(json.dumps({{"event": "result", "result": {{"conversation_id": session, "status": "SUCCESS", "response": answer, "usage": {{}}}}}}))
"#
            )
            .as_str(),
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

/// These tests follow one writing task through its agents, so the required
/// quality gates a real plan gets are switched off;
/// `required_gates_join_plans_that_write` switches them back on.
fn config_with(codex: &Path, agy: &Path, change: impl Fn(String) -> String) -> Config {
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
    .replace("required_gates = true", "required_gates = false")
    .replace("local_commits = true", "local_commits = false");
    Config::from_toml(&change(text)).unwrap()
}

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn project(dir: &Path) -> std::path::PathBuf {
    let project = dir.join("project");
    std::fs::create_dir(&project).unwrap();
    git(&project, &["init", "--quiet", "-b", "main"]);
    std::fs::write(project.join("README"), "readme\n").unwrap();
    git(&project, &["add", "."]);
    git(&project, &["commit", "--quiet", "-m", "base"]);
    // A related directory the plan names in its scope.
    std::fs::create_dir_all(dir.join("shared")).unwrap();
    project.canonicalize().unwrap()
}

fn run_plan(core: &std::sync::Arc<Core>, project: &Path) -> String {
    let conversation = support::legacy_conversation(core, project).unwrap();
    core.send(&conversation.id, "add a greeting").unwrap();
    wait_idle(core, &conversation.id);
    let plan = core.store.latest_plan(&conversation.id).unwrap()[0]
        .plan_id
        .clone();
    core.accept_plan(&conversation.id, &plan).unwrap();
    wait_idle(core, &conversation.id);
    conversation.id
}

fn original_tasks(core: &Core, conversation: &str) -> Vec<antislop::lean::store::Task> {
    core.store
        .latest_plan(conversation)
        .unwrap()
        .into_iter()
        .filter(|task| task.spec["kind"] != "options")
        .collect()
}

fn wait_idle(core: &Core, conversation: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !core.is_idle(conversation) {
        assert!(Instant::now() < deadline, "conversation never went idle");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn required_gates_join_plans_that_write() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let (codex, agy) = fake_clis(dir.path());
    let config = config_with(&codex, &agy, |text| {
        text.replace("required_gates = false", "required_gates = true")
    });
    let core = Core::open(&project.join(".antislop"), config).unwrap();
    let conversation = support::legacy_conversation(&core, &project).unwrap();
    core.send(&conversation.id, "add a greeting").unwrap();
    wait_idle(&core, &conversation.id);
    let plan = core.store.latest_plan(&conversation.id).unwrap();
    let kinds: Vec<String> = plan
        .iter()
        .map(|task| task.spec["kind"].as_str().unwrap().to_owned())
        .collect();
    for writer in plan.iter().filter(|task| task.spec["writes"] == true) {
        let dependencies = writer.spec["depends_on"].as_array().unwrap();
        assert!(plan.iter().any(|task| task.spec["kind"] == "options"
            && task.spec["writes"] == false
            && dependencies.contains(&task.spec["id"])));
    }
    let plan: Vec<_> = plan
        .iter()
        .filter(|task| task.spec["kind"] != "options")
        .collect();
    let kinds: Vec<_> = kinds.iter().filter(|kind| *kind != "options").collect();
    assert_eq!(plan.len(), 7, "{kinds:?}");
    let writer = plan[0].spec["id"].clone();
    for gate in &plan[1..] {
        // The security and performance audits only read.
        let audit = matches!(gate.spec["kind"].as_str(), Some("security" | "performance"));
        assert_eq!(gate.spec["writes"], !audit);
        assert!(
            gate.spec["depends_on"]
                .as_array()
                .unwrap()
                .contains(&writer),
            "{:?}",
            gate.spec
        );
    }
    assert_eq!(
        kinds[1..],
        [
            "refactoring",
            "static_analysis",
            "testing",
            "security",
            "performance",
            "integration"
        ]
    );
}

#[test]
fn an_approved_writer_lands_in_the_project_without_a_commit() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let base = git(&project, &["rev-parse", "HEAD"]);
    let (codex, agy) = fake_clis(dir.path());
    let core = Core::open(&project.join(".antislop"), config(&codex, &agy)).unwrap();
    let conversation = run_plan(&core, &project);

    let task = &original_tasks(&core, &conversation)[0];
    assert_eq!(task.status, "approved", "{:?}", task.report);
    assert!(task.report.as_deref().unwrap().contains("not committed"));
    // local_commits = false in the example configuration: applied, not committed.
    assert_eq!(git(&project, &["rev-parse", "HEAD"]), base);
    assert_eq!(git(&project, &["status", "--porcelain"]), "?? hello.txt");
    assert_eq!(
        std::fs::read_to_string(project.join("hello.txt")).unwrap(),
        "from worker\n"
    );
    assert_eq!(git(&project, &["worktree", "list"]).lines().count(), 1);

    let calls: Vec<Value> = std::fs::read_to_string(dir.path().join("calls.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let reviewer = |call: &&Value| {
        let args = call["args"].to_string();
        args.contains("You review a plan") || args.contains("You audit")
    };
    // Plan review and the final audit read the project itself.
    let reviews: Vec<_> = calls.iter().filter(reviewer).collect();
    assert_eq!(reviews.len(), 2);
    assert!(
        reviews
            .iter()
            .all(|call| call["cwd"] == project.to_str().unwrap())
    );
    let agents: Vec<_> = calls
        .iter()
        .filter(|call| call["bin"] == "agy" && !reviewer(call))
        .collect();
    assert_eq!(agents.len(), 2);
    let worktree = agents[0]["cwd"].as_str().unwrap();
    assert!(worktree.contains("/.antislop/worktrees/"), "{worktree}");
    assert_eq!(
        agents[1]["cwd"], agents[0]["cwd"],
        "verifier reads the worktree"
    );
    let worker_args = agents[0]["args"].to_string();
    assert!(worker_args.contains("accept-edits") && worker_args.contains("--sandbox"));
    // The plan names ../shared; only the read-only verifier gets it from
    // its CLI, a writer is never given a directory to write outside.
    let shared = dir.path().join("shared").canonicalize().unwrap();
    let shared = shared.to_str().unwrap();
    assert!(!worker_args.contains(shared), "{worker_args}");
    assert!(agents[1]["args"].to_string().contains(shared));
}

#[test]
fn a_conflict_returns_to_the_writer_and_the_repair_is_verified_before_landing() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let (codex, agy) = fake_clis_with_concurrent_edit(dir.path(), &project);
    let script = std::fs::read_to_string(&agy).unwrap().replace(
        "answer = \"```integration_question\\nuncommitted edits in hello.txt: keep the user draft or accept the worker greeting?\\n```\"",
        "with open(\"hello.txt\", \"w\") as repaired:\n        repaired.write(\"user draft\\nfrom worker\\n\")\n    answer = \"Preserved the user draft and added the worker greeting.\"",
    ).replace("elif \"You verify\" in prompt:", "elif \"You verify\" in prompt or \"account again\" in prompt:");
    let script = script.replace(
        "primary.write(\"user draft\\n\")",
        &format!("primary.write(\"user draft\\n\")\n        with open({:?}, \"w\") as unrelated:\n            unrelated.write(\"concurrent README\\n\")", project.join("README")),
    ).replace(
        "session = \"verifier\"",
        &format!("session = \"verifier\"\n    assert open({:?}).read() == \"user draft\\n\", \"unverified repair touched primary tree\"", project.join("hello.txt")),
    );
    std::fs::write(&agy, script).unwrap();
    let core = Core::open(&project.join(".antislop"), config(&codex, &agy)).unwrap();
    let conversation = run_plan(&core, &project);
    let task = original_tasks(&core, &conversation)[0].clone();
    assert_eq!(task.status, "approved", "{:?}", task.report);
    assert_eq!(task.round, 2, "a fresh verifier round checks the repair");
    assert_eq!(
        std::fs::read_to_string(project.join("hello.txt")).unwrap(),
        "user draft\nfrom worker\n"
    );
    assert_eq!(
        std::fs::read_to_string(project.join("README")).unwrap(),
        "concurrent README\n"
    );
    assert_eq!(
        core.diff(&conversation, &task.plan_id, "t1").unwrap()["files"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "stale unrelated worktree files are not part of the repaired delta"
    );
    let calls = std::fs::read_to_string(dir.path().join("calls.jsonl")).unwrap();
    assert!(calls.contains("Automatic integration conflict"));
    assert!(calls.contains("Verify the conflict resolution independently"));
    assert!(calls.contains("integration_question"));
    let logged: Vec<Value> = calls
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let repair = logged
        .iter()
        .filter_map(|call| call["args"].as_array()?.last()?.as_str())
        .find(|prompt| prompt.contains("Automatic integration conflict"))
        .unwrap();
    for required in [
        "Round handoff",
        "Create hello.txt",
        "Latest worker report",
        "wrote hello.txt",
        "Requirements and acceptance criteria",
        "Current open comments",
        "hello.txt",
        "README",
        "Changes already present in the primary working tree",
        "never edit the primary tree during repair",
    ] {
        assert!(
            repair.contains(required),
            "missing {required} from integration handoff: {repair}"
        );
    }
    let verify = logged
        .iter()
        .filter_map(|call| call["args"].as_array()?.last()?.as_str())
        .find(|prompt| prompt.contains("Verify the conflict resolution independently"))
        .unwrap();
    assert!(verify.contains("Round: 2"));
    assert!(verify.contains("Your obligations"));
    assert!(verify.contains("Preserved the user draft and added the worker greeting."));
}

#[test]
fn a_writer_integrates_over_the_local_changes_it_started_with() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    std::fs::write(project.join("hello.txt"), "user draft\n").unwrap();
    let (codex, agy) = fake_clis(dir.path());
    let core = Core::open(&project.join(".antislop"), config(&codex, &agy)).unwrap();
    let conversation = run_plan(&core, &project);

    let task = &original_tasks(&core, &conversation)[0];
    assert_eq!(task.status, "approved", "{:?}", task.report);
    assert_eq!(
        std::fs::read_to_string(project.join("hello.txt")).unwrap(),
        "from worker\n"
    );
    assert_eq!(
        git(
            &project,
            &[
                "show",
                &format!("{}:hello.txt", task.base.as_ref().unwrap())
            ]
        ),
        "user draft"
    );
    // The user was told before the writer started.
    let notices: Vec<_> = core
        .store
        .events_since(&conversation, 0, 10_000)
        .unwrap()
        .into_iter()
        .filter(|event| event.kind == "notice")
        .filter(|event| {
            // This configuration also has the required gates off, which is
            // told separately.
            !event.payload["text"]
                .as_str()
                .unwrap()
                .contains("required_gates is off")
        })
        .collect();
    assert_eq!(notices.len(), 1);
    assert!(
        notices[0].payload["text"]
            .as_str()
            .unwrap()
            .contains("uncommitted changes in hello.txt"),
        "{:?}",
        notices[0].payload
    );
}

#[test]
fn resuming_integrates_an_approved_change_once_the_user_is_done() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let (codex, agy) = fake_clis_with_concurrent_edit(dir.path(), &project);
    let core = Core::open(&project.join(".antislop"), config(&codex, &agy)).unwrap();
    let conversation = run_plan(&core, &project);
    let task = original_tasks(&core, &conversation)[0].clone();
    assert_eq!(task.status, "needs_user");
    let calls_before = std::fs::read_to_string(dir.path().join("calls.jsonl"))
        .unwrap()
        .lines()
        .filter(|line| line.contains("accept-edits"))
        .count();

    // The user drops their draft; resuming lands the approved change without
    // asking the worker again.
    std::fs::remove_file(project.join("hello.txt")).unwrap();
    core.resume_plan(&conversation, &task.plan_id).unwrap();
    wait_idle(&core, &conversation);

    let task = &core
        .store
        .plan(&task.plan_id)
        .unwrap()
        .into_iter()
        .find(|row| row.id == task.id)
        .unwrap();
    assert_eq!(task.status, "approved", "{:?}", task.report);
    assert_eq!(
        std::fs::read_to_string(project.join("hello.txt")).unwrap(),
        "from worker\n"
    );
    let calls_after = std::fs::read_to_string(dir.path().join("calls.jsonl"))
        .unwrap()
        .lines()
        .filter(|line| line.contains("accept-edits"))
        .count();
    assert!(
        calls_after > calls_before,
        "the worker resolves the choice and is verified again"
    );
    assert!(core.resume_plan(&conversation, &task.plan_id).is_err());
}

#[test]
fn the_user_reads_a_change_comments_a_line_and_asks_for_a_review() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let (codex, agy) = fake_clis(dir.path());
    let core = Core::open(&project.join(".antislop"), config(&codex, &agy)).unwrap();
    let conversation = run_plan(&core, &project);
    let task = original_tasks(&core, &conversation).remove(0);
    let plan = task.plan_id.clone();

    let diff = core.diff(&conversation, &plan, "t1").unwrap();
    let file = &diff["files"][0];
    assert_eq!(
        (
            &file["path"],
            &file["status"],
            &file["added"],
            &file["removed"]
        ),
        (
            &Value::from("hello.txt"),
            &Value::from("added"),
            &Value::from(1),
            &Value::from(0)
        )
    );
    assert_eq!(file["lines"][1]["text"], "from worker");
    assert_eq!(file["lines"][1]["new"], 1);
    assert_eq!(diff["head"], task.result.clone().unwrap().as_str());

    core.comment(&conversation, "diff:t1:hello.txt:1", "add a name")
        .unwrap();
    wait_idle(&core, &conversation);
    core.review_change(&conversation, &plan, "t1").unwrap();
    assert!(
        core.review_change(&conversation, &plan, "t1").is_err(),
        "one review at a time"
    );
    wait_idle(&core, &conversation);

    let reviews: Vec<Value> = core
        .store
        .events_since(&conversation, 0, 10_000)
        .unwrap()
        .into_iter()
        .filter(|event| event.kind == "code_review")
        .map(|event| event.payload)
        .collect();
    assert_eq!(reviews.len(), 2);
    assert_eq!(reviews[1]["approved"], false);
    assert_eq!(reviews[1]["findings"][0]["issue"], "greeting lacks a name");
    let calls: Vec<Value> = std::fs::read_to_string(dir.path().join("calls.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let review = calls
        .iter()
        .find(|call| {
            call["args"]
                .to_string()
                .contains("You review a code change")
        })
        .unwrap();
    let prompt = review["args"].to_string();
    assert!(prompt.contains("add a name") && prompt.contains("line 1 of hello.txt"));
    assert!(prompt.contains(&format!(
        "git diff {} {}",
        task.base.unwrap(),
        task.result.unwrap()
    )));
    assert_eq!(
        review["cwd"],
        project.to_str().unwrap(),
        "the worktree is gone"
    );
    let orchestrator = calls.iter().rfind(|call| call["bin"] == "codex").unwrap();
    assert!(
        orchestrator["args"]
            .to_string()
            .contains("[code review of task t1 \\\"Add greeting\\\", requested by the user]"),
        "{}",
        orchestrator["args"]
    );
    assert!(
        core.diff(&conversation, &plan, "t9").is_err(),
        "unknown task"
    );
}

fn in_place(text: String) -> String {
    text.replace("mode = \"task-worktrees\"", "mode = \"in-place\"")
}

fn chained_writing_clis(dir: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let (codex, agy) = fake_clis(dir);
    let plan = serde_json::json!({"tasks": [
        {"id": "t1", "title": "Add greeting", "goal": "Create hello.txt", "writes": true,
         "scope": ["."], "kind": "implementation", "verifier": {"focus": "file content"}},
        {"id": "t2", "title": "Extend greeting", "goal": "Update hello.txt", "writes": true,
         "depends_on": ["t1"], "scope": ["."], "kind": "implementation", "verifier": {"focus": "file content"}}
    ]});
    let script = std::fs::read_to_string(&codex).unwrap();
    let start = script.find("plan = ").unwrap();
    let end = script[start..].find("\nanswer = ").unwrap() + start;
    std::fs::write(
        &codex,
        format!(
            "{}plan = json.loads({:?}){}",
            &script[..start],
            plan.to_string(),
            &script[end..]
        ),
    )
    .unwrap();
    let script = std::fs::read_to_string(&agy).unwrap().replace(
        "f.write(\"from worker\\n\")",
        "f.write(\"from second worker\\n\" if \"Task id: t2\\n\" in prompt else \"from worker\\n\")",
    );
    std::fs::write(&agy, script).unwrap();
    (codex, agy)
}

#[test]
fn in_place_chained_writers_can_extend_approved_uncommitted_results() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let head = git(&project, &["rev-parse", "HEAD"]);
    let (codex, agy) = chained_writing_clis(dir.path());
    let core = Core::open(
        &project.join(".antislop"),
        config_with(&codex, &agy, in_place),
    )
    .unwrap();
    // The same rule applies across conversations and plans.
    for _ in 0..2 {
        let conversation = run_plan(&core, &project);
        for task in core.store.latest_plan(&conversation).unwrap() {
            assert_eq!(task.status, "approved", "{:?}", task.report);
        }
        assert_eq!(
            std::fs::read_to_string(project.join("hello.txt")).unwrap(),
            "from second worker\n"
        );
        assert_eq!(git(&project, &["rev-parse", "HEAD"]), head);
        assert_eq!(git(&project, &["status", "--porcelain"]), "?? hello.txt");
    }
}

#[test]
fn in_place_ownership_survives_restart_but_later_user_edits_stay_protected() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let (codex, agy) = fake_clis(dir.path());
    let state = project.join(".antislop");
    let core = Core::open(&state, config_with(&codex, &agy, in_place)).unwrap();
    let conversation = run_plan(&core, &project);
    assert_eq!(original_tasks(&core, &conversation)[0].status, "approved");
    drop(core);

    let script = std::fs::read_to_string(&agy)
        .unwrap()
        .replace("from worker\\n", "updated by worker\\n");
    std::fs::write(&agy, script).unwrap();
    let core = Core::open(&state, config_with(&codex, &agy, in_place)).unwrap();
    let conversation = run_plan(&core, &project);
    assert_eq!(original_tasks(&core, &conversation)[0].status, "approved");
    assert_eq!(
        std::fs::read_to_string(project.join("hello.txt")).unwrap(),
        "updated by worker\n"
    );

    // Neither new user content nor a revert to an older approved version
    // may be treated as the latest approved output.
    for text in ["new user draft\n", "from worker\n"] {
        std::fs::write(project.join("hello.txt"), text).unwrap();
        let conversation = run_plan(&core, &project);
        let task = &original_tasks(&core, &conversation)[0];
        assert_eq!(task.status, "needs_user", "{:?}", task.report);
        assert!(
            task.report
                .as_deref()
                .unwrap()
                .contains("files you were editing: hello.txt")
        );
        assert_eq!(
            git(
                &project,
                &[
                    "show",
                    &format!("{}:hello.txt", task.base.as_ref().unwrap())
                ]
            ),
            text.trim_end()
        );
    }
}

#[test]
fn in_place_approval_does_not_claim_unrelated_user_files_in_its_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    std::fs::write(project.join("README"), "user draft\n").unwrap();
    let (codex, agy) = fake_clis(dir.path());
    let core = Core::open(
        &project.join(".antislop"),
        config_with(&codex, &agy, in_place),
    )
    .unwrap();
    let conversation = run_plan(&core, &project);
    assert_eq!(original_tasks(&core, &conversation)[0].status, "approved");
    let script = std::fs::read_to_string(&agy).unwrap().replace(
        "with open(\"hello.txt\", \"w\")",
        "with open(\"README\", \"w\")",
    );
    std::fs::write(&agy, script).unwrap();
    let conversation = run_plan(&core, &project);
    let task = &original_tasks(&core, &conversation)[0];
    assert_eq!(task.status, "needs_user", "{:?}", task.report);
    assert!(
        task.report
            .as_deref()
            .unwrap()
            .contains("files you were editing: README")
    );
    assert_eq!(
        git(
            &project,
            &["show", &format!("{}:README", task.base.as_ref().unwrap())]
        ),
        "user draft"
    );
    // Detection retains the overwritten files; it does not silently roll back.
    assert_eq!(
        std::fs::read_to_string(project.join("README")).unwrap(),
        "from worker\n"
    );
}

#[test]
fn in_place_rejected_changes_do_not_become_approved_outputs() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    std::fs::write(project.join("hello.txt"), "user draft\n").unwrap();
    let (codex, agy) = fake_clis(dir.path());
    let core = Core::open(
        &project.join(".antislop"),
        config_with(&codex, &agy, in_place),
    )
    .unwrap();
    let first = run_plan(&core, &project);
    assert_eq!(original_tasks(&core, &first)[0].status, "needs_user");
    let script = std::fs::read_to_string(&agy)
        .unwrap()
        .replace("from worker\\n", "second attempt\\n");
    std::fs::write(&agy, script).unwrap();
    let second = run_plan(&core, &project);
    let task = &original_tasks(&core, &second)[0];
    assert_eq!(task.status, "needs_user", "{:?}", task.report);
    assert!(
        task.report
            .as_deref()
            .unwrap()
            .contains("files you were editing: hello.txt")
    );
}

#[test]
fn in_place_a_writer_edits_the_project_itself_and_reviewers_see_a_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let base = git(&project, &["rev-parse", "HEAD"]);
    let (codex, agy) = fake_clis(dir.path());
    let core = Core::open(
        &project.join(".antislop"),
        config_with(&codex, &agy, in_place),
    )
    .unwrap();
    let conversation = run_plan(&core, &project);

    let task = &original_tasks(&core, &conversation)[0];
    assert_eq!(task.status, "approved", "{:?}", task.report);
    assert!(task.report.as_deref().unwrap().contains("applied in place"));
    assert_eq!(
        git(&project, &["rev-parse", "HEAD"]),
        base,
        "nothing committed"
    );
    assert_eq!(git(&project, &["status", "--porcelain"]), "?? hello.txt");
    assert!(
        !project.join(".antislop/worktrees").exists()
            || project
                .join(".antislop/worktrees")
                .read_dir()
                .unwrap()
                .next()
                .is_none()
    );
    let (snapshot, result) = (task.base.clone().unwrap(), task.result.clone().unwrap());
    assert_eq!(git(&project, &["rev-parse", &format!("{snapshot}^")]), base);
    assert_eq!(
        git(&project, &["show", &format!("{result}:hello.txt")]),
        "from worker"
    );

    let calls: Vec<Value> = std::fs::read_to_string(dir.path().join("calls.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let agents: Vec<&Value> = calls
        .iter()
        .filter(|call| {
            let args = call["args"].to_string();
            call["bin"] == "agy"
                && !args.contains("You review a plan")
                && !args.contains("You audit")
        })
        .collect();
    assert_eq!(agents.len(), 2);
    assert!(
        agents
            .iter()
            .all(|call| call["cwd"] == project.to_str().unwrap())
    );
    assert!(
        agents[1]["args"]
            .to_string()
            .contains(&format!("git diff {snapshot} {result}"))
    );
    let diff = core.diff(&conversation, &task.plan_id, "t1").unwrap();
    assert_eq!(diff["files"][0]["path"], "hello.txt");
}

#[test]
fn in_place_a_writer_that_touches_the_users_edits_stops_and_keeps_their_version() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    // The user is editing hello.txt; the worker will overwrite it.
    std::fs::write(project.join("hello.txt"), "user draft\n").unwrap();
    let (codex, agy) = fake_clis(dir.path());
    let core = Core::open(
        &project.join(".antislop"),
        config_with(&codex, &agy, in_place),
    )
    .unwrap();
    let conversation = run_plan(&core, &project);

    let task = &original_tasks(&core, &conversation)[0];
    assert_eq!(task.status, "needs_user");
    let report = task.report.as_deref().unwrap();
    assert!(
        report.contains("files you were editing: hello.txt"),
        "{report}"
    );
    let snapshot = task.base.clone().unwrap();
    assert!(report.contains(&format!("git checkout {snapshot} -- <file>")));
    assert_eq!(
        git(&project, &["show", &format!("{snapshot}:hello.txt")]),
        "user draft",
        "the user's version is recoverable"
    );
    let calls = std::fs::read_to_string(dir.path().join("calls.jsonl")).unwrap();
    assert!(calls.contains("do not modify them") && calls.contains("- hello.txt"));
}

/// The card feedback a task ended with, and the last `task` event that
/// carried it.
fn card(core: &Core, conversation: &str, spec_id: &str) -> (Value, Value) {
    let task = core
        .store
        .latest_plan(conversation)
        .unwrap()
        .into_iter()
        .find(|task| task.spec["id"] == spec_id)
        .unwrap();
    let last = core
        .store
        .events_since(conversation, 0, 10_000)
        .unwrap()
        .into_iter()
        .rfind(|event| event.kind == "task" && event.payload["id"] == spec_id)
        .expect("no task event");
    (
        task.feedback.expect("the task ended with card feedback"),
        last.payload["feedback"].clone(),
    )
}

#[test]
fn an_applied_change_gives_its_card_the_effect_and_the_full_coverage() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let (codex, agy) = fake_clis(dir.path());
    let core = Core::open(&project.join(".antislop"), config(&codex, &agy)).unwrap();
    let conversation = run_plan(&core, &project);
    let task = core
        .store
        .latest_plan(&conversation)
        .unwrap()
        .into_iter()
        .find(|task| task.spec["id"] == "t1")
        .unwrap();

    let (feedback, from_event) = card(&core, &conversation, "t1");
    assert_eq!(
        from_event, feedback,
        "the card is announced, not only stored"
    );
    assert_eq!(feedback["round"], task.round);
    assert_eq!(feedback["integration"]["outcome"], "applied");
    assert_eq!(feedback["integration"]["source"], "run");
    assert_eq!(
        feedback["integration"]["files"],
        serde_json::json!(["hello.txt"])
    );
    assert!(
        feedback["integration"]["detail"]
            .as_str()
            .unwrap()
            .contains("not committed")
    );
    // The effect names the change, bounded, never the full report.
    assert_eq!(
        feedback["effect"],
        "1 file changed, 1 insertion(+) — hello.txt"
    );
    // Coverage accounts for the verifier's real obligations: its one risk,
    // the common checks and the kind's checklist — all answered ok.
    let spec: antislop::lean::tasks::TaskSpec = serde_json::from_value(task.spec.clone()).unwrap();
    let obligations = antislop::lean::tasks::obligations(&spec);
    assert_eq!(feedback["coverage"]["total"], obligations.len());
    assert_eq!(
        feedback["coverage"]["ok"].as_u64().unwrap()
            + feedback["coverage"]["issue"].as_u64().unwrap()
            + feedback["coverage"]["blocked"].as_u64().unwrap()
            + feedback["coverage"]["na"].as_u64().unwrap()
            + feedback["coverage"]["missing"].as_u64().unwrap(),
        obligations.len() as u64
    );
    assert_eq!(feedback["coverage"]["missing"], 0);
    assert_eq!(feedback["coverage"]["unknown"], serde_json::json!([]));
}

#[test]
fn a_worker_stated_effect_takes_the_card_and_no_extra_model_call_is_spent() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let (codex, agy) = fake_clis_answered(
        dir.path(),
        "\"wrote hello.txt\\n\\n```effect\\nAdded a greeting — hello.txt\\n```\"",
    );
    let core = Core::open(&project.join(".antislop"), config(&codex, &agy)).unwrap();
    let conversation = run_plan(&core, &project);
    let (feedback, from_event) = card(&core, &conversation, "t1");
    // The worker's own bounded line is the card's effect, not the diffstat.
    assert_eq!(feedback["effect"], "Added a greeting — hello.txt");
    // The block is dropped from the stored report, never copied into the
    // task event whole: the announced feedback is the same bounded string.
    let task = core
        .store
        .latest_plan(&conversation)
        .unwrap()
        .into_iter()
        .find(|task| task.spec["id"] == "t1")
        .unwrap();
    let report = task.report.as_deref().unwrap();
    assert!(report.contains("wrote hello.txt"), "{report}");
    assert!(!report.contains("```effect"), "block removed: {report}");
    assert_eq!(from_event["effect"], feedback["effect"]);
    // Same run, one worker turn: the round ran on a single worker call.
    let calls = std::fs::read_to_string(dir.path().join("calls.jsonl")).unwrap();
    let worker_turns = calls
        .lines()
        .filter(|line| line.contains("\"bin\": \"agy\"") && line.contains("You are a worker"))
        .count();
    assert_eq!(worker_turns, 1, "no extra model call: {calls}");
}

#[test]
fn an_empty_effect_block_stays_in_the_report_and_the_diffstat_fills_in() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let (codex, agy) =
        fake_clis_answered(dir.path(), "\"wrote hello.txt\\n\\n```effect\\n\\n```\"");
    let core = Core::open(&project.join(".antislop"), config(&codex, &agy)).unwrap();
    let conversation = run_plan(&core, &project);
    let (feedback, _) = card(&core, &conversation, "t1");
    assert_eq!(
        feedback["effect"], "1 file changed, 1 insertion(+) — hello.txt",
        "the runtime derives the effect when the block says nothing"
    );
    let task = core
        .store
        .latest_plan(&conversation)
        .unwrap()
        .into_iter()
        .find(|task| task.spec["id"] == "t1")
        .unwrap();
    let report = task.report.as_deref().unwrap();
    assert!(
        report.contains("```effect"),
        "the damaged block is kept, not swallowed: {report}"
    );
}

#[test]
fn a_committed_change_names_its_commit_on_the_card() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let (codex, agy) = fake_clis(dir.path());
    let core = Core::open(
        &project.join(".antislop"),
        config_with(&codex, &agy, |text| {
            text.replace("local_commits = false", "local_commits = true")
        }),
    )
    .unwrap();
    let conversation = run_plan(&core, &project);

    let (feedback, _) = card(&core, &conversation, "t1");
    let head = git(&project, &["rev-parse", "HEAD"]);
    assert_eq!(feedback["integration"]["outcome"], "committed");
    assert_eq!(feedback["integration"]["commit"], head);
    assert_eq!(
        feedback["integration"]["files"],
        serde_json::json!(["hello.txt"])
    );
    assert_eq!(git(&project, &["status", "--porcelain"]), "");
}

#[test]
fn without_write_permission_the_card_says_the_branch_and_keeps_the_project_clean() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let head = git(&project, &["rev-parse", "HEAD"]);
    let (codex, agy) = fake_clis(dir.path());
    let core = Core::open(
        &project.join(".antislop"),
        config_with(&codex, &agy, |text| {
            text.replace("local_write = true", "local_write = false")
        }),
    )
    .unwrap();
    let conversation = run_plan(&core, &project);

    let task = &original_tasks(&core, &conversation)[0];
    assert_eq!(task.status, "needs_user", "{:?}", task.report);
    let (feedback, _) = card(&core, &conversation, "t1");
    assert_eq!(feedback["integration"]["outcome"], "left_on_branch");
    assert_eq!(
        feedback["integration"]["branch"],
        format!("antislop/{}", task.id)
    );
    assert!(
        feedback["integration"]["detail"]
            .as_str()
            .unwrap()
            .contains("local writes are not allowed"),
        "the raw reason reaches the card"
    );
    assert_eq!(git(&project, &["rev-parse", "HEAD"]), head);
    assert_eq!(git(&project, &["status", "--porcelain"]), "");
}

#[test]
fn a_writer_that_changes_nothing_reports_no_changes() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let (codex, agy) = fake_clis(dir.path());
    // The worker no longer writes: its round commits nothing on top of base.
    let script = std::fs::read_to_string(&agy).unwrap().replace(
        "with open(\"hello.txt\", \"w\")",
        "with open(os.devnull, \"w\")",
    );
    std::fs::write(&agy, script).unwrap();
    let core = Core::open(&project.join(".antislop"), config(&codex, &agy)).unwrap();
    let conversation = run_plan(&core, &project);

    let (feedback, _) = card(&core, &conversation, "t1");
    assert_eq!(feedback["integration"]["outcome"], "no_changes");
    assert_eq!(feedback["effect"], "no file changes");
    assert_eq!(feedback["integration"]["detail"], "no file changes");
    assert!(!feedback.as_object().unwrap().contains_key("files"));
}

#[test]
fn protected_user_edits_leave_their_file_and_the_reason_on_the_card() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let (codex, agy) = fake_clis_with_concurrent_edit(dir.path(), &project);
    let core = Core::open(&project.join(".antislop"), config(&codex, &agy)).unwrap();
    let conversation = run_plan(&core, &project);

    let task = &original_tasks(&core, &conversation)[0];
    assert_eq!(task.status, "needs_user");
    let (feedback, _) = card(&core, &conversation, "t1");
    assert_eq!(feedback["round"], task.round);
    assert_eq!(feedback["integration"]["outcome"], "user_edits");
    assert_eq!(
        feedback["integration"]["files"],
        serde_json::json!(["hello.txt"])
    );
    assert_eq!(
        feedback["integration"]["branch"],
        format!("antislop/{}", task.id)
    );
    assert!(
        feedback["integration"]["detail"]
            .as_str()
            .unwrap()
            .contains("uncommitted edits in hello.txt"),
        "the raw reason reaches the card"
    );
    assert!(feedback["effect"].as_str().unwrap().contains("hello.txt"));
}

#[test]
fn a_resume_conflicting_with_the_users_commit_shows_the_conflict_on_the_card() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let (codex, agy) = fake_clis_with_concurrent_edit(dir.path(), &project);
    let core = Core::open(&project.join(".antislop"), config(&codex, &agy)).unwrap();
    let conversation = run_plan(&core, &project);
    let task = original_tasks(&core, &conversation)[0].clone();
    assert_eq!(task.status, "needs_user");

    // The user commits their own version of the line the change also writes:
    // the approved change no longer applies cleanly.
    std::fs::write(project.join("hello.txt"), "another user version\n").unwrap();
    git(&project, &["add", "hello.txt"]);
    git(&project, &["commit", "--quiet", "-m", "user version"]);
    core.resume_plan(&conversation, &task.plan_id).unwrap();
    wait_idle(&core, &conversation);

    let task = &core
        .store
        .plan(&task.plan_id)
        .unwrap()
        .into_iter()
        .find(|row| row.id == task.id)
        .unwrap();
    assert_eq!(task.status, "needs_user", "{:?}", task.report);
    let (feedback, _) = card(&core, &conversation, "t1");
    assert_eq!(feedback["integration"]["outcome"], "conflict");
    assert_eq!(
        feedback["integration"]["files"],
        serde_json::json!(["hello.txt"])
    );
    assert!(
        feedback["integration"]["detail"]
            .as_str()
            .unwrap()
            .contains("keep the user draft or accept the worker greeting?"),
        "the resolver asks the user which behavior to accept"
    );
    assert_eq!(
        std::fs::read_to_string(project.join("hello.txt")).unwrap(),
        "another user version\n",
        "the user's committed content is not overwritten by a conflict"
    );
}

#[test]
fn in_place_work_is_named_as_such_on_the_card() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let base = git(&project, &["rev-parse", "HEAD"]);
    let (codex, agy) = fake_clis(dir.path());
    let core = Core::open(
        &project.join(".antislop"),
        config_with(&codex, &agy, in_place),
    )
    .unwrap();
    let conversation = run_plan(&core, &project);

    let (feedback, _) = card(&core, &conversation, "t1");
    assert_eq!(feedback["integration"]["outcome"], "in_place_applied");
    assert!(
        feedback["integration"]["detail"]
            .as_str()
            .unwrap()
            .starts_with("applied in place"),
        "{feedback}"
    );
    assert_eq!(
        feedback["integration"]["files"],
        serde_json::json!(["hello.txt"])
    );
    assert_eq!(git(&project, &["rev-parse", "HEAD"]), base);
}

#[test]
fn an_in_place_commit_names_its_commit_on_the_card() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let (codex, agy) = fake_clis(dir.path());
    let core = Core::open(
        &project.join(".antislop"),
        config_with(&codex, &agy, |text| {
            in_place(text).replace("local_commits = false", "local_commits = true")
        }),
    )
    .unwrap();
    let conversation = run_plan(&core, &project);

    let (feedback, _) = card(&core, &conversation, "t1");
    assert_eq!(feedback["integration"]["outcome"], "in_place_committed");
    assert_eq!(
        feedback["integration"]["commit"],
        git(&project, &["rev-parse", "HEAD"])
    );
    assert_eq!(git(&project, &["status", "--porcelain"]), "");
}

#[test]
fn an_in_place_worker_over_the_users_edits_names_the_protection_on_the_card() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    std::fs::write(project.join("hello.txt"), "user draft\n").unwrap();
    let (codex, agy) = fake_clis(dir.path());
    let core = Core::open(
        &project.join(".antislop"),
        config_with(&codex, &agy, in_place),
    )
    .unwrap();
    let conversation = run_plan(&core, &project);

    let (feedback, _) = card(&core, &conversation, "t1");
    assert_eq!(feedback["integration"]["outcome"], "in_place_user_edits");
    assert_eq!(
        feedback["integration"]["files"],
        serde_json::json!(["hello.txt"])
    );
    assert!(
        feedback["integration"]["detail"]
            .as_str()
            .unwrap()
            .contains("files you were editing: hello.txt"),
        "the raw reason reaches the card"
    );
}

/// One read-only task and a verifier whose coverage adds an invented id:
/// a read-only card must say `not_applicable` and count only real obligations.
fn read_only_clis(dir: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let (codex, agy) = fake_clis(dir);
    let plan = serde_json::json!({"tasks": [
        {"id": "t1", "title": "Audit greetings", "goal": "Report on README",
         "kind": "analysis", "verifier": {"focus": "coverage of the audit",
         "risks": ["a missed file"]}}
    ]});
    let script = std::fs::read_to_string(&codex).unwrap();
    let start = script.find("plan = ").unwrap();
    let end = script[start..].find("\nanswer = ").unwrap() + start;
    std::fs::write(
        &codex,
        format!(
            "{}plan = json.loads({:?}){}",
            &script[..start],
            plan.to_string(),
            &script[end..]
        ),
    )
    .unwrap();
    let script = std::fs::read_to_string(&agy)
        .unwrap()
        .replace(
            r#"data["coverage"] = {i: {"status": "ok", "note": "checked"} for i in ids}"#,
            r#"data["coverage"] = {i: {"status": "ok", "note": "checked"} for i in ids}
    data["coverage"]["X9"] = {"status": "ok", "note": "invented"}
    data["coverage"]["V1"] = {"status": "n/a", "note": "no build here"}
    data["coverage"]["R1"] = {"status": "issue", "note": "one file unreadable"}"#,
        )
        .replace(
            "answer = \"wrote hello.txt\"",
            "answer = \"README read; 3 notes filed\"",
        )
        .replace("\"approved\": \"git diff\" in prompt", "\"approved\": True");
    std::fs::write(&agy, script).unwrap();
    (codex, agy)
}

#[test]
fn a_read_only_card_says_not_applicable_and_counts_only_real_obligations() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let (codex, agy) = read_only_clis(dir.path());
    let core = Core::open(&project.join(".antislop"), config(&codex, &agy)).unwrap();
    let conversation = run_plan(&core, &project);

    let task = core
        .store
        .latest_plan(&conversation)
        .unwrap()
        .into_iter()
        .find(|task| task.spec["id"] == "t1")
        .unwrap();
    assert_eq!(task.status, "approved", "{:?}", task.report);
    let (feedback, _) = card(&core, &conversation, "t1");
    assert_eq!(feedback["integration"]["outcome"], "not_applicable");
    assert_eq!(feedback["integration"]["source"], "run");
    // The read-only effect is the report's opening line, not the report.
    assert_eq!(feedback["effect"], "README read; 3 notes filed");
    let spec: antislop::lean::tasks::TaskSpec = serde_json::from_value(task.spec.clone()).unwrap();
    let obligations = antislop::lean::tasks::obligations(&spec);
    assert_eq!(feedback["coverage"]["total"], obligations.len());
    // The invented id is reported and counted nowhere. The `issue` on the
    // read-only risk describes the code, so it is counted but does not reject.
    assert_eq!(feedback["coverage"]["unknown"], serde_json::json!(["X9"]));
    assert_eq!(feedback["coverage"]["unknown_more"], 0);
    assert_eq!(feedback["effect_cut"], serde_json::json!(false));
    assert_eq!(feedback["coverage"]["na"], 1);
    assert_eq!(feedback["coverage"]["issue"], 1);
    assert_eq!(feedback["coverage"]["blocked"], 0);
    assert_eq!(feedback["coverage"]["missing"], 0);
}

#[test]
fn a_retry_invalidates_the_earlier_rounds_card_result() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let (codex, agy) = fake_clis_with_concurrent_edit(dir.path(), &project);
    // A new round starts a fresh verifier session. Coverage follow-ups within
    // that round also need verifier answers from the stub.
    let script = std::fs::read_to_string(&agy).unwrap().replace(
        "elif \"You verify\" in prompt:",
        "elif (\"You verify\" in prompt or \"account again\" in prompt or \"does not account for\" in prompt):",
    );
    std::fs::write(&agy, script).unwrap();
    let core = Core::open(&project.join(".antislop"), config(&codex, &agy)).unwrap();
    let conversation = run_plan(&core, &project);
    let task = original_tasks(&core, &conversation)[0].clone();
    assert_eq!(task.status, "needs_user");
    let (blocked, _) = card(&core, &conversation, "t1");
    assert_eq!(blocked["integration"]["outcome"], "user_edits");

    // The user resolves the block; a retry with guidance starts a new round.
    std::fs::remove_file(project.join("hello.txt")).unwrap();
    core.recover_task(antislop::lean::TaskRecoveryRequest {
        conversation: conversation.clone(),
        plan: task.plan_id.clone(),
        task: "t1".into(),
        action: antislop::lean::RecoveryAction::RetryWithGuidance,
        consent_files: None,
        freshness_token: core
            .task_freshness_token(&conversation, &task.plan_id, &task.id)
            .unwrap(),
        message: Some("first ask the user".into()),
    })
    .unwrap();
    wait_idle(&core, &conversation);

    let task = core.task(&task.plan_id, "t1").unwrap();
    assert_eq!(task.status, "approved", "{:?}", task.report);
    assert!(task.round >= 2, "the retry ran a later round");
    let events: Vec<Value> = core
        .store
        .events_since(&conversation, 0, 10_000)
        .unwrap()
        .into_iter()
        .filter(|event| event.kind == "task" && event.payload["id"] == "t1")
        .map(|event| event.payload)
        .collect();
    // The retry's own announcements carry no earlier round's card data.
    let queued = events
        .iter()
        .rfind(|event| event["status"] == "queued")
        .expect("the retry queued the task");
    assert_eq!(queued["feedback"], serde_json::Value::Null);
    for event in events.iter().filter(|e| e["round"].as_u64() >= Some(2)) {
        if event["status"] != "approved" {
            assert_ne!(
                event["feedback"]["integration"]["outcome"], "user_edits",
                "{event}"
            );
        }
    }
    let (feedback, _) = card(&core, &conversation, "t1");
    assert_eq!(feedback["round"], task.round);
    assert_eq!(feedback["integration"]["outcome"], "applied");
}

#[test]
fn a_recovery_integration_names_itself_on_the_card() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let (codex, agy) = fake_clis_with_concurrent_edit(dir.path(), &project);
    let core = Core::open(&project.join(".antislop"), config(&codex, &agy)).unwrap();
    let conversation = run_plan(&core, &project);
    let task = original_tasks(&core, &conversation)[0].clone();
    assert_eq!(task.status, "needs_user");

    // The user consents away their edit and lets the runtime land the work.
    std::fs::write(project.join("hello.txt"), "from worker\n").unwrap();
    let recovery = core
        .recover_task(antislop::lean::TaskRecoveryRequest {
            conversation: conversation.clone(),
            plan: task.plan_id.clone(),
            task: "t1".into(),
            action: antislop::lean::RecoveryAction::IntegrateWithConsent,
            consent_files: Some(vec!["hello.txt".into()]),
            freshness_token: core
                .task_freshness_token(&conversation, &task.plan_id, &task.id)
                .unwrap(),
            message: None,
        })
        .unwrap();
    assert!(recovery.integrated_files.contains(&"hello.txt".to_owned()));

    let (feedback, _) = card(&core, &conversation, "t1");
    assert_eq!(feedback["integration"]["outcome"], "applied");
    assert_eq!(feedback["integration"]["source"], "recovery");
    assert_eq!(
        feedback["integration"]["files"],
        serde_json::json!(["hello.txt"])
    );
    let kept = core
        .recover_task(antislop::lean::TaskRecoveryRequest {
            conversation: conversation.clone(),
            plan: task.plan_id.clone(),
            task: "t1".into(),
            action: antislop::lean::RecoveryAction::ForceApprove,
            consent_files: None,
            freshness_token: core
                .task_freshness_token(&conversation, &task.plan_id, &task.id)
                .unwrap(),
            message: None,
        })
        .is_err();
    assert!(kept, "an approved task needs no further recovery");
}

#[test]
fn keeping_the_branch_by_recovery_reports_the_branch_on_the_card() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let (codex, agy) = fake_clis_with_concurrent_edit(dir.path(), &project);
    let core = Core::open(&project.join(".antislop"), config(&codex, &agy)).unwrap();
    let conversation = run_plan(&core, &project);
    let task = original_tasks(&core, &conversation)[0].clone();
    // Simulate a worktree created by the legacy short-name runtime.
    let branch = format!("antislop/{}", &task.id[..8]);
    let worktree = project.join(".antislop/worktrees").join(&task.id);
    git(&worktree, &["branch", "-m", &branch]);

    core.recover_task(antislop::lean::TaskRecoveryRequest {
        conversation: conversation.clone(),
        plan: task.plan_id.clone(),
        task: "t1".into(),
        action: antislop::lean::RecoveryAction::KeepOnBranch,
        consent_files: None,
        freshness_token: core
            .task_freshness_token(&conversation, &task.plan_id, &task.id)
            .unwrap(),
        message: None,
    })
    .unwrap();
    let (feedback, _) = card(&core, &conversation, "t1");
    assert_eq!(feedback["integration"]["outcome"], "left_on_branch");
    assert_eq!(feedback["integration"]["source"], "recovery");
    assert_eq!(feedback["integration"]["branch"], branch);
    // The work is on the branch, and the user's file is untouched.
    assert_eq!(
        git(&project, &["show", &format!("{branch}:hello.txt")]),
        "from worker"
    );
    assert_eq!(
        std::fs::read_to_string(project.join("hello.txt")).unwrap(),
        "user draft\n"
    );
}

#[test]
fn a_force_approval_puts_the_users_word_on_the_card() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let (codex, agy) = fake_clis_with_concurrent_edit(dir.path(), &project);
    let core = Core::open(&project.join(".antislop"), config(&codex, &agy)).unwrap();
    let conversation = run_plan(&core, &project);
    let task = original_tasks(&core, &conversation)[0].clone();
    assert_eq!(task.status, "needs_user");

    let error = core
        .recover_task(antislop::lean::TaskRecoveryRequest {
            conversation: conversation.clone(),
            plan: task.plan_id.clone(),
            task: "t1".into(),
            action: antislop::lean::RecoveryAction::ForceApprove,
            consent_files: None,
            freshness_token: core
                .task_freshness_token(&conversation, &task.plan_id, &task.id)
                .unwrap(),
            message: Some("the draft is fine".into()),
        })
        .unwrap_err();
    assert!(error.to_string().contains("not integrated"));
    assert_eq!(core.task(&task.plan_id, "t1").unwrap().status, "needs_user");
    assert_eq!(
        std::fs::read_to_string(project.join("hello.txt")).unwrap(),
        "user draft\n"
    );
}

#[test]
fn cancelling_a_dependency_cannot_discard_the_dependent_task() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let (codex, agy) = fake_clis_with_concurrent_edit(dir.path(), &project);
    // The plan now has a second task waiting on the first.
    let script = std::fs::read_to_string(&codex).unwrap().replace(
        r#""verifier": {"focus": "file content"}}]}"#,
        r#""verifier": {"focus": "file content"}}, {"id": "t2", "title": "Extend greeting",
           "goal": "Extend the greeting", "writes": True, "scope": ["."],
           "kind": "implementation", "verifier": {"focus": "file content"},
           "depends_on": ["t1"]}]}"#,
    );
    std::fs::write(&codex, script).unwrap();
    let core = Core::open(&project.join(".antislop"), config(&codex, &agy)).unwrap();
    let conversation = run_plan(&core, &project);
    let task = original_tasks(&core, &conversation)[0].clone();
    assert_eq!(task.status, "needs_user");
    // t2 waited on t1, never ran, and the scheduler skipped it once t1 blocked.
    assert_eq!(core.task(&task.plan_id, "t2").unwrap().status, "queued");

    let error = core
        .recover_task(antislop::lean::TaskRecoveryRequest {
            conversation: conversation.clone(),
            plan: task.plan_id.clone(),
            task: "t1".into(),
            action: antislop::lean::RecoveryAction::CancelTask,
            consent_files: None,
            freshness_token: core
                .task_freshness_token(&conversation, &task.plan_id, &task.id)
                .unwrap(),
            message: None,
        })
        .unwrap_err();
    assert!(error.to_string().contains("cannot be discarded"));
    assert_eq!(core.task(&task.plan_id, "t1").unwrap().status, "needs_user");
    assert_eq!(core.task(&task.plan_id, "t2").unwrap().status, "queued");
}

/// A worker effect longer than the card's byte budget is cut on whole
/// grapheme clusters — a ZWJ family survives intact — and the card gets
/// the runtime's explicit `effect_cut` flag at the moment of the cut,
/// not something a viewer has to read off the text's last character.
#[test]
fn a_cut_stated_effect_is_marked_by_metadata_not_only_by_the_mark() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let (codex, agy) = fake_clis_answered(
        dir.path(),
        "\"wrote hello.txt\\n\\n```effect\\n\" + \"\\U0001F468\\u200D\\U0001F469\\u200D\\U0001F467\\u200D\\U0001F466 \" * 12 + \"\\n```\"",
    );
    let core = Core::open(&project.join(".antislop"), config(&codex, &agy)).unwrap();
    let conversation = run_plan(&core, &project);
    let (feedback, from_event) = card(&core, &conversation, "t1");
    let effect = feedback["effect"].as_str().expect("an effect");
    // Bytes are the budget: the whole is at most EFFECT_LIMIT, never a
    // split cluster: every ZWJ belongs to a complete family.
    assert!(
        effect.len() <= antislop::lean::tasks::EFFECT_LIMIT,
        "{}",
        effect.len()
    );
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";
    assert!(
        effect.ends_with('…'),
        "a cut effect ends with the mark: {effect}"
    );
    let families = effect.matches(family).count();
    assert!(families >= 1, "{effect:?}");
    assert_eq!(
        effect.matches('\u{200D}').count(),
        families * 3,
        "no lone joiners: a cluster was never split"
    );
    // The cut is metadata the runtime stamped when it cut, carried whole
    // into the announced event.
    assert_eq!(feedback["effect_cut"], serde_json::json!(true));
    assert_eq!(from_event, feedback);
    let task = core
        .store
        .latest_plan(&conversation)
        .unwrap()
        .into_iter()
        .find(|task| task.spec["id"] == "t1")
        .unwrap();
    let report = task.report.as_deref().unwrap();
    assert!(!report.contains("```effect"), "block removed: {report}");
    assert!(report.contains("wrote hello.txt"), "{report}");
}

/// Honest card text that merely ends in an ellipsis was not cut: the
/// metadata says so even though the old "recognise a cut by its trailing
/// mark" reading would have been wrong.
#[test]
fn an_honest_ellipsis_is_not_reported_as_a_cut() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let (codex, agy) = fake_clis_answered(
        dir.path(),
        "\"wrote hello.txt\\n\\n```effect\\ndone…\\n```\"",
    );
    let core = Core::open(&project.join(".antislop"), config(&codex, &agy)).unwrap();
    let conversation = run_plan(&core, &project);
    let (feedback, from_event) = card(&core, &conversation, "t1");
    assert_eq!(feedback["effect"], "done…");
    assert_eq!(feedback["effect_cut"], serde_json::json!(false));
    assert_eq!(from_event, feedback);
}

#[test]
fn integration_repair_delivers_options_and_refuses_a_silent_variant_switch() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let (codex, agy) = fake_clis_with_concurrent_edit(dir.path(), &project);
    // Without the binding and dependency data the simulated repair silently
    // chooses an unrelated alternative. With them it reports the conflict as
    // an explicit question instead of silently changing the selected variant.
    let script = std::fs::read_to_string(&agy).unwrap().replace(
        "answer = \"```integration_question\\nuncommitted edits in hello.txt: keep the user draft or accept the worker greeting?\\n```\"",
        r#"if "Selected: direct fixture change" in prompt and "Never silently switch variants" in prompt:
        answer = "```integration_question\nSilent variant switch rejected: the alternative conflicts with the selected direct fixture change. Which variant should be used?\n```"
    else:
        with open("hello.txt", "w") as repaired:
            repaired.write("unjustified alternative variant\n")
        answer = "Repaired.""#,
    );
    std::fs::write(&agy, script).unwrap();
    let core = Core::open(&project.join(".antislop"), config(&codex, &agy)).unwrap();
    let conversation = run_plan(&core, &project);
    let task = original_tasks(&core, &conversation)[0].clone();
    assert_eq!(task.status, "needs_user", "{:?}", task.report);
    assert!(
        task.report
            .as_deref()
            .unwrap()
            .contains("Silent variant switch rejected")
    );
    assert_eq!(
        std::fs::read_to_string(project.join("hello.txt")).unwrap(),
        "user draft\n"
    );
    let calls = std::fs::read_to_string(dir.path().join("calls.jsonl")).unwrap();
    let repair: Value = calls
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|call| {
            call["args"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()
                .as_str()
                .unwrap()
                .contains("Automatic integration conflict")
        })
        .unwrap();
    let prompt = repair["args"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()
        .as_str()
        .unwrap();
    assert!(prompt.contains("Selected: direct fixture change"));
    assert!(prompt.contains("options report is binding"));
    assert!(prompt.contains("Never silently switch variants"));
    assert!(prompt.contains("justify a departure with evidence"));
}

use architect::{config::Config, lean::Core};
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
    ids = re.findall(r"^- ([RC][0-9]+): ", prompt, re.M)
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
        std::fs::set_permissions(script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
    }
    (codex, agy)
}

fn config(codex: &Path, agy: &Path) -> Config {
    config_with(codex, agy, |text| text)
}

fn config_with(codex: &Path, agy: &Path, change: impl Fn(String) -> String) -> Config {
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
    let conversation = core.start(project).unwrap();
    core.send(&conversation.id, "dodaj powitanie").unwrap();
    wait_idle(core, &conversation.id);
    let plan = core.store.latest_plan(&conversation.id).unwrap()[0]
        .plan_id
        .clone();
    core.accept_plan(&conversation.id, &plan).unwrap();
    wait_idle(core, &conversation.id);
    conversation.id
}

fn wait_idle(core: &Core, conversation: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !core.is_idle(conversation) {
        assert!(Instant::now() < deadline, "conversation never went idle");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn an_approved_writer_lands_in_the_project_without_a_commit() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let base = git(&project, &["rev-parse", "HEAD"]);
    let (codex, agy) = fake_clis(dir.path());
    let core = Core::open(&project.join(".antislop"), config(&codex, &agy)).unwrap();
    let conversation = run_plan(&core, &project);

    let task = &core.store.latest_plan(&conversation).unwrap()[0];
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
fn a_writer_never_touches_files_the_user_is_editing() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    std::fs::write(project.join("hello.txt"), "user draft\n").unwrap();
    let (codex, agy) = fake_clis(dir.path());
    let core = Core::open(&project.join(".antislop"), config(&codex, &agy)).unwrap();
    let conversation = run_plan(&core, &project);

    let task = &core.store.latest_plan(&conversation).unwrap()[0];
    assert_eq!(task.status, "needs_user");
    let report = task.report.as_deref().unwrap();
    assert!(
        report.contains("uncommitted edits in hello.txt"),
        "{report}"
    );
    assert_eq!(
        std::fs::read_to_string(project.join("hello.txt")).unwrap(),
        "user draft\n"
    );
    let branch = format!("antislop/{}", &task.id[..8]);
    assert_eq!(
        git(&project, &["show", &format!("{branch}:hello.txt")]),
        "from worker"
    );
}

#[test]
fn resuming_integrates_an_approved_change_once_the_user_is_done() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    std::fs::write(project.join("hello.txt"), "user draft\n").unwrap();
    let (codex, agy) = fake_clis(dir.path());
    let core = Core::open(&project.join(".antislop"), config(&codex, &agy)).unwrap();
    let conversation = run_plan(&core, &project);
    let task = core.store.latest_plan(&conversation).unwrap()[0].clone();
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

    let task = &core.store.plan(&task.plan_id).unwrap()[0];
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
    assert_eq!(calls_after, calls_before, "no new worker turn");
    assert!(core.resume_plan(&conversation, &task.plan_id).is_err());
}

#[test]
fn the_user_reads_a_change_comments_a_line_and_asks_for_a_review() {
    let dir = tempfile::tempdir().unwrap();
    let project = project(dir.path());
    let (codex, agy) = fake_clis(dir.path());
    let core = Core::open(&project.join(".antislop"), config(&codex, &agy)).unwrap();
    let conversation = run_plan(&core, &project);
    let task = core.store.latest_plan(&conversation).unwrap().remove(0);
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

    core.comment(&conversation, "diff:t1:hello.txt:1", "dodaj imię")
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
    assert!(prompt.contains("dodaj imię") && prompt.contains("line 1 of hello.txt"));
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

    let task = &core.store.latest_plan(&conversation).unwrap()[0];
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

    let task = &core.store.latest_plan(&conversation).unwrap()[0];
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

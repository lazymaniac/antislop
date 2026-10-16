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
  "writes": True, "kind": "implementation", "verifier": {"focus": "file content"}}]}
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
if "You verify" in prompt:
    session = "verifier"
    answer = "```verdict\n" + json.dumps({"approved": "git diff" in prompt, "findings": []}) + "\n```"
else:
    session = "worker"
    with open("hello.txt", "w") as f:
        f.write("from worker\n")
    answer = "wrote hello.txt"
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
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/architect.toml"))
        .unwrap()
        .replace("/opt/homebrew/bin/codex", &codex.to_string_lossy())
        .replace("/Users/sebastian/.local/bin/agy", &agy.to_string_lossy());
    Config::from_toml(&text).unwrap()
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
    // local_commits = false in architect.toml: applied, not committed.
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
    let agents: Vec<_> = calls.iter().filter(|call| call["bin"] == "agy").collect();
    assert_eq!(agents.len(), 2);
    let worktree = agents[0]["cwd"].as_str().unwrap();
    assert!(worktree.contains("/.antislop/worktrees/"), "{worktree}");
    assert_eq!(
        agents[1]["cwd"], agents[0]["cwd"],
        "verifier reads the worktree"
    );
    let worker_args = agents[0]["args"].to_string();
    assert!(worker_args.contains("accept-edits") && worker_args.contains("--sandbox"));
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

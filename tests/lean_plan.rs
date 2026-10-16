use architect::{config::Config, lean::Core};
use serde_json::Value;
use std::path::Path;
use std::time::{Duration, Instant};

const CALLS: &str = "calls.jsonl";

/// Codex-shaped orchestrator: proposes a two-task plan, then summarizes the
/// runtime's results. Agy-shaped worker/verifier: the verifier rejects the
/// storage task once, then approves the worker's fix.
fn fake_clis(dir: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let log = dir.join(CALLS);
    let common = format!(
        r#"#!/usr/bin/env python3
import json, sys
prompt = sys.argv[-1]
with open({log:?}, "a") as f:
    f.write(json.dumps({{"bin": sys.argv[0].rsplit("/", 1)[-1], "args": sys.argv[1:]}}) + "\n")
"#,
        log = log.to_string_lossy()
    );
    let codex = dir.join("codex");
    std::fs::write(
        &codex,
        common.clone()
            + r#"
plan = {"tasks": [
  {"id": "t1", "title": "Storage", "goal": "Assess storage", "scope": ["src"],
   "kind": "architecture", "verifier": {"focus": "claims", "risks": ["r-storage"]}},
  {"id": "t2", "title": "TUI", "goal": "Assess TUI", "depends_on": ["t1"],
   "kind": "analysis", "verifier": {"focus": "claims", "risks": ["r-tui"]}}]}
if "[runtime]" in prompt:
    answer = "summary: " + ("fixed report" if "fixed report" in prompt else "missing")
else:
    answer = "Dwa zadania.\n```plan\n" + json.dumps(plan) + "\n```"
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
def verdict(approved, findings):
    return "ok\n```verdict\n" + json.dumps({"approved": approved, "findings": findings}) + "\n```"
REVIEW = ("You review a plan", "new plan:", "You audit", "next plan:")
if any(marker in prompt for marker in REVIEW):
    session = "reviewer"
    answer = "```verdict\n{\"approved\": true, \"findings\": []}\n```"
elif "You verify" in prompt:
    session = "verifier"
    approved = "Storage" not in prompt
    answer = verdict(approved, [] if approved else [{"severity": "high", "issue": "wrong claim", "where": "src/a.rs:1"}])
elif "Your verifier found problems" in prompt:
    session, answer = "worker", "fixed report"
elif "updated report" in prompt:
    session, answer = "verifier", verdict(True, [])
else:
    session, answer = "worker", "first report for " + prompt.split("Task: ")[1].split("\n")[0]
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
    Config::from_toml(&text).unwrap()
}

fn wait_idle(core: &Core, conversation: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !core.is_idle(conversation) {
        assert!(Instant::now() < deadline, "conversation never went idle");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn calls(dir: &Path) -> Vec<(String, String)> {
    std::fs::read_to_string(dir.join(CALLS))
        .unwrap()
        .lines()
        .map(|line| {
            let call: Value = serde_json::from_str(line).unwrap();
            let args = call["args"].as_array().unwrap();
            (
                call["bin"].as_str().unwrap().to_owned(),
                args.iter()
                    .map(|arg| arg.as_str().unwrap())
                    .collect::<Vec<_>>()
                    .join(" "),
            )
        })
        .collect()
}

fn is_reviewer(args: &str) -> bool {
    args.contains("You review a plan") || args.contains("You audit")
}

fn is_verifier(args: &str) -> bool {
    !is_reviewer(args) && (args.contains("You verify") || args.contains("Check it again"))
}

#[test]
fn a_plan_runs_worker_verifier_pairs_and_reports_back() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir_all(project.join("src")).unwrap();
    let (codex, agy) = fake_clis(dir.path());
    let core = Core::open(&dir.path().join("state"), config(&codex, &agy)).unwrap();
    let conversation = core.start(&project).unwrap();

    core.send(&conversation.id, "przeanalizuj architekturę projektu")
        .unwrap();
    wait_idle(&core, &conversation.id);
    let messages = core.store.messages_after(&conversation.id, 0).unwrap();
    assert_eq!(messages[1].text, "Dwa zadania.", "plan block is not shown");
    let plan = core.store.latest_plan(&conversation.id).unwrap()[0]
        .plan_id
        .clone();
    core.accept_plan(&conversation.id, &plan).unwrap();
    wait_idle(&core, &conversation.id);

    let tasks = core.store.latest_plan(&conversation.id).unwrap();
    let summary: Vec<_> = tasks
        .iter()
        .map(|task| (task.status.as_str(), task.round))
        .collect();
    assert_eq!(summary, [("approved", 2), ("approved", 1)]);
    assert_eq!(tasks[0].report.as_deref(), Some("fixed report"));

    let reply = core
        .store
        .messages_after(&conversation.id, 0)
        .unwrap()
        .into_iter()
        .rev()
        .find(|message| message.author == "orchestrator")
        .unwrap();
    assert_eq!(reply.text, "summary: fixed report");

    let calls = calls(dir.path());
    let workers: Vec<_> = calls
        .iter()
        .filter(|(bin, args)| bin == "agy" && !is_verifier(args) && !is_reviewer(args))
        .collect();
    let verifiers: Vec<_> = calls
        .iter()
        .filter(|(bin, args)| bin == "agy" && is_verifier(args))
        .collect();
    assert!(
        workers
            .iter()
            .all(|(_, args)| !args.contains("r-storage") && !args.contains("r-tui"))
    );
    assert!(workers.iter().all(|(_, args)| args.contains("--mode plan")));
    let storage_verifier = &verifiers[0].1;
    assert!(storage_verifier.contains("r-storage") && !storage_verifier.contains("r-tui"));
    assert!(
        verifiers[1].1.contains("--conversation verifier"),
        "re-check resumes"
    );
    let reviews: Vec<_> = calls
        .iter()
        .filter(|(_, args)| is_reviewer(args))
        .map(|(_, args)| args.contains("You audit"))
        .collect();
    assert_eq!(reviews, [false, true], "one plan review, then one audit");
    let tui_verifier = &verifiers.last().unwrap().1;
    assert!(tui_verifier.contains("r-tui") && !tui_verifier.contains("r-storage"));
}

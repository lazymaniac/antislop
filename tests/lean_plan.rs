use antislop::{config::Config, lean::Core};
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
  {"id": "t1", "title": "Storage", "summary": "Oceń sposób zapisu danych.", "goal": "Assess storage", "scope": ["src"],
   "kind": "architecture", "verifier": {"summary": "Sprawdź wnioski o zapisie danych.", "focus": "claims", "risks": ["r-storage"]}},
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
def cover(answer):
    import re
    ids = re.findall(r"^- ([RCV][0-9]+): ", prompt, re.M)
    if not ids or "```verdict" not in answer:
        return answer
    head, body = answer.rsplit("```verdict", 1)
    data = json.loads(body.split("```")[0])
    data["coverage"] = {i: {"status": "ok", "note": "checked"} for i in ids}
    return head + "```verdict\n" + json.dumps(data) + "\n```"
def verdict(approved, findings):
    return "ok\n```verdict\n" + json.dumps({"approved": approved, "findings": findings}) + "\n```"
REVIEW = ("You review a plan", "new plan:", "You audit", "next plan:")
if any(marker in prompt for marker in REVIEW):
    session = "reviewer"
    answer = "```verdict\n{\"approved\": true, \"findings\": []}\n```"
elif "You verify" in prompt:
    session = "verifier"
    approved = "Task: Storage" not in prompt
    answer = verdict(approved, [] if approved else [{"severity": "high", "issue": "wrong claim", "where": "src/a.rs:1"}])
elif "Your verifier found problems" in prompt:
    session, answer = "worker", "fixed report"
elif "updated report" in prompt:
    session, answer = "verifier", verdict(True, [])
else:
    session, answer = "worker", "first report for " + prompt.split("Task: ")[1].split("\n")[0]
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

fn wait_idle(core: &Core, conversation: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !core.is_idle(conversation) {
        assert!(Instant::now() < deadline, "conversation never went idle");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn plan_summaries_survive_core_storage_and_event_replay() {
    use antislop::lean::{store::Store, tui::view::View};
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let (codex, agy) = fake_clis(dir.path());
    let core = Core::open(&state, config(&codex, &agy)).unwrap();
    let conversation = core.start(dir.path()).unwrap();
    core.send(&conversation.id, "Oceń zapis danych i interfejs.")
        .unwrap();
    wait_idle(&core, &conversation.id);
    let original = core.store.latest_plan(&conversation.id).unwrap();
    assert_eq!(original[0].spec["summary"], "Oceń sposób zapisu danych.");
    assert_eq!(
        original[0].spec["verifier"]["summary"],
        "Sprawdź wnioski o zapisie danych."
    );
    assert!(
        original[1].spec.get("summary").is_none(),
        "legacy task remains readable"
    );
    drop(core);
    let store = Store::open(&state.join("core.db")).unwrap();
    assert_eq!(store.latest_plan(&conversation.id).unwrap(), original);
    let events = store.events_since(&conversation.id, 0, 1000).unwrap();
    let mut replay = View::default();
    for event in &events {
        replay.apply(&serde_json::to_value(event).unwrap());
    }
    assert_eq!(
        replay.tasks[0].summary.as_deref(),
        Some("Oceń sposób zapisu danych.")
    );
    assert_eq!(
        replay.tasks[0].verifier_summary.as_deref(),
        Some("Sprawdź wnioski o zapisie danych.")
    );
    assert!(replay.tasks[1].summary.is_none());
    assert!(replay.tasks[1].verifier_summary.is_none());
    assert_eq!(replay.tasks[0].goal, "Assess storage");
    assert_eq!(replay.tasks[0].focus, "claims");
    assert_eq!(replay.tasks[0].risks, ["r-storage"]);
}

#[test]
fn a_plan_comment_gets_an_independent_revision_and_verification() {
    let dir = tempfile::tempdir().unwrap();
    let (codex, agy) = fake_clis(dir.path());
    let core = Core::open(&dir.path().join("state"), config(&codex, &agy)).unwrap();
    let conversation = core.start(dir.path()).unwrap();
    core.send(&conversation.id, "Assess storage and TUI")
        .unwrap();
    wait_idle(&core, &conversation.id);
    let initial = core.store.latest_plan(&conversation.id).unwrap()[0]
        .plan_id
        .clone();
    let comment = core
        .comment_scoped(
            &conversation.id,
            Some(&initial),
            &format!("plan:{initial}:line:2"),
            "Explain storage verification in more detail",
        )
        .unwrap();
    // The comment waits as a draft: the agent starts when the user sends the
    // review, and the plan cannot be accepted meanwhile.
    assert_eq!(comment.status, "draft");
    wait_idle(&core, &conversation.id);
    assert_eq!(
        core.store.latest_plan(&conversation.id).unwrap()[0].plan_id,
        initial
    );
    assert!(
        core.store
            .last_turn(&conversation.id, antislop::lean::COMMENT_ORCHESTRATOR)
            .unwrap()
            .is_none()
    );
    let refused = core.accept_plan(&conversation.id, &initial).unwrap_err();
    assert!(refused.to_string().contains("Review"), "{refused}");
    assert!(
        core.set_comment(&conversation.id, comment.id, "open")
            .unwrap_err()
            .to_string()
            .contains("draft")
    );
    assert_eq!(core.review_comments(&conversation.id, &initial).unwrap(), 1);
    wait_idle(&core, &conversation.id);
    let comment = core
        .store
        .comments(&conversation.id, "resolved")
        .unwrap()
        .into_iter()
        .find(|entry| entry.id == comment.id)
        .expect("resolved after the revision");
    let revised = core.store.latest_plan(&conversation.id).unwrap()[0]
        .plan_id
        .clone();
    assert_ne!(initial, revised);
    assert!(
        core.store
            .comments(&conversation.id, "resolved")
            .unwrap()
            .iter()
            .any(|entry| entry.id == comment.id)
    );
    let turn = core
        .store
        .last_turn(&conversation.id, antislop::lean::COMMENT_ORCHESTRATOR)
        .unwrap()
        .unwrap();
    assert_eq!(turn.status, antislop::lean::store::TurnStatus::Done);
    let calls = calls(dir.path());
    let revision = calls
        .iter()
        .find(|(binary, prompt)| {
            binary == "codex" && prompt.contains("Address each new user comment")
        })
        .unwrap();
    assert!(revision.1.contains(&format!("Referenced plan {initial}")));
    assert!(revision.1.contains("Assess storage and TUI"));
    assert!(
        revision
            .1
            .contains("Explain storage verification in more detail")
    );
    let review = core.store.plan_history(&conversation.id, &revised).unwrap();
    assert!(
        review
            .turns
            .iter()
            .any(|turn| turn.role == "plan-verifier" && turn.status == "done")
    );
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

#[test]
fn one_review_sends_all_draft_comments_to_a_single_turn() {
    let dir = tempfile::tempdir().unwrap();
    let (codex, agy) = fake_clis(dir.path());
    let core = Core::open(&dir.path().join("state"), config(&codex, &agy)).unwrap();
    let conversation = core.start(dir.path()).unwrap();
    core.send(&conversation.id, "Assess storage and TUI")
        .unwrap();
    wait_idle(&core, &conversation.id);
    let plan = core.store.latest_plan(&conversation.id).unwrap()[0]
        .plan_id
        .clone();
    let mut ids = vec![];
    for (line, text) in [
        (2, "first remark"),
        (3, "second remark"),
        (4, "dropped remark"),
    ] {
        ids.push(
            core.comment_scoped(
                &conversation.id,
                Some(&plan),
                &format!("plan:{plan}:line:{line}"),
                text,
            )
            .unwrap()
            .id,
        );
    }
    // A draft the user deletes is never sent.
    let deleted = core
        .set_comment(&conversation.id, ids[2], "deleted")
        .unwrap();
    assert_eq!(deleted.status, "deleted");
    assert_eq!(core.review_comments(&conversation.id, &plan).unwrap(), 2);
    wait_idle(&core, &conversation.id);
    let turns = core
        .store
        .sessions(&conversation.id)
        .unwrap()
        .into_iter()
        .filter(|turn| turn.role == antislop::lean::COMMENT_ORCHESTRATOR)
        .count();
    assert_eq!(turns, 1, "both comments are answered in one turn");
    let prompt = calls(dir.path())
        .into_iter()
        .find(|(_, prompt)| prompt.contains("Address each new user comment"))
        .unwrap()
        .1;
    assert!(prompt.contains("first remark") && prompt.contains("second remark"));
    assert!(!prompt.contains("dropped remark"));
    // Nothing is left to send.
    assert!(core.review_comments(&conversation.id, &plan).is_err());
}

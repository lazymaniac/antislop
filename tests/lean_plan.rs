mod common;
mod support;
use antislop::{config::Config, lean::Core};
use serde_json::Value;
use std::path::Path;
use std::time::{Duration, Instant};

const CALLS: &str = "calls.jsonl";

#[test]
fn a_verifier_started_after_a_live_update_uses_the_new_profile_in_the_same_round() {
    let dir = tempfile::tempdir().unwrap();
    let (codex, agy) = fake_clis(dir.path());
    let gate = dir.path().join("release-worker");
    let marker = dir.path().join("worker-started");
    let source = std::fs::read_to_string(&agy).unwrap().replace(
        "else:\n    session, answer = \"worker\", \"first report for \"",
        &format!("else:\n    import os, time\n    open({:?}, 'w').close()\n    while not os.path.exists({:?}): time.sleep(0.01)\n    session, answer = \"worker\", \"first report for \"", marker.to_string_lossy(), gate.to_string_lossy()),
    );
    std::fs::write(&agy, source).unwrap();
    let initial = config(&codex, &agy);
    let core = Core::open(&dir.path().join("state"), initial.clone()).unwrap();
    let conversation = support::fixture_conversation(&core, dir.path()).unwrap();
    core.send(&conversation.id, "Assess storage and TUI")
        .unwrap();
    wait_idle(&core, &conversation.id);
    let task = core.store.latest_plan(&conversation.id).unwrap().remove(0);
    core.accept_plan(&conversation.id, &task.plan_id).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !marker.exists() {
        assert!(Instant::now() < deadline, "worker did not reach the gate");
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut updated = initial;
    updated.profiles.get_mut("review").unwrap().model = Some("live-verifier-model".into());
    core.replace_config(updated);
    std::fs::write(gate, "continue").unwrap();
    wait_idle(&core, &conversation.id);
    let calls = std::fs::read_to_string(dir.path().join(CALLS)).unwrap();
    let first_verifier: Value = calls
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|call| {
            let prompt = call["args"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()
                .as_str()
                .unwrap();
            prompt.contains("You verify") && prompt.contains("Task: Storage")
        })
        .unwrap();
    assert!(
        first_verifier["args"]
            .as_array()
            .unwrap()
            .iter()
            .any(|arg| arg == "live-verifier-model"),
        "{first_verifier}"
    );
}

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
  {"id": "t1", "title": "Storage", "summary": "Assess the storage approach.", "goal": "Assess storage", "scope": ["src"],
   "kind": "architecture", "verifier": {"summary": "Verify the storage findings.", "focus": "claims", "risks": ["r-storage"]}},
  {"id": "t2", "title": "TUI", "goal": "Assess TUI", "depends_on": ["t1"],
   "kind": "analysis", "verifier": {"focus": "claims", "risks": ["r-tui"]}}]}
if "[runtime]" in prompt:
    answer = "summary: " + ("fixed report" if "fixed report" in prompt else "missing")
else:
    answer = "Two tasks.\n```plan\n" + json.dumps(plan) + "\n```"
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
    ids = re.findall(r"^- ([A-Za-z0-9_.-]+): ", prompt, re.M)
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
    approved = "Task: Storage" not in prompt or "fixed report" in prompt
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
        support::install_ledger_protocol(script);
        std::fs::set_permissions(script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        common::interactive(script);
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
    let conversation = support::fixture_conversation(&core, dir.path()).unwrap();
    core.send(&conversation.id, "Assess storage and the interface.")
        .unwrap();
    wait_idle(&core, &conversation.id);
    let original = core.store.latest_plan(&conversation.id).unwrap();
    assert_eq!(original[0].spec["summary"], "Assess the storage approach.");
    assert_eq!(
        original[0].spec["verifier"]["summary"],
        "Verify the storage findings."
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
        Some("Assess the storage approach.")
    );
    assert_eq!(
        replay.tasks[0].verifier_summary.as_deref(),
        Some("Verify the storage findings.")
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
    let conversation = support::fixture_conversation(&core, dir.path()).unwrap();
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
    let conversation = support::fixture_conversation(&core, &project).unwrap();

    core.send(&conversation.id, "analyze the project architecture")
        .unwrap();
    wait_idle(&core, &conversation.id);
    let messages = core.store.messages_after(&conversation.id, 0).unwrap();
    let shown = messages
        .iter()
        .find(|message| message.author == "orchestrator")
        .expect("the plan's words reach the user");
    assert_eq!(shown.text, "Two tasks.", "plan block is not shown");
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
    assert!(workers.iter().all(|(_, args)| {
        if args.contains("Task: Storage") {
            args.contains("r-storage") && !args.contains("r-tui")
        } else {
            args.contains("r-tui") && !args.contains("r-storage")
        }
    }));
    assert!(workers.iter().all(|(_, args)| args.contains("--mode plan")));
    let storage_verifier = &verifiers[0].1;
    assert!(storage_verifier.contains("r-storage") && !storage_verifier.contains("r-tui"));
    assert!(
        !verifiers[1].1.contains("--conversation verifier"),
        "a verifier starts fresh in the next round"
    );
    assert!(verifiers[1].1.contains("Task: Storage"));
    assert!(verifiers[1].1.contains("fixed report"));
    let first_round = core.store.task_round(&tasks[0].id, 1).unwrap();
    assert_eq!(first_round["verdict"]["approved"], false);
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
    let conversation = support::fixture_conversation(&core, dir.path()).unwrap();
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

#[test]
fn malformed_plan_json_is_corrected_automatically_with_a_finite_retry_limit() {
    for (persistent, missing) in [(false, false), (true, false), (false, true), (true, true)] {
        let dir = tempfile::tempdir().unwrap();
        let (codex, agy) = fake_clis(dir.path());
        let source = std::fs::read_to_string(&codex).unwrap();
        let marker = "print(json.dumps({\"type\": \"thread.started\", \"thread_id\": \"orch\"}))";
        assert!(source.contains(marker));
        let hook = format!(
            r#"
import os
if not os.path.exists('format-error-sent') or {persistent}:
    open('format-error-sent','w').close()
    _ledger_emit('Draft ready.\n```plan\n{{"tasks":[{{"id":"t1","goal":"Text with an unescaped "quote""}}]}}\n```')
"#,
            persistent = if persistent { "True" } else { "False" }
        );
        let hook = if missing {
            hook.lines()
                .map(|line| {
                    if line.trim_start().starts_with("_ledger_emit(") {
                        "    _ledger_emit('```ledger\\n{\"entries\":[]}\\n```')"
                    } else {
                        line
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            hook
        };

        std::fs::write(&codex, source.replace(marker, &format!("{hook}\n{marker}"))).unwrap();
        let core = Core::open(&dir.path().join("state"), config(&codex, &agy)).unwrap();
        let conversation = support::fixture_conversation(&core, dir.path()).unwrap();
        core.send(&conversation.id, "Assess storage and TUI")
            .unwrap();
        wait_idle(&core, &conversation.id);
        let repairs = core
            .store
            .messages_by(&conversation.id, "plan-format-repair", 0)
            .unwrap();
        assert_eq!(repairs.len(), if persistent { 2 } else { 1 });
        let plan = core.store.latest_plan(&conversation.id).unwrap();
        if persistent {
            assert!(plan.is_empty());
        } else {
            assert!(!plan.is_empty());
            let turns = core.store.events_since(&conversation.id, 0, 2000).unwrap();
            assert_eq!(
                turns
                    .iter()
                    .filter(|e| e.kind == "turn_started"
                        && e.payload["role"] == "requirements-extraction-analyst")
                    .count(),
                1
            );
        }
    }
}

#[test]
fn restart_recovers_invalid_or_missing_plans_without_a_new_user_message() {
    for (answer, analyst_failed) in [
        ("```plan\n{broken JSON}\n```", false),
        ("```ledger\n{\"entries\":[]}\n```", false),
        ("I will prepare the plan.", false),
        (
            r#"```plan
{"tasks":[{"id":"t1","title":"Inspect","goal":"Inspect the project","kind":"analysis","verifier":{"focus":"Check findings"}}]}
```"#,
            true,
        ),
    ] {
        use antislop::lean::store::{Store, TurnStatus};
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        std::fs::create_dir(&state).unwrap();
        std::fs::set_permissions(&state, std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .unwrap();
        let (codex, agy) = fake_clis(dir.path());
        let store = Store::open(&state.join("core.db")).unwrap();
        let conversation = store
            .create_conversation(dir.path().to_str().unwrap())
            .unwrap();
        let message = store
            .add_message(&conversation.id, "user", "Assess storage and TUI")
            .unwrap();
        store
            .interview_summary(&conversation.id, "Assess storage and TUI", message.seq)
            .unwrap();
        let message = store.approve_requirements(&conversation.id).unwrap();
        let turn = store
            .start_turn(&conversation.id, "planner", "codex", message.seq, None)
            .unwrap();
        store.set_session(&turn.id, "orch").unwrap();
        store
            .append_event(
                &conversation.id,
                Some(&turn.id),
                None,
                "agent_final",
                &json!({"text":answer}),
            )
            .unwrap();
        store.finish_turn(&turn.id, TurnStatus::Done, None).unwrap();
        if analyst_failed {
            store
                .append_event(
                    &conversation.id,
                    None,
                    None,
                    "requirements_failed",
                    &json!({"error":"missing ledger JSON block"}),
                )
                .unwrap();
        }
        drop(store);
        let core = Core::open(&state, config(&codex, &agy)).unwrap();
        wait_idle(&core, &conversation.id);
        assert!(!core.store.latest_plan(&conversation.id).unwrap().is_empty());
        assert_eq!(
            core.store
                .messages_by(
                    &conversation.id,
                    if analyst_failed {
                        "requirements-format-repair"
                    } else {
                        "plan-format-repair"
                    },
                    0
                )
                .unwrap()
                .len(),
            1
        );
    }
}

#[test]
fn repairing_json_after_the_ledger_mapping_reuses_the_requirements() {
    let dir = tempfile::tempdir().unwrap();
    let (codex, agy) = fake_clis(dir.path());
    let source = std::fs::read_to_string(&codex).unwrap();
    let marker = "def _ledger_answer(answer):";
    assert!(source.contains(marker));
    let hook = r#"def _ledger_answer(answer):
    if 'Regenerate the task plan from the verified ledger' in _ledger_prompt and '```plan' in answer and not _lo.path.exists('post-ledger-format-error'):
        open('post-ledger-format-error','w').close()
        return '```plan\n{"tasks":[{unquoted: "bad"}]}\n```'
"#;
    std::fs::write(&codex, source.replace(marker, hook)).unwrap();
    let core = Core::open(&dir.path().join("state"), config(&codex, &agy)).unwrap();
    let conversation = support::fixture_conversation(&core, dir.path()).unwrap();
    core.send(&conversation.id, "Assess storage and TUI")
        .unwrap();
    wait_idle(&core, &conversation.id);
    assert!(dir.path().join("post-ledger-format-error").exists());
    assert!(!core.store.latest_plan(&conversation.id).unwrap().is_empty());
    assert_eq!(
        core.store
            .messages_by(&conversation.id, "plan-format-repair", 0)
            .unwrap()
            .len(),
        1
    );
    let events = core.store.events_since(&conversation.id, 0, 2000).unwrap();
    for (role, expected) in [
        ("requirements-extraction-analyst", 1),
        ("requirements-extraction-verifier", 0),
    ] {
        assert_eq!(
            events
                .iter()
                .filter(|e| e.kind == "turn_started" && e.payload["role"] == role)
                .count(),
            expected,
            "{role} must not repeat during JSON repair"
        );
    }
}

#[test]
fn orchestrator_extends_a_running_graph_before_the_pending_verification() {
    let dir = tempfile::tempdir().unwrap();
    let (codex, agy) = fake_clis(dir.path());
    let release = dir.path().join("release-worker");
    let started = dir.path().join("worker-started");
    let worker = std::fs::read_to_string(&agy).unwrap().replace(
        "else:\n    session, answer = \"worker\", \"first report for \"",
        &format!("else:\n    import os, time\n    if 'Task: Storage' in prompt:\n        open({:?}, 'w').close()\n        while not os.path.exists({:?}): time.sleep(0.01)\n    session, answer = \"worker\", \"first report for \"", started.to_string_lossy(), release.to_string_lossy()),
    );
    std::fs::write(&agy, worker).unwrap();
    let hook = r#"
if 'insert-extra' in prompt and 'Current task contracts for plan' in prompt:
    import re
    base, snapshot = re.search(r'Current task contracts for plan ([-\w]+):\n([^\n]+)', prompt).groups()
    snapshot = json.loads(snapshot)
    revision = snapshot['base_revision']
    snapshot = snapshot['tasks']
    extra = dict(snapshot[0])
    extra.update(id='extra', title='Extra', goal='Assess additional details', kind='analysis', writes=False, depends_on=['t1'])
    gate = dict(next(task for task in snapshot if task['id'] == 't2'))
    gate['depends_on'] = ['extra']
    import os,urllib.request
    body={'jsonrpc':'2.0','id':1,'method':'tools/call','params':{'name':'submit_plan_revision','arguments':{'turn':os.environ['FIXTURE_TURN'],'request_key':'insert-extra','revision':{'plan':base,'base_revision':revision,'reason':'Add focused analysis before downstream verification','tasks':[extra,gate],'retire':[]}}}}
    req=urllib.request.Request(os.environ['ANTISLOP_MCP_URL'],json.dumps(body).encode(),{'Authorization':'Bearer '+os.environ['ANTISLOP_MCP_TOKEN'],'Content-Type':'application/json'})
    reply=json.load(urllib.request.urlopen(req,timeout=10))
    assert not reply['result'].get('isError'),reply
    answer='Revision submitted for independent review.'
    print(json.dumps({'type':'thread.started','thread_id':'orch'}))
    print(json.dumps({'type':'item.completed','item':{'type':'agent_message','text':answer}}))
    print(json.dumps({'type':'turn.completed','usage':{}}))
    sys.exit(0)
"#;
    let orchestrator = std::fs::read_to_string(&codex).unwrap().replace(
        "plan = {\"tasks\": [",
        &format!("{hook}\nplan = {{\"tasks\": ["),
    );
    std::fs::write(&codex, orchestrator).unwrap();
    let core = Core::open(&dir.path().join("state"), config(&codex, &agy)).unwrap();
    let conversation = support::fixture_conversation(&core, dir.path()).unwrap();
    core.send(&conversation.id, "Assess storage and TUI")
        .unwrap();
    wait_idle(&core, &conversation.id);
    let before = core.store.latest_plan(&conversation.id).unwrap();
    let plan = &before[0].plan_id;
    core.accept_plan(&conversation.id, plan).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while !started.exists() {
        assert!(Instant::now() < deadline, "worker did not start");
        std::thread::sleep(Duration::from_millis(10));
    }
    core.send(&conversation.id, "insert-extra").unwrap();
    while core.store.plan(plan).unwrap().len() < 3 {
        assert!(
            Instant::now() < deadline,
            "graph was not extended: {:?}",
            core.store.events_since(&conversation.id, 0, 10000).unwrap()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let extended = core.store.plan(plan).unwrap();
    assert_eq!(extended[0].id, before[0].id);
    assert_eq!(extended[1].spec["id"], "extra");
    assert_eq!(extended[2].id, before[1].id);
    assert_eq!(extended[2].spec["depends_on"], serde_json::json!(["extra"]));
    assert_eq!(extended[2].round, 0);
    std::fs::write(release, "continue").unwrap();
    wait_idle(&core, &conversation.id);
    let finished = core.store.plan(plan).unwrap();
    assert!(
        finished.iter().all(|task| task.status == "approved"),
        "{finished:?}"
    );
    let events = core.store.events_since(&conversation.id, 0, 10000).unwrap();
    let starts = events
        .iter()
        .filter(|event| event.kind == "turn_started")
        .filter_map(|event| event.payload["role"].as_str())
        .collect::<Vec<_>>();
    let extra = starts
        .iter()
        .position(|role| **role == format!("worker:{}", finished[1].id))
        .unwrap();
    let downstream = starts
        .iter()
        .position(|role| **role == format!("worker:{}", finished[2].id))
        .unwrap();
    assert!(extra < downstream);
    // The first draft is revised into the ledger-mapped plan; extending the
    // running graph adds no further plan.
    assert_eq!(
        core.store
            .plans_page(&conversation.id, 0, 10)
            .unwrap()
            .total,
        2
    );
}

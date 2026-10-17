use architect::{config::Config, lean::Core};
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Fake codex (orchestrator) and agy (everyone else). Both log every prompt
/// to `prompts.jsonl`; `answer` comes from the given Python statements.
fn open(dir: &Path, orchestrator: &str, agent: &str) -> Arc<Core> {
    let log = dir.join("prompts.jsonl");
    let header = format!(
        r#"#!/usr/bin/env python3
import json, sys, time
prompt = sys.argv[-1]
with open({log:?}, "a") as f:
    f.write(json.dumps({{"bin": sys.argv[0].rsplit("/", 1)[-1], "prompt": prompt}}) + "\n")
def verdict(approved):
    import re
    ids = re.findall(r"^- ([RCV][0-9]+): ", prompt, re.M)
    coverage = {{i: {{"status": "ok", "note": "checked"}} for i in ids}}
    return "```verdict\n" + json.dumps({{"approved": approved, "findings": [], "coverage": coverage}}) + "\n```"
def task(id, title, depends=()):
    return {{"id": id, "title": title, "goal": "g", "depends_on": list(depends),
            "kind": "analysis", "verifier": {{"focus": "f"}}}}
def plan(*tasks):
    return "Plan.\n```plan\n" + json.dumps({{"tasks": list(tasks)}}) + "\n```"
def message(to, kind, text):
    return "\n```message\n" + json.dumps({{"to": to, "kind": kind, "text": text}}) + "\n```"
"#,
        log = log.to_string_lossy()
    );
    let write = |name: &str, body: String| {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        path
    };
    let codex = write(
        "codex",
        format!(
            "{header}{orchestrator}\nprint(json.dumps({{\"type\": \"thread.started\", \"thread_id\": \"orch\"}}))\n\
             print(json.dumps({{\"type\": \"item.completed\", \"item\": {{\"type\": \"agent_message\", \"text\": answer}}}}))\n\
             print(json.dumps({{\"type\": \"turn.completed\", \"usage\": {{}}}}))\n"
        ),
    );
    let agy = write(
        "agy",
        format!(
            "{header}\nreview = any(m in prompt for m in (\"You review a plan\", \"You audit\", \"You verify\"))\n\
             session = \"reviewer\" if review else \"worker-\" + str(abs(hash(prompt.split(\"Task id: \")[-1][:3])))\n\
             if review:\n    answer = verdict(True)\nelse:\n{agent}\n\
             print(json.dumps({{\"event\": \"init\", \"conversation_id\": session}}))\n\
             print(json.dumps({{\"event\": \"result\", \"result\": {{\"conversation_id\": session, \"status\": \"SUCCESS\", \"response\": answer, \"usage\": {{}}}}}}))\n"
        ),
    );
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
    std::fs::create_dir_all(dir.join("project")).unwrap();
    Core::open(&dir.join("state"), Config::from_toml(&text).unwrap()).unwrap()
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done() {
        assert!(Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn prompts(dir: &Path) -> Vec<(String, String)> {
    std::fs::read_to_string(dir.join("prompts.jsonl"))
        .unwrap()
        .lines()
        .map(|line| {
            let call: Value = serde_json::from_str(line).unwrap();
            (
                call["bin"].as_str().unwrap().to_owned(),
                call["prompt"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn events(core: &Core, conversation: &str, kind: &str) -> Vec<Value> {
    core.store
        .events_since(conversation, 0, 10_000)
        .unwrap()
        .into_iter()
        .filter(|event| event.kind == kind)
        .map(|event| event.payload)
        .collect()
}

fn run(core: &Arc<Core>, dir: &Path) -> String {
    let conversation = core.start(&dir.join("project")).unwrap().id;
    core.send(&conversation, "zbadaj").unwrap();
    wait_until("plan proposed", || core.is_idle(&conversation));
    let plan = core.store.latest_plan(&conversation).unwrap()[0]
        .plan_id
        .clone();
    core.accept_plan(&conversation, &plan).unwrap();
    conversation
}

#[test]
fn a_worker_tells_another_task_what_it_found() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        r#"
answer = "done" if "[runtime]" in prompt else plan(task("t1", "Protocol"), task("t2", "Client", ["t1"]), task("t3", "Docs"))
"#,
        r#"    answer = "protocol report" + message("t2", "finding", "the API sends UTF-16") + message("t9", "status", "lost") if "Task id: t1" in prompt else "report""#,
    );
    let conversation = run(&core, dir.path());
    wait_until("plan finished", || core.is_idle(&conversation));

    let tasks = core.store.latest_plan(&conversation).unwrap();
    assert_eq!(tasks[0].report.as_deref(), Some("protocol report"));
    let worker_prompts: Vec<String> = prompts(dir.path())
        .into_iter()
        .filter(|(bin, prompt)| bin == "agy" && prompt.contains("Task id:"))
        .map(|(_, prompt)| prompt)
        .collect();
    let client = worker_prompts
        .iter()
        .find(|prompt| prompt.contains("Task id: t2"))
        .unwrap();
    assert!(
        client.starts_with("Messages for you:\n- from t1 (finding): the API sends UTF-16"),
        "{client}"
    );
    let docs = worker_prompts
        .iter()
        .find(|prompt| prompt.contains("Task id: t3"))
        .unwrap();
    assert!(!docs.contains("UTF-16"), "only the recipient gets it");
    assert!(docs.contains("- t1: Protocol") && docs.contains("```message"));

    let mail = events(&core, &conversation, "mail");
    assert_eq!(mail.len(), 1);
    assert_eq!(
        (&mail[0]["sender"], &mail[0]["recipient"]),
        (&Value::from("t1"), &Value::from("t2"))
    );
    assert_eq!(events(&core, &conversation, "mail_delivered").len(), 1);
    let rejected = events(&core, &conversation, "mail_rejected");
    assert_eq!(rejected[0]["recipient"], "t9");
}

#[test]
fn the_user_writes_to_a_working_agent_and_the_orchestrator_learns_of_it() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        r#"
answer = "done" if "[runtime]" in prompt else plan(task("t1", "Parser"))
"#,
        r#"    if "Messages for you" in prompt:
        answer = "report with UTF-8 case"
    else:
        time.sleep(1)
        answer = "first report""#,
    );
    let conversation = run(&core, dir.path());
    wait_until("worker started", || {
        std::fs::read_to_string(dir.path().join("prompts.jsonl"))
            .is_ok_and(|log| log.contains("Task id: t1"))
    });
    let sent = core
        .submit(&conversation, "@t1 dodaj przypadek UTF-8")
        .unwrap();
    assert_eq!(sent["recipient_finished"], false);
    wait_until("plan finished", || core.is_idle(&conversation));

    let task = &core.store.latest_plan(&conversation).unwrap()[0];
    assert_eq!(task.report.as_deref(), Some("report with UTF-8 case"));
    let all = prompts(dir.path());
    let extra = all
        .iter()
        .find(|(_, prompt)| {
            prompt.contains("dodaj przypadek UTF-8") && prompt.contains("arrived while you worked")
        })
        .expect("the worker got a turn for the message");
    assert!(
        extra
            .1
            .contains("- from user (user_instruction): dodaj przypadek UTF-8")
    );
    let summary = &all.iter().rfind(|(bin, _)| bin == "codex").unwrap().1;
    assert!(
        summary.contains("(the user wrote to t1) dodaj przypadek UTF-8"),
        "{summary}"
    );
    // Not a message to the orchestrator: no turn of its own was started.
    let messages = core.store.messages_after(&conversation, 0).unwrap();
    assert!(
        messages
            .iter()
            .all(|message| !message.text.contains("dodaj przypadek"))
    );
}

#[test]
fn a_comment_reaches_the_orchestrator_and_stays_until_the_user_resolves_it() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        r#"
answer = "noted" if "[runtime]" in prompt or "[user comment" in prompt or "dalej" in prompt else plan(task("t1", "Parser"))
"#,
        r#"    answer = "report""#,
    );
    let conversation = run(&core, dir.path());
    wait_until("plan finished", || core.is_idle(&conversation));

    let comment = core
        .comment(&conversation, "worker:t1", "brakuje przypadku UTF-8")
        .unwrap();
    assert_eq!(comment.status, "open");
    wait_until("comment answered", || core.is_idle(&conversation));
    let orchestrator = |all: Vec<(String, String)>| {
        all.into_iter()
            .filter(|(bin, _)| bin == "codex")
            .map(|(_, prompt)| prompt)
            .collect::<Vec<_>>()
    };
    let prompts_now = orchestrator(prompts(dir.path()));
    assert!(
        prompts_now.last().unwrap().contains(&format!(
            "[user comment #{} on the worker of task t1 \"Parser\"] brakuje przypadku UTF-8",
            comment.id
        )),
        "{prompts_now:?}"
    );

    // Unresolved, it is in the next turn's prompt too.
    core.send(&conversation, "dalej").unwrap();
    wait_until("answered", || core.is_idle(&conversation));
    let next = orchestrator(prompts(dir.path())).pop().unwrap();
    assert!(
        next.contains("not resolved yet") && next.contains("brakuje przypadku UTF-8"),
        "{next}"
    );

    let resolved = core
        .set_comment(&conversation, comment.id, "resolved")
        .unwrap();
    assert_eq!(resolved.status, "resolved");
    core.send(&conversation, "dalej").unwrap();
    wait_until("answered", || core.is_idle(&conversation));
    let last = orchestrator(prompts(dir.path())).pop().unwrap();
    assert!(!last.contains("UTF-8"), "{last}");
    let statuses: Vec<String> = events(&core, &conversation, "comment")
        .iter()
        .map(|comment| comment["status"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(statuses, ["open", "resolved"]);
    assert!(core.set_comment(&conversation, comment.id, "done").is_err());
}

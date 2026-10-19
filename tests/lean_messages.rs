use architect::{config::Config, lean::Core};
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Fake codex (orchestrator) and agy (everyone else). Both log every prompt
/// to `prompts.jsonl`; `answer` comes from the given Python statements.
fn open(dir: &Path, orchestrator: &str, agent: &str) -> Arc<Core> {
    open_limited(dir, orchestrator, agent, 4)
}

/// Like [`open`], with at most `max_parallel` tasks of a plan at a time.
fn open_limited(dir: &Path, orchestrator: &str, agent: &str, max_parallel: usize) -> Arc<Core> {
    open_edited(
        dir,
        orchestrator,
        agent,
        &[(
            "max_parallel = 4",
            &format!("max_parallel = {max_parallel}"),
        )],
    )
}

/// Like [`open`], with the example configuration edited by `replacements`.
fn open_edited(
    dir: &Path,
    orchestrator: &str,
    agent: &str,
    replacements: &[(&str, &str)],
) -> Arc<Core> {
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
    let text = replacements
        .iter()
        .fold(text, |text, (from, to)| text.replace(from, to));
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

#[test]
fn a_turn_stores_its_prompt_reasoning_output_and_whole_answer() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        r#"
print(json.dumps({"type": "item.completed", "item": {"type": "reasoning", "text": "Weigh the options"}}))
print(json.dumps({"type": "item.completed", "item": {"type": "command_execution", "command": "ls", "aggregated_output": "out-A\n" + "x" * 5000, "exit_code": 3}}))
answer = "First sentence.\nSecond line of the answer that is kept whole."
"#,
        "    answer = 'unused'",
    );
    let conversation = core.start(&dir.path().join("project")).unwrap().id;
    core.send(&conversation, "hello there").unwrap();
    wait_until("answered", || core.is_idle(&conversation));
    let agent: Vec<Value> = events(&core, &conversation, "agent");
    let of = |kind: &str| -> Vec<&Value> { agent.iter().filter(|e| e["type"] == kind).collect() };
    assert!(
        of("prompt")[0]["text"]
            .as_str()
            .unwrap()
            .contains("hello there")
    );
    assert_eq!(of("reasoning")[0]["delta"], "Weigh the options");
    let output = of("output")[0];
    assert_eq!(output["exit_code"], 3);
    let text = output["text"].as_str().unwrap();
    assert!(text.starts_with("out-A") && text.ends_with("…[truncated]"));
    assert!(text.len() < 2100);
    // The answer is one event with all its lines, and not repeated.
    let answers = of("text");
    assert_eq!(answers.len(), 1);
    assert!(
        answers[0]["delta"]
            .as_str()
            .unwrap()
            .contains("Second line of the answer that is kept whole.")
    );
}

fn plan_with_two_draft_comments(core: &Arc<Core>, dir: &Path) -> (String, String, Vec<u64>) {
    let conversation = core.start(&dir.join("project")).unwrap().id;
    core.send(&conversation, "plan it").unwrap();
    wait_until("plan proposed", || core.is_idle(&conversation));
    let plan = core.store.latest_plan(&conversation).unwrap()[0]
        .plan_id
        .clone();
    let ids = ["first remark", "second remark"]
        .iter()
        .enumerate()
        .map(|(n, text)| {
            core.comment_scoped(
                &conversation,
                Some(&plan),
                &format!("plan:{plan}:line:{}", n + 1),
                text,
            )
            .unwrap()
            .id
        })
        .collect();
    (conversation, plan, ids)
}

#[test]
fn only_the_comments_the_agent_lists_as_addressed_are_resolved() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        r#"
import re
if "Address each new user comment" in prompt:
    ids = re.findall(r"user comment #(\d+) on", prompt)
    answer = "Revised.\n```comments\n" + json.dumps({"addressed": [{"id": int(ids[0]), "how": "changed"}]}) + "\n```\n" + plan(task("t1", "Parser v2"))
else:
    answer = plan(task("t1", "Parser"))
"#,
        // The follow-up check of a revised plan is a review too.
        "    answer = verdict(True) if \"new plan\" in prompt else 'unused'",
    );
    let (conversation, plan, ids) = plan_with_two_draft_comments(&core, dir.path());
    core.review_comments(&conversation, &plan).unwrap();
    wait_until("revised", || {
        core.is_idle(&conversation)
            && core.store.latest_plan(&conversation).unwrap()[0].plan_id != plan
    });
    let status = |id: u64| {
        ["open", "resolved"]
            .into_iter()
            .find(|s| {
                core.store
                    .comments(&conversation, s)
                    .unwrap()
                    .iter()
                    .any(|c| c.id == id)
            })
            .unwrap()
    };
    assert_eq!(status(ids[0]), "resolved", "the listed comment");
    assert_eq!(
        status(ids[1]),
        "open",
        "the unlisted one waits for the user"
    );
    // The block is not shown to the user.
    let said = core
        .store
        .messages_by(&conversation, "orchestrator", 0)
        .unwrap();
    assert!(said.iter().all(|m| !m.text.contains("```comments")));
}

#[test]
fn comments_answered_in_words_stay_open_and_are_reported_as_replied() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        r#"
if "Address each new user comment" in prompt:
    answer = "Because the plan is split by module."
else:
    answer = plan(task("t1", "Parser"))
"#,
        "    answer = 'unused'",
    );
    let (conversation, plan, ids) = plan_with_two_draft_comments(&core, dir.path());
    core.review_comments(&conversation, &plan).unwrap();
    wait_until("answered", || {
        core.is_idle(&conversation) && !events(&core, &conversation, "comment_replied").is_empty()
    });
    let open: Vec<u64> = core
        .store
        .comments(&conversation, "open")
        .unwrap()
        .iter()
        .map(|c| c.id)
        .collect();
    assert_eq!(open, ids);
    let replied = &events(&core, &conversation, "comment_replied")[0];
    assert_eq!(replied["comments"], serde_json::json!(ids));
    assert_eq!(
        core.store.latest_plan(&conversation).unwrap()[0].plan_id,
        plan
    );
}

/// Two independent read-only tasks whose workers log when they start and end.
fn timed_plan(dir: &Path, max_parallel: usize) -> Vec<(f64, f64)> {
    let log = dir.join("times.log");
    let core = open_limited(
        dir,
        r#"
answer = plan(task("t1", "One"), task("t2", "Two")) if "[runtime]" not in prompt else "done"
"#,
        &format!(
            r#"    import time
    start = time.time()
    time.sleep(0.8)
    with open({log:?}, "a") as f:
        f.write(f"{{start}} {{time.time()}}\n")
    answer = "report""#,
            log = log.to_string_lossy()
        ),
        max_parallel,
    );
    let conversation = core.start(&dir.join("project")).unwrap().id;
    core.send(&conversation, "two things").unwrap();
    wait_until("plan proposed", || core.is_idle(&conversation));
    let plan = core.store.latest_plan(&conversation).unwrap()[0]
        .plan_id
        .clone();
    core.accept_plan(&conversation, &plan).unwrap();
    wait_until("plan done", || {
        core.is_idle(&conversation)
            && core
                .store
                .plan(&plan)
                .unwrap()
                .iter()
                .all(|t| t.status == "approved")
    });
    std::fs::read_to_string(log)
        .unwrap()
        .lines()
        .map(|line| {
            let (a, b) = line.split_once(' ').unwrap();
            (a.parse().unwrap(), b.parse().unwrap())
        })
        .collect()
}

#[test]
fn independent_tasks_overlap_in_time_unless_the_limit_says_otherwise() {
    let dir = tempfile::tempdir().unwrap();
    let mut spans = timed_plan(dir.path(), 4);
    spans.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    assert_eq!(spans.len(), 2);
    assert!(
        spans[1].0 < spans[0].1,
        "the second started before the first ended: {spans:?}"
    );

    let dir = tempfile::tempdir().unwrap();
    let mut spans = timed_plan(dir.path(), 1);
    spans.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    assert!(spans[1].0 >= spans[0].1, "one at a time: {spans:?}");
}

const WRITING_PLAN: &str = r#"
def writer(id, extra=None):
    t = task(id, "Change", [])
    t.update({"writes": True, "kind": "implementation", "scope": ["a.rs"]})
    return t
answer = plan(writer("t1")) if "[runtime]" not in prompt else "done"
"#;

#[test]
fn the_plan_verifier_is_told_when_a_plan_that_changes_code_has_no_context_scout() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        WRITING_PLAN,
        "    answer = verdict(True) if \"You review a plan\" in prompt else 'unused'",
    );
    let conversation = core.start(&dir.path().join("project")).unwrap().id;
    core.send(&conversation, "change a.rs").unwrap();
    wait_until("plan proposed", || core.is_idle(&conversation));
    let review = prompts(dir.path())
        .into_iter()
        .find(|(_, prompt)| prompt.contains("You review a plan"))
        .expect("the plan was reviewed")
        .1;
    assert!(review.contains("no context scout"), "{review}");
    assert!(review.contains("do not approve it until"));
}

#[test]
fn turning_the_required_gates_off_is_said_when_a_plan_changes_code() {
    let dir = tempfile::tempdir().unwrap();
    let core = open_edited(
        dir.path(),
        WRITING_PLAN,
        "    answer = verdict(True) if \"You review a plan\" in prompt else 'unused'",
        &[("required_gates = true", "required_gates = false")],
    );
    let conversation = core.start(&dir.path().join("project")).unwrap().id;
    core.send(&conversation, "change a.rs").unwrap();
    wait_until("plan proposed", || core.is_idle(&conversation));
    let notices = events(&core, &conversation, "notice");
    assert!(
        notices.iter().any(|n| n["text"]
            .as_str()
            .unwrap()
            .contains("required_gates is off")),
        "{notices:?}"
    );
    // With the gates on, there is no such notice.
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        WRITING_PLAN,
        "    answer = verdict(True) if \"You review a plan\" in prompt else 'unused'",
    );
    let conversation = core.start(&dir.path().join("project")).unwrap().id;
    core.send(&conversation, "change a.rs").unwrap();
    wait_until("plan proposed", || core.is_idle(&conversation));
    assert!(
        events(&core, &conversation, "notice")
            .iter()
            .all(|n| !n["text"].as_str().unwrap().contains("required_gates"))
    );
}

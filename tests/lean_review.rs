use architect::{config::Config, lean::Core};
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

const AGENT: &str = r#"
def say(session, answer):
    print(json.dumps({"event": "result", "result": {"conversation_id": session,
        "status": "SUCCESS", "response": answer, "usage": {}}}))
def verdict(approved, findings=()):
    return "```verdict\n" + json.dumps({"approved": approved, "findings": list(findings)}) + "\n```"
def task(id, title, depends=()):
    return {"id": id, "title": title, "goal": "g", "depends_on": list(depends),
            "kind": "analysis", "verifier": {"focus": "f"}}
def plan(*tasks):
    return "Plan.\n```plan\n" + json.dumps({"tasks": list(tasks)}) + "\n```"
"#;

/// Fake codex (orchestrator) and agy (everyone else) whose answers are the
/// given Python statements; each sets `answer` from `prompt`.
fn open(dir: &Path, autonomous: bool, orchestrator: &str, agent: &str) -> Arc<Core> {
    let write = |name: &str, body: String| {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        path
    };
    let header =
        format!("#!/usr/bin/env python3\nimport json, sys\nprompt = sys.argv[-1]\n{AGENT}");
    let codex = write(
        "codex",
        format!(
            "{header}{orchestrator}\nprint(json.dumps({{\"type\": \"thread.started\", \"thread_id\": \"orch\"}}))\n\
             print(json.dumps({{\"type\": \"item.completed\", \"item\": {{\"type\": \"agent_message\", \"text\": answer}}}}))\n\
             print(json.dumps({{\"type\": \"turn.completed\", \"usage\": {{}}}}))\n"
        ),
    );
    let agy = write("agy", format!("{header}{agent}\n"));
    let mut text = std::fs::read_to_string(concat!(
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
    if autonomous {
        text = text.replace("mode = \"supervised\"", "mode = \"autonomous\"");
    }
    std::fs::create_dir_all(dir.join("project")).unwrap();
    Core::open(&dir.join("state"), Config::from_toml(&text).unwrap()).unwrap()
}

fn wait_idle(core: &Core, conversation: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !core.is_idle(conversation) {
        assert!(Instant::now() < deadline, "conversation never went idle");
        std::thread::sleep(Duration::from_millis(20));
    }
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

fn ask(core: &Arc<Core>, dir: &Path, text: &str) -> String {
    let conversation = core.start(&dir.join("project")).unwrap().id;
    core.send(&conversation, text).unwrap();
    wait_idle(core, &conversation);
    conversation
}

#[test]
fn a_plan_the_verifier_sends_back_is_revised_before_the_user_sees_it() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        false,
        r#"
if "[plan verifier]" in prompt:
    answer = plan(task("t1", "Model"), task("t2", "Uses model, fixed", ["t1"]))
else:
    answer = plan(task("t1", "Model"), task("t2", "Uses model"))
"#,
        r#"
if "You review a plan" in prompt or "new plan:" in prompt:
    ok = "fixed" in prompt.split("new plan:")[-1]
    say("plan", verdict(ok, [] if ok else [{"severity": "high", "issue": "t2 needs t1's result", "where": "t2"}]))
"#,
    );
    let conversation = ask(&core, dir.path(), "zbuduj model i użyj go");

    let reviews = events(&core, &conversation, "plan_review");
    assert_eq!(reviews.len(), 1);
    assert_eq!(reviews[0]["approved"], false);
    let proposed = events(&core, &conversation, "plan_proposed");
    assert_eq!(proposed.len(), 1, "only the checked plan is shown");
    assert_eq!(proposed[0]["review"]["approved"], true);
    assert_eq!(proposed[0]["tasks"][1]["title"], "Uses model, fixed");
    assert_eq!(
        proposed[0]["automatic"], false,
        "supervised waits for the user"
    );

    let messages = core.store.messages_after(&conversation, 0).unwrap();
    let sent_back = messages
        .iter()
        .find(|message| message.author == "plan-verifier")
        .unwrap();
    assert!(sent_back.text.contains("t2 needs t1's result"));
    let latest = core.store.latest_plan(&conversation).unwrap();
    assert!(latest.iter().all(|task| task.status == "proposed"));
    assert_ne!(
        core.store
            .plan(reviews[0]["plan"].as_str().unwrap())
            .unwrap()[0]
            .status,
        "proposed",
        "the rejected plan cannot be accepted"
    );
}

#[test]
fn autonomous_mode_runs_a_checked_plan_and_reports_audit_gaps() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        true,
        r#"
if "[runtime]" in prompt:
    answer = "gap noted" if "not fully delivered" in prompt else "all done"
else:
    answer = plan(task("t1", "Analyse"))
"#,
        r#"
if "You review a plan" in prompt:
    say("plan", verdict(True))
elif "You audit" in prompt:
    say("audit", verdict(False, [{"severity": "high", "issue": "README not covered", "where": "README"}]))
elif "You verify" in prompt:
    say("verifier", verdict(True))
else:
    say("worker", "analysis report")
"#,
    );
    let conversation = ask(&core, dir.path(), "przeanalizuj i opisz w README");

    let tasks = core.store.latest_plan(&conversation).unwrap();
    assert_eq!(tasks[0].status, "approved", "ran without /accept");
    let audits = events(&core, &conversation, "audit");
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0]["approved"], false);
    let messages = core.store.messages_after(&conversation, 0).unwrap();
    let report = messages
        .iter()
        .find(|message| message.author == "runtime")
        .unwrap();
    assert!(
        report.text.contains("README not covered"),
        "{}",
        report.text
    );
    assert_eq!(messages.last().unwrap().text, "gap noted");
}

#[test]
fn autonomous_plans_stop_for_the_user_after_a_few_rounds() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        true,
        r#"answer = plan(task("t1", "Again"))"#,
        r#"
if "You verify" in prompt or "You review a plan" in prompt or "new plan:" in prompt \
        or "You audit" in prompt or "next plan:" in prompt:
    say("reviewer", verdict(True))
else:
    say("worker", "report")
"#,
    );
    let conversation = ask(&core, dir.path(), "rób dalej");

    let proposed = events(&core, &conversation, "plan_proposed");
    let automatic: Vec<_> = proposed
        .iter()
        .map(|plan| plan["automatic"] == true)
        .collect();
    assert_eq!(automatic, [true, true, true, false]);
    assert!(
        proposed[3]["waiting"]
            .as_str()
            .unwrap()
            .contains("without you")
    );
}

#[test]
fn a_plan_whose_review_failed_is_shown_but_never_runs_by_itself() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        true,
        r#"answer = plan(task("t1", "Analyse"))"#,
        r#"
print("login required", file=sys.stderr)
sys.exit(1)
"#,
    );
    let conversation = ask(&core, dir.path(), "analiza");

    let proposed = events(&core, &conversation, "plan_proposed");
    assert_eq!(proposed.len(), 1);
    assert!(
        proposed[0]["review"]["error"]
            .as_str()
            .unwrap()
            .contains("login required")
    );
    assert_eq!(proposed[0]["automatic"], false);
    let tasks = core.store.latest_plan(&conversation).unwrap();
    assert_eq!(tasks[0].status, "proposed", "the user can still accept it");
}

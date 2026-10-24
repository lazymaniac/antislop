mod common;
mod support;
use antislop::{config::Config, lean::Core};
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

const AGENT: &str = r#"
def bump(name):
    import os
    path = os.path.join(os.path.dirname(sys.argv[0]), name)
    n = int(open(path).read()) + 1 if os.path.exists(path) else 1
    open(path, "w").write(str(n))
    return n
import re
def cover(answer):
    ids = re.findall(r"^- ([BRCV][0-9]+): ", prompt, re.M)
    if not ids or "```verdict" not in answer:
        return answer
    head, body = answer.rsplit("```verdict", 1)
    data = json.loads(body.split("```")[0])
    data["coverage"] = {i: {"status": "ok", "note": "checked"} for i in ids}
    return head + "```verdict\n" + json.dumps(data) + "\n```"
def say(session, answer, covered=True):
    if covered:
        answer = cover(answer)
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
        support::install_ledger_protocol(&path);
        std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
    common::interactive(&path);
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
    let conversation = support::legacy_conversation(core, &dir.join("project"))
        .unwrap()
        .id;
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
    let conversation = ask(&core, dir.path(), "build a model and use it");

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
fn a_plan_no_agent_can_carry_out_goes_back_before_review() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        false,
        r#"
broken = task("t1", "Change it")
broken["kind"] = "implementation"
if "checked the proposed plan" in prompt:
    answer = plan(task("t1", "Change it, readable"))
else:
    answer = plan(broken)
"#,
        r#"
if "You review a plan" in prompt or "new plan:" in prompt:
    say("plan", verdict(True))
"#,
    );
    let conversation = ask(&core, dir.path(), "change the code");

    let messages = core.store.messages_after(&conversation, 0).unwrap();
    let sent_back = messages
        .iter()
        .find(|message| message.author == "plan-lint")
        .expect("the orchestrator is told before any review");
    assert!(
        sent_back
            .text
            .contains("- t1 [implementation-is-read-only]")
    );
    let reviews = events(&core, &conversation, "plan_review");
    assert!(reviews.is_empty(), "the verifier never saw the broken plan");
    let proposed = events(&core, &conversation, "plan_proposed");
    assert_eq!(proposed.len(), 1);
    assert_eq!(proposed[0]["tasks"][0]["title"], "Change it, readable");
}

#[test]
fn a_blocker_is_cleared_by_a_task_inserted_before_the_blocked_one() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        true,
        r#"
fix = {"id": "inv", "title": "Inventory", "goal": "Produce the inventory", "kind": "analysis",
       "verifier": {"focus": "f"}}
if "blocked (MissingInput)" in prompt:
    answer = "Adding a task.\n```recover\n" + json.dumps({"actions": [{"task": "t1", "action": "add_task", "spec": fix}]}) + "\n```"
elif "[runtime]" in prompt:
    answer = "done"
else:
    answer = plan(task("t1", "Work"))
"#,
        r#"
if "You review a plan" in prompt or "You audit" in prompt:
    say("reviewer", verdict(True))
elif "You verify" in prompt or "updated report" in prompt:
    say("verifier", verdict(True))
elif "Task id: t1" in prompt and bump("t1") == 1:
    say("worker", "Cannot go on.\n```blocker\n" + json.dumps({"kind": "missing_input", "needs": "an inventory", "why": "none exists"}) + "\n```")
else:
    say("worker", "report")
"#,
    );
    let conversation = ask(&core, dir.path(), "work");

    let plan = core.store.latest_plan(&conversation).unwrap();
    let ids: Vec<_> = plan
        .iter()
        .map(|t| t.spec["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["inv", "t1"], "inserted before the blocked task");
    assert!(plan.iter().all(|t| t.status == "approved"), "{plan:#?}");
    assert_eq!(plan[0].spec["resolves"], "t1");
    assert_eq!(plan[1].spec["depends_on"][0], "inv");
    assert_eq!(plan[1].round, 2, "the blocked task resumed in a new round");
    let recoveries = events(&core, &conversation, "task_recovery");
    assert_eq!(recoveries[0]["action"], "add_task");
    let messages = core.store.messages_after(&conversation, 0).unwrap();
    assert!(
        messages
            .iter()
            .any(|m| m.text.contains("blocked (MissingInput)"))
    );
}

#[test]
fn a_task_that_is_too_large_is_cut_into_parts_that_come_first() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        true,
        r#"
a = {"id": "a", "title": "Part A", "goal": "g", "kind": "analysis", "verifier": {"focus": "f"}}
b = {"id": "b", "title": "Part B", "goal": "g", "kind": "analysis", "depends_on": ["a"], "verifier": {"focus": "f"}}
if "blocked (TooLarge)" in prompt:
    answer = "Cutting it.\n```recover\n" + json.dumps({"actions": [{"task": "t1", "action": "split_task", "parts": [a, b]}]}) + "\n```"
elif "[runtime]" in prompt:
    answer = "done"
else:
    answer = plan(task("t1", "Work"))
"#,
        r#"
if "You review a plan" in prompt or "You audit" in prompt:
    say("reviewer", verdict(True))
elif "You verify" in prompt or "updated report" in prompt:
    say("verifier", verdict(True))
elif "Task id: t1" in prompt and bump("t1") == 1:
    say("worker", "Too much.\n```blocker\n" + json.dumps({"kind": "too_large", "needs": "a smaller task", "why": "two areas", "suggest": "one part each"}) + "\n```")
else:
    say("worker", "report")
"#,
    );
    let conversation = ask(&core, dir.path(), "work");

    let plan = core.store.latest_plan(&conversation).unwrap();
    let ids: Vec<_> = plan
        .iter()
        .map(|t| t.spec["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["a", "b", "t1"], "the parts come before the whole");
    assert!(plan.iter().all(|t| t.status == "approved"), "{plan:#?}");
    assert_eq!(plan[0].spec["part_of"], "t1");
    assert_eq!(plan[1].spec["depends_on"][0], "a");
    assert_eq!(plan[2].spec["depends_on"], serde_json::json!(["a", "b"]));
    assert_eq!(plan[2].round, 2, "the whole resumed in a new round");
    let recoveries = events(&core, &conversation, "task_recovery");
    assert_eq!(recoveries[0]["action"], "split_task");
}

#[test]
fn a_split_that_breaks_the_rules_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        true,
        r#"
a = {"id": "a", "title": "Part A", "goal": "g", "kind": "analysis", "verifier": {"focus": "f"}}
b = {"id": "b", "title": "Part B", "goal": "g", "kind": "analysis", "depends_on": ["a"], "verifier": {"focus": "f"}}
if "blocked (TooLarge)" in prompt:
    answer = "Cutting it.\n```recover\n" + json.dumps({"actions": [{"task": "t1", "action": "split_task", "parts": [a]}]}) + "\n```"
elif "[runtime]" in prompt:
    answer = "done"
else:
    answer = plan(task("t1", "Work"))
"#,
        r#"
if "You review a plan" in prompt or "You audit" in prompt:
    say("reviewer", verdict(True))
elif "You verify" in prompt or "updated report" in prompt:
    say("verifier", verdict(True))
elif "Task id: t1" in prompt and bump("t1") == 1:
    say("worker", "Too much.\n```blocker\n" + json.dumps({"kind": "too_large", "needs": "a smaller task", "why": "two areas", "suggest": "one part each"}) + "\n```")
else:
    say("worker", "report")
"#,
    );
    let conversation = ask(&core, dir.path(), "work");

    let plan = core.store.latest_plan(&conversation).unwrap();
    assert_eq!(plan.len(), 1, "nothing was inserted");
    assert_eq!(plan[0].status, "needs_user");
    let notices = events(&core, &conversation, "notice");
    assert!(notices.iter().any(|n| {
        n["text"]
            .as_str()
            .is_some_and(|text| text.contains("2 to 8 parts"))
    }));
}

#[test]
fn a_refused_permission_is_left_to_the_user_not_to_another_agent() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        true,
        r#"
fix = {"id": "inv", "title": "Inventory", "goal": "g", "kind": "analysis", "verifier": {"focus": "f"}}
if "blocked (Permission)" in prompt:
    answer = "Trying.\n```recover\n" + json.dumps({"actions": [{"task": "t1", "action": "add_task", "spec": fix}]}) + "\n```"
elif "[runtime]" in prompt:
    answer = "done"
else:
    answer = plan(task("t1", "Work"))
"#,
        r#"
if "You review a plan" in prompt or "You audit" in prompt:
    say("reviewer", verdict(True))
elif "Task id: t1" in prompt:
    say("worker", "Denied.\n```blocker\n" + json.dumps({"kind": "permission", "needs": "write access", "why": "sandbox"}) + "\n```")
else:
    say("worker", "report")
"#,
    );
    let conversation = ask(&core, dir.path(), "work");

    let plan = core.store.latest_plan(&conversation).unwrap();
    assert_eq!(plan.len(), 1, "no task was inserted");
    assert_eq!(plan[0].status, "needs_user");
    let notices = events(&core, &conversation, "notice");
    assert!(notices.iter().any(|n| {
        n["text"]
            .as_str()
            .is_some_and(|text| text.contains("left to the user"))
    }));
    let messages = core.store.messages_after(&conversation, 0).unwrap();
    assert!(messages.iter().any(|m| {
        m.text
            .contains("Only the user can lift the permission blocker of t1")
    }));
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
    let conversation = ask(&core, dir.path(), "analyze and describe in README");

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
fn autonomous_plans_go_on_until_one_repeats_a_plan_that_ran() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        true,
        r#"answer = plan(task("t1", "Step " + str(min(bump("plans"), 4))))"#,
        r#"
if "You verify" in prompt or "You review a plan" in prompt or "new plan:" in prompt \
        or "You audit" in prompt or "next plan:" in prompt:
    say("reviewer", verdict(True))
else:
    say("worker", "report")
"#,
    );
    let conversation = ask(&core, dir.path(), "keep going");

    let proposed = events(&core, &conversation, "plan_proposed");
    let automatic: Vec<_> = proposed
        .iter()
        .map(|plan| plan["automatic"] == true)
        .collect();
    // Steps 1–4 run on their own; the second "Step 4" waits for the user.
    assert_eq!(automatic, [true, true, true, true, false]);
    assert!(
        proposed[4]["waiting"]
            .as_str()
            .unwrap()
            .contains("same plan already ran")
    );
}

#[test]
fn autonomous_rounds_go_on_while_the_worker_gets_closer() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        true,
        r#"answer = "done" if "[runtime]" in prompt else plan(task("t1", "Fix"))"#,
        r#"
if "You review a plan" in prompt or "You audit" in prompt:
    say("reviewer", verdict(True))
elif "found problems" in prompt:
    say("worker", "report")
elif "You verify" in prompt or "updated report" in prompt:
    n = bump("verdicts")
    say("verifier", verdict(n >= 5, [] if n >= 5 else [{"severity": "high", "issue": "case %d" % n, "where": "a"}]))
else:
    say("worker", "report")
"#,
    );
    let conversation = ask(&core, dir.path(), "fix");

    let task = &core.store.latest_plan(&conversation).unwrap()[0];
    assert_eq!(task.status, "approved", "{:?}", task.report);
    assert_eq!(task.round, 5, "more rounds than supervised mode runs alone");
}

#[test]
fn a_verifier_repeating_its_findings_stops_the_rounds() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        true,
        r#"answer = "done" if "[runtime]" in prompt else plan(task("t1", "Fix"))"#,
        r#"
if "You review a plan" in prompt or "You audit" in prompt:
    say("reviewer", verdict(True))
elif "found problems" in prompt:
    say("worker", "report")
elif "You verify" in prompt or "updated report" in prompt:
    say("verifier", verdict(False, [{"severity": "high", "issue": "still wrong", "where": "a"}]))
else:
    say("worker", "report")
"#,
    );
    let conversation = ask(&core, dir.path(), "fix");

    let task = &core.store.latest_plan(&conversation).unwrap()[0];
    assert_eq!(task.status, "needs_user");
    assert_eq!(task.round, 2);
    assert!(
        task.report
            .as_deref()
            .unwrap()
            .contains("repeated the same findings")
    );
    // Frontends are told why it stopped.
    let last = events(&core, &conversation, "task").pop().unwrap();
    assert_eq!(last["status"], "needs_user");
    assert!(
        last["reason"]
            .as_str()
            .unwrap()
            .contains("not getting closer")
    );
}

#[test]
fn the_same_obligations_open_round_after_round_stop_the_rounds_however_they_are_worded() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        true,
        r#"answer = "done" if "[runtime]" in prompt else plan(task("t1", "Fix"))"#,
        r#"
if "You review a plan" in prompt or "You audit" in prompt:
    say("reviewer", verdict(True))
elif "found problems" in prompt:
    say("worker", "report")
elif "You verify" in prompt or "updated report" in prompt:
    n = bump("verdicts")
    ids = re.findall(r"^- ([BRCV][0-9]+): ", prompt, re.M)
    cov = {i: {"status": "ok", "note": "checked"} for i in ids}
    cov["V1"] = {"status": "issue", "note": "open %d" % n}
    findings = [{"severity": "high", "issue": "case %d" % n, "where": "a"}]
    say("verifier", "```verdict\n" + json.dumps({"approved": False, "findings": findings, "coverage": cov}) + "\n```", False)
else:
    say("worker", "report")
"#,
    );
    let conversation = ask(&core, dir.path(), "fix");

    let task = &core.store.latest_plan(&conversation).unwrap()[0];
    assert_eq!(task.status, "needs_user");
    assert_eq!(task.round, 4, "three rounds in a row without a new best");
    let report = task.report.as_deref().unwrap();
    assert!(
        report.contains("left the same obligations open"),
        "{report}"
    );
}

#[test]
fn an_autonomous_task_stops_at_the_ceiling_even_when_the_problems_keep_changing() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        true,
        r#"answer = "done" if "[runtime]" in prompt else plan(task("t1", "Fix"))"#,
        r#"
if "You review a plan" in prompt or "You audit" in prompt:
    say("reviewer", verdict(True))
elif "found problems" in prompt:
    say("worker", "report")
elif "You verify" in prompt or "updated report" in prompt:
    n = bump("verdicts")
    ids = re.findall(r"^- ([BRCV][0-9]+): ", prompt, re.M)
    cov = {i: {"status": "ok", "note": "checked"} for i in ids}
    cov["V1" if n % 2 else "V2"] = {"status": "issue", "note": "open %d" % n}
    findings = [{"severity": "high", "issue": "case %d" % n, "where": "a"}]
    say("verifier", "```verdict\n" + json.dumps({"approved": False, "findings": findings, "coverage": cov}) + "\n```", False)
else:
    say("worker", "report")
"#,
    );
    let conversation = ask(&core, dir.path(), "fix");

    let task = &core.store.latest_plan(&conversation).unwrap()[0];
    assert_eq!(task.status, "needs_user");
    assert_eq!(task.round, 12, "AUTONOMOUS_ROUNDS");
    let last = events(&core, &conversation, "task").pop().unwrap();
    assert!(last["reason"].as_str().unwrap().contains("are over"));
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
    let conversation = ask(&core, dir.path(), "analysis");

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

const RISKY_PLAN: &str = r#"
if "[runtime]" in prompt:
    answer = "done"
else:
    answer = plan(dict(task("t1", "Revenue"), verifier={"focus": "numbers", "risks": ["units of money"]}))
"#;

#[test]
fn a_verdict_that_skips_an_obligation_is_asked_again_and_never_approves() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        true,
        RISKY_PLAN,
        r#"
if "You review a plan" in prompt or "You audit" in prompt:
    say("reviewer", verdict(True))
elif "You verify" in prompt or "does not account" in prompt:
    say("verifier", verdict(True), covered=False)
else:
    say("worker", "report")
"#,
    );
    let conversation = ask(&core, dir.path(), "calculate revenue");

    let task = &core.store.latest_plan(&conversation).unwrap()[0];
    assert_eq!(task.status, "needs_user");
    let verdict = task.verdict.clone().unwrap();
    assert_eq!(verdict["approved"], false);
    assert!(
        verdict["findings"][0]["issue"]
            .as_str()
            .unwrap()
            .contains("R1 (units of money)"),
        "{verdict}"
    );
    let verifier_turns = events(&core, &conversation, "turn_started")
        .iter()
        .filter(|turn| turn["role"] == format!("verifier:{}", task.id))
        .count();
    assert_eq!(verifier_turns, 2, "asked once more, then the user");
}

#[test]
fn an_obligation_marked_issue_sends_its_note_to_the_worker() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        true,
        RISKY_PLAN,
        r#"
import re
ids = re.findall(r"^- ([RCV][0-9]+): ", prompt, re.M)
if "You review a plan" in prompt or "You audit" in prompt:
    say("reviewer", verdict(True))
elif "You verify" in prompt and "fixed: converted" not in prompt:
    # The units check, as an issue: the rest is asked for again.
    lines = re.findall(r"^- ([RCV][0-9]+): (.*)$", prompt, re.M)
    uid = next(i for i, text in lines if "units and denominators" in text)
    coverage = {"R1": {"status": "ok", "note": "the report flags the mixed currencies"},
                uid: {"status": "issue", "note": "EUR added to PLN"}}
    say("verifier", "```verdict\n" + json.dumps({"approved": True, "findings": [], "coverage": coverage}) + "\n```", covered=False)
elif "does not account" in prompt:
    assert "R1" not in ids, ids
    say("verifier", verdict(True))
elif "found problems" in prompt:
    say("worker", "fixed: " + ("EUR added to PLN" in prompt and "converted" or "?"))
elif "updated report" in prompt or ("You verify" in prompt and "fixed: converted" in prompt):
    say("verifier", verdict(True))
else:
    say("worker", "report")
"#,
    );
    let conversation = ask(&core, dir.path(), "calculate revenue");

    let task = &core.store.latest_plan(&conversation).unwrap()[0];
    assert_eq!(task.status, "approved", "{:?}", task.verdict);
    assert_eq!(task.report.as_deref(), Some("fixed: converted"));
}

#[test]
fn a_paused_conversation_starts_no_new_work_but_still_talks() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        false,
        r#"
if "[runtime]" in prompt or "user question marker" in prompt:
    answer = "answer"
else:
    answer = plan(task("t1", "Analyse"))
"#,
        r#"
if "You verify" in prompt or "You review a plan" in prompt or "You audit" in prompt:
    say("reviewer", verdict(True))
else:
    say("worker", "report")
"#,
    );
    let conversation = ask(&core, dir.path(), "analysis");
    let plan = core.store.latest_plan(&conversation).unwrap()[0]
        .plan_id
        .clone();

    core.pause(&conversation, true).unwrap();
    core.accept_plan(&conversation, &plan).unwrap();
    core.send(&conversation, "user question marker").unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while core
        .store
        .messages_after(&conversation, 0)
        .unwrap()
        .last()
        .is_none_or(|message| message.text != "answer")
    {
        assert!(
            Instant::now() < deadline,
            "the orchestrator answers while paused"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(300));
    let worker_turns = || {
        events(&core, &conversation, "turn_started")
            .iter()
            .filter(|turn| turn["role"].as_str().unwrap().starts_with("worker:"))
            .count()
    };
    assert_eq!(worker_turns(), 0, "no new work while paused");
    assert!(
        !core.is_idle(&conversation),
        "the plan waits, it is not over"
    );
    assert!(
        core.store.paused(&conversation).unwrap(),
        "kept in the store"
    );

    core.pause(&conversation, false).unwrap();
    wait_idle(&core, &conversation);
    assert_eq!(worker_turns(), 1);
    let task = &core.store.latest_plan(&conversation).unwrap()[0];
    assert_eq!(task.status, "approved");
    let paused: Vec<bool> = events(&core, &conversation, "paused")
        .iter()
        .map(|event| event["paused"] == true)
        .collect();
    assert_eq!(paused, [true, false]);
}

#[test]
fn a_mode_change_applies_to_what_the_conversation_does_next() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        false,
        r#"
if "[runtime]" in prompt:
    answer = "done"
else:
    answer = plan(task("t1", "Step " + str(bump("plans"))))
"#,
        r#"
if "You verify" in prompt or "You review a plan" in prompt or "new plan:" in prompt \
        or "You audit" in prompt or "next plan:" in prompt:
    say("reviewer", verdict(True))
else:
    say("worker", "report")
"#,
    );
    let conversation = ask(&core, dir.path(), "step");
    assert!(!core.autonomous(&conversation).unwrap());

    core.set_mode(&conversation, true).unwrap();
    core.send(&conversation, "next step").unwrap();
    wait_idle(&core, &conversation);
    core.set_mode(&conversation, false).unwrap();
    core.send(&conversation, "and one more").unwrap();
    wait_idle(&core, &conversation);

    let automatic: Vec<_> = events(&core, &conversation, "plan_proposed")
        .iter()
        .map(|plan| plan["automatic"] == true)
        .collect();
    assert_eq!(automatic, [false, true, false]);
    let plans = events(&core, &conversation, "plan_proposed");
    let first = plans[0]["plan"].as_str().unwrap();
    assert!(
        core.store
            .plan(first)
            .unwrap()
            .iter()
            .all(|task| task.status == "proposed"),
        "switching to autonomous starts nothing the user did not accept"
    );
    let modes: Vec<_> = events(&core, &conversation, "mode")
        .iter()
        .map(|mode| mode["autonomous"] == true)
        .collect();
    assert_eq!(modes, [true, false]);
}

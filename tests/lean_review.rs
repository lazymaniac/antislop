mod common;
mod support;
use antislop::{config::Config, lean::Core};
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

const AGENT: &str = r#"
def bump(name, step=1):
    import os
    path = os.path.join(os.path.dirname(sys.argv[0]), name)
    n = (int(open(path).read()) if os.path.exists(path) else 0) + step
    open(path, "w").write(str(n))
    return n
import re
def cover(answer):
    ids = re.findall(r"^- ([A-Za-z0-9_.-]+): ", prompt, re.M)
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
    // Every round starts fresh worker and verifier terminals, and a task runs
    // up to a dozen rounds.
    let deadline = Instant::now() + Duration::from_secs(600);
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
    let conversation = support::fixture_conversation(core, &dir.join("project"))
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
        // A plan sent back for ledger mapping is regenerated, not a further step.
        r#"answer = plan(task("t1", "Step " + str(min(bump("plans", 0 if "Regenerate the task plan" in prompt else 1), 4))))"#,
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
    say("verifier", verdict(n >= 5, [] if n >= 5 else [{"severity": "high", "id":"case-%d" % n,"issue": "case %d" % n, "where": "a"}]))
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
    assert_eq!(task.status, "coordinating");
    assert_eq!(task.round, 2);
    assert!(
        task.report
            .as_deref()
            .unwrap()
            .contains("no previously open obligation")
    );
    // Frontends are told why it stopped.
    let last = events(&core, &conversation, "task").pop().unwrap();
    assert_eq!(last["status"], "coordinating");
    assert!(
        last["reason"]
            .as_str()
            .unwrap()
            .contains("no previously open obligation")
    );
}

#[test]
fn a_verifier_inventing_new_finding_ids_each_round_still_hands_the_task_back() {
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
    say("verifier", verdict(False, [{"severity": "high", "id": "new-%d" % n, "issue": "problem %d" % n, "where": "a"}]))
else:
    say("worker", "report")
"#,
    );
    let conversation = ask(&core, dir.path(), "fix");

    let task = &core.store.latest_plan(&conversation).unwrap()[0];
    assert_eq!(task.status, "coordinating", "{:?}", task.report);
    assert_eq!(task.round, 16, "one run takes at most sixteen rounds");
    let last = events(&core, &conversation, "task").pop().unwrap();
    assert!(
        last["reason"]
            .as_str()
            .unwrap()
            .contains("correction rounds ran without approval")
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
    ids = re.findall(r"^- ([A-Za-z0-9_.-]+): ", prompt, re.M)
    cov = {i: {"status": "ok", "note": "checked"} for i in ids}
    cov["V1"] = {"status": "issue", "note": "open %d" % n}
    findings = [{"severity": "high", "id":"same-case","issue": "case %d" % n, "where": "a"}]
    say("verifier", "```verdict\n" + json.dumps({"approved": False, "findings": findings, "coverage": cov}) + "\n```", False)
else:
    say("worker", "report")
"#,
    );
    let conversation = ask(&core, dir.path(), "fix");

    let task = &core.store.latest_plan(&conversation).unwrap()[0];
    assert_eq!(task.status, "coordinating");
    assert_eq!(
        task.round, 2,
        "unchanged local obligations require coordination"
    );
    let report = task.report.as_deref().unwrap();
    assert!(report.contains("no previously open obligation"), "{report}");
}

#[test]
fn independently_verified_progress_continues_past_many_rounds() {
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
    ids = re.findall(r"^- ([A-Za-z0-9_.-]+): ", prompt, re.M)
    cov = {i: {"status": "ok", "note": "checked"} for i in ids}
    if n < 14: cov["V1" if n % 2 else "V2"] = {"status": "issue", "note": "open %d" % n}
    findings = [{"severity": "high", "id":"case-%d" % n,"issue": "case %d" % n, "where": "a"}]
    say("verifier", "```verdict\n" + json.dumps({"approved": n >= 14, "findings": [] if n >= 14 else findings, "coverage": cov}) + "\n```", False)
else:
    say("worker", "report")
"#,
    );
    let conversation = ask(&core, dir.path(), "fix");

    let task = &core.store.latest_plan(&conversation).unwrap()[0];
    assert_eq!(task.status, "approved");
    assert_eq!(task.round, 14);
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
    say("verifier", json.dumps({"type":"review","report":"Skipped obligations","verdict":{"approved":True,"findings":[],"coverage":{}}}), covered=False)
else:
    say("worker", "report")
"#,
    );
    let conversation = ask(&core, dir.path(), "calculate revenue");

    let task = &core.store.latest_plan(&conversation).unwrap()[0];
    assert_eq!(task.status, "failed");
    assert!(task.report.as_deref().unwrap().contains("verdict.coverage"));
    assert!(task.verdict.is_none());
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
ids = re.findall(r"^- ([A-Za-z0-9_.-]+): ", prompt, re.M)
if "You review a plan" in prompt or "You audit" in prompt:
    say("reviewer", verdict(True))
elif "You verify" in prompt and "fixed: converted" not in prompt:
    # The units check, as an issue: the rest is asked for again.
    lines = re.findall(r"^- ([A-Za-z0-9_.-]+): (.*)$", prompt, re.M)
    uid = next(i for i, text in lines if "units and denominators" in text)
    coverage = {i:{"status":"ok","note":"checked"} for i in ids}
    coverage[uid] = {"status":"issue","note":"EUR added to PLN"}
    say("verifier", "```verdict\n" + json.dumps({"approved": False, "findings": [], "coverage": coverage}) + "\n```", covered=False)
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
    let deadline = Instant::now() + Duration::from_secs(120);
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
    // The finished autonomous plan ends the cycle: the next request starts a
    // new interview, which the supervised mode leaves for the user to approve.
    assert_eq!(automatic, [false, true]);
    assert!(core.store.workflow(&conversation).unwrap().interviewing());
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

#[test]
fn runtime_refuses_a_cyclic_persisted_graph_before_starting_workers() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        false,
        r#"answer = plan(task("t1", "Scout"), task("t2", "Consumer", ["t1"]))"#,
        r#"
if "You review a plan" in prompt:
    say("reviewer", verdict(True))
else:
    raise RuntimeError("no worker may start for an invalid graph")
"#,
    );
    let conversation = ask(&core, dir.path(), "inventory and verify");
    let mut tasks = core.store.latest_plan(&conversation).unwrap();
    tasks[0].spec["depends_on"] = serde_json::json!(["t2"]);
    let db = rusqlite::Connection::open(dir.path().join("state/core.db")).unwrap();
    db.execute(
        "UPDATE tasks SET spec=?1 WHERE id=?2",
        rusqlite::params![tasks[0].spec.to_string(), tasks[0].id],
    )
    .unwrap();
    let error = core
        .accept_plan(&conversation, &tasks[0].plan_id)
        .unwrap_err();
    assert!(error.to_string().contains("not an earlier task"), "{error}");
    assert!(
        core.store
            .latest_plan(&conversation)
            .unwrap()
            .iter()
            .all(|task| task.status == "proposed" && task.round == 0)
    );
}

#[test]
fn plan_corrections_continue_beyond_three_rejections_when_findings_close() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        false,
        r#"answer=plan(task('t1','Focused plan'))"#,
        r#"
if 'You review a plan' in prompt or 'new plan:' in prompt:
    n=bump('plan-review-count')
    findings=[] if n>=6 else [{'id':'plan-%d'%i,'severity':'high','issue':'Missing dependency %d'%i,'where':'t1'} for i in range(n,6)]
    say('plan',verdict(n>=6,findings))
"#,
    );
    let conversation = ask(&core, dir.path(), "Plan the local work");
    let proposed = events(&core, &conversation, "plan_proposed");
    assert_eq!(proposed.len(), 1);
    assert_eq!(proposed[0]["review"]["approved"], true);
    assert_eq!(events(&core, &conversation, "plan_review").len(), 5);
}

#[test]
fn a_plan_review_that_only_says_approved_is_asked_for_its_verdict_block() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        false,
        r#"answer=plan(task('t1','Focused plan'))"#,
        r#"
if 'required verdict block' in prompt:
    bump('repairs')
    say('plan', verdict(True))
elif 'You review a plan' in prompt or 'new plan:' in prompt:
    say('plan', 'Plan review: approved. The plan is consistent.', False)
"#,
    );
    let conversation = ask(&core, dir.path(), "Plan the local work");
    let proposed = events(&core, &conversation, "plan_proposed");
    assert_eq!(proposed.len(), 1);
    assert_eq!(proposed[0]["review"]["approved"], true);
    assert!(events(&core, &conversation, "plan_review").is_empty());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("repairs")).unwrap(),
        "1"
    );
}

#[test]
fn rewording_the_same_plan_finding_stops_automatic_replanning() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        false,
        r#"answer=plan(task('t1','Focused plan'))"#,
        r#"
if 'You review a plan' in prompt or 'new plan:' in prompt:
    n=bump('plan-review-count')
    say('plan',verdict(False,[{'id':'missing-input','severity':'high','issue':'Missing input, wording %d'%n,'where':'t1'}]))
"#,
    );
    let conversation = ask(&core, dir.path(), "Plan the local work");
    let proposed = events(&core, &conversation, "plan_proposed");
    assert_eq!(proposed.len(), 1);
    assert_eq!(proposed[0]["review"]["approved"], false);
    let starts = events(&core, &conversation, "turn_started");
    assert_eq!(
        starts
            .iter()
            .filter(|e| e["role"] == "plan-verifier")
            .count(),
        2
    );
    assert!(events(&core, &conversation, "notice").iter().any(|e| {
        e["text"]
            .as_str()
            .is_some_and(|s| s.contains("closure of a previous finding"))
    }));
}

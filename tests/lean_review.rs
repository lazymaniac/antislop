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
            "change_kind": "none", "kind": "analysis", "reviewer": {"summary": "f"}}
def plan(*tasks):
    return "Plan.\n```plan\n" + json.dumps({"tasks": list(tasks)}) + "\n```"
"#;

/// Fake codex (orchestrator) and agy (everyone else) whose answers are the
/// given Python statements; each sets `answer` from `prompt`.
fn open(dir: &Path, orchestrator: &str, agent: &str) -> Arc<Core> {
    let write = |name: &str, body: String| {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        support::install_ledger_protocol(&path);
        std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        common::interactive(&path);
        path
    };
    let header = format!(
        "#!/usr/bin/env python3\nimport json, sys, os\nprompt = sys.argv[-1]\nwith open(os.path.join(os.path.dirname(__file__), \"fixture-prompts.jsonl\"), \"a\") as trace: trace.write(json.dumps({{\"cli\":os.path.basename(__file__),\"prompt\":prompt}})+\"\\n\")\n{AGENT}"
    );
    let codex = write(
        "codex",
        format!(
            "{header}{orchestrator}\nprint(json.dumps({{\"type\": \"thread.started\", \"thread_id\": \"orch\"}}))\n\
             print(json.dumps({{\"type\": \"item.completed\", \"item\": {{\"type\": \"agent_message\", \"text\": answer}}}}))\n\
             print(json.dumps({{\"type\": \"turn.completed\", \"usage\": {{}}}}))\n"
        ),
    );
    let plan_gate = r#"
if 'You review a plan' in prompt and os.path.exists(os.path.join(os.path.dirname(__file__),'plan-only-hold')):
    import time
    root=os.path.dirname(__file__)
    open(os.path.join(root,'plan-only-entered'),'w').close()
    while not os.path.exists(os.path.join(root,'plan-only-release')):time.sleep(0.01)
"#;
    let agy = write("agy", format!("{header}{plan_gate}{agent}\n"));
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
    std::fs::create_dir_all(dir.join("project")).unwrap();
    let mut configuration = Config::from_toml(&text).unwrap();
    configuration
        .routing
        .get_mut(&antislop::config::WorkKind::Planning)
        .unwrap()
        .executor = "orchestrator".into();
    Core::open(&dir.join("state"), configuration).unwrap()
}

fn wait_idle(core: &Core, conversation: &str) {
    // Every round starts fresh worker and reviewer terminals, and a task runs
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

/// Inspect planning with execution explicitly paused before review completes.
fn ask_plan(core: &Arc<Core>, dir: &Path, text: &str) -> String {
    std::fs::write(dir.join("plan-only-hold"), "").unwrap();
    let conversation = support::fixture_conversation(core, &dir.join("project"))
        .unwrap()
        .id;
    core.send(&conversation, text).unwrap();
    let deadline = Instant::now() + Duration::from_secs(120);
    while !dir.join("plan-only-entered").exists() {
        assert!(Instant::now() < deadline, "plan review did not start");
        std::thread::sleep(Duration::from_millis(20));
    }
    core.pause(&conversation, true).unwrap();
    std::fs::write(dir.join("plan-only-release"), "").unwrap();
    while events(core, &conversation, "plan_proposed").is_empty() {
        assert!(Instant::now() < deadline, "checked plan was not published");
        std::thread::sleep(Duration::from_millis(20));
    }
    core.cancel(&conversation);
    wait_idle(core, &conversation);
    conversation
}

#[test]
fn a_plan_the_reviewer_sends_back_is_revised_before_the_user_sees_it() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        r#"
if "[plan reviewer]" in prompt:
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
    let conversation = ask_plan(&core, dir.path(), "build a model and use it");

    let reviews = events(&core, &conversation, "plan_review");
    assert_eq!(
        reviews.len(),
        1,
        "{:?}",
        core.store.events_since(&conversation, 0, 1000).unwrap()
    );
    assert_eq!(reviews[0]["approved"], false);
    let proposed = events(&core, &conversation, "plan_proposed");
    assert_eq!(proposed.len(), 1, "only the checked plan is shown");
    assert_eq!(proposed[0]["review"]["approved"], true);
    assert_eq!(proposed[0]["tasks"][1]["title"], "Uses model, fixed");
    assert_eq!(
        proposed[0]["automatic"], true,
        "checked plans start without a permission gate"
    );

    let messages = core.store.messages_after(&conversation, 0).unwrap();
    let sent_back = messages
        .iter()
        .find(|message| message.author == "plan-reviewer")
        .unwrap();
    assert!(sent_back.text.contains("t2 needs t1's result"));
    let turns = core.store.events_since(&conversation, 0, 10_000).unwrap();
    let planning_roles: Vec<_> = turns
        .iter()
        .filter(|event| event.kind == "turn_started")
        .filter_map(|event| event.payload["role"].as_str())
        .filter(|role| matches!(*role, "planner" | "orchestrator" | "comment-orchestrator"))
        .collect();
    assert_eq!(
        planning_roles,
        ["planner", "planner"],
        "findings go directly to the planner"
    );
    let latest = core.store.latest_plan(&conversation).unwrap();
    assert!(latest.iter().all(|task| task.status != "approved"));
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
        r#"
broken = task("t1", "Change it")
broken["kind"] = "implementation"
broken["change_kind"] = "none"
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
    let conversation = ask_plan(&core, dir.path(), "change the code");

    // submit_plan refuses the plan within the turn, and the agent corrects it.
    let refused = ["codex", "agy"]
        .iter()
        .filter_map(|name| {
            std::fs::read_to_string(dir.path().join(format!("{name}.rejections"))).ok()
        })
        .collect::<String>();
    assert!(
        refused.contains("- t1 [implementation-has-no-delivery]"),
        "{refused}"
    );
    let reviews = events(&core, &conversation, "plan_review");
    assert!(reviews.is_empty(), "the reviewer never saw the broken plan");
    let proposed = events(&core, &conversation, "plan_proposed");
    assert_eq!(
        proposed.len(),
        1,
        "{refused}\n{:?}",
        core.store.events_since(&conversation, 0, 2000).unwrap()
    );
    assert_eq!(proposed[0]["tasks"][0]["title"], "Change it, readable");
}

#[test]
fn checked_plan_runs_automatically_and_reports_audit_gaps() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        r#"
if "The plan finished. Results of each task follow." in prompt:
    answer = "gap noted" if "not fully delivered" in prompt else "all done"
else:
    answer = plan(task("t1", "Analyse"))
"#,
        r#"
if "You review a plan" in prompt:
    say("plan", verdict(True))
elif "You audit" in prompt:
    say("audit", verdict(False, [{"severity": "high", "issue": "README not covered", "where": "README"}]))
elif "You review" in prompt:
    say("reviewer", verdict(True))
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
        .find(|message| {
            message.author == "runtime" && message.text.starts_with("The plan finished.")
        })
        .unwrap();
    assert!(
        report.text.contains("README not covered"),
        "{}",
        report.text
    );
    let reply = &messages.last().unwrap().text;
    assert!(
        reply.starts_with("gap noted\n") && reply.contains("\nAudit response:\n"),
        "{reply}"
    );
}

#[test]
fn repeated_checked_plans_continue_without_an_execution_count_gate() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        // A plan sent back for ledger mapping is regenerated, not a further step.
        r#"step = bump("plans", 0 if "Regenerate the task plan" in prompt else 1)
answer = plan(task("t1", "Step " + str(min(step, 4)))) if step <= 5 else "Repeated work verified.""#,
        r#"
if "You review" in prompt or "You review a plan" in prompt or "new plan:" in prompt \
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
    assert_eq!(automatic, [true, true, true, true, true]);
    assert!(proposed[4]["waiting"].is_null());
    let repeated = proposed[4]["plan"].as_str().unwrap();
    assert!(
        core.store
            .plan(repeated)
            .unwrap()
            .iter()
            .all(|task| task.status == "approved")
    );
}

#[test]
fn rounds_continue_while_the_worker_gets_closer() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        r#"answer = "done" if "The plan finished. Results of each task follow." in prompt and "You are the planner" not in prompt else plan(task("t1", "Fix"))"#,
        r#"
if "You review a plan" in prompt or "You audit" in prompt:
    say("reviewer", verdict(True))
elif "found problems" in prompt:
    say("worker", "report")
elif "You review" in prompt or "updated report" in prompt:
    n = bump("verdicts")
    say("reviewer", verdict(n >= 5, [] if n >= 5 else [{"severity": "high", "id":"case-%d" % n,"issue": "case %d" % n, "where": "a%d" % n}]))
else:
    say("worker", "report")
"#,
    );
    let conversation = ask(&core, dir.path(), "fix");

    let task = &core.store.latest_plan(&conversation).unwrap()[0];
    assert_eq!(task.status, "approved", "{:?}", task.report);
    assert_eq!(task.round, 5, "all required repair rounds run");
}

#[test]
fn repeated_review_findings_do_not_stop_repairs() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        r#"answer = "done" if "The plan finished. Results of each task follow." in prompt and "You are the planner" not in prompt else plan(task("t1", "Fix"))"#,
        r#"
if "You review a plan" in prompt or "You audit" in prompt:
    say("reviewer", verdict(True))
elif "found problems" in prompt:
    say("worker", "report")
elif "You review" in prompt or "updated report" in prompt:
    n = bump("verdicts")
    say("reviewer", verdict(n >= 6, [] if n >= 6 else [{"severity": "high", "issue": "still wrong", "where": "a"}]))
else:
    say("worker", "report")
"#,
    );
    let conversation = ask(&core, dir.path(), "fix");

    let task = &core.store.latest_plan(&conversation).unwrap()[0];
    assert_eq!(task.status, "approved", "{:?}", task.report);
    assert_eq!(task.round, 6);
    assert!(events(&core, &conversation, "notice").iter().any(|event| {
        event["text"]
            .as_str()
            .is_some_and(|text| text.contains("worker continues"))
    }));
}

#[test]
fn repairs_continue_beyond_sixteen_rounds() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        r#"answer = "done" if "The plan finished. Results of each task follow." in prompt and "You are the planner" not in prompt else plan(task("t1", "Fix"))"#,
        r#"
if "You review a plan" in prompt or "You audit" in prompt:
    say("reviewer", verdict(True))
elif "found problems" in prompt:
    say("worker", "report")
elif "You review" in prompt or "updated report" in prompt:
    n = bump("verdicts")
    say("reviewer", verdict(n >= 20, [] if n >= 20 else [{"severity": "high", "id": "new-%d" % n, "issue": "problem %d" % n, "where": "a%d" % n}]))
else:
    say("worker", "report")
"#,
    );
    let conversation = support::fixture_conversation(&core, &dir.path().join("project"))
        .unwrap()
        .id;
    core.send(&conversation, "fix").unwrap();
    let deadline = Instant::now() + Duration::from_secs(120);
    let task = loop {
        let tasks = core.store.latest_plan(&conversation).unwrap();
        if let Some(task) = tasks
            .into_iter()
            .find(|task| task.status == "approved" && task.round == 20)
        {
            break task;
        }
        assert!(
            Instant::now() < deadline,
            "twenty approved rounds were not observed; events={:?}",
            core.store
                .events_since(&conversation, 0, 200)
                .unwrap()
                .iter()
                .filter(|event| event.payload.get("error").is_some())
                .collect::<Vec<_>>()
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    // This test measures repair execution, not the deliberately negative final
    // audit supplied by the worker-only fixture. Cancel after observing success.
    core.cancel(&conversation);
    wait_idle(&core, &conversation);
    assert_eq!(task.status, "approved");
    assert_eq!(task.round, 20);
}

#[test]
fn reworded_obligations_remain_visible_without_a_stall_stop() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        r#"answer = "done" if "The plan finished. Results of each task follow." in prompt and "You are the planner" not in prompt else plan(task("t1", "Fix"))"#,
        r#"
if "You review a plan" in prompt or "You audit" in prompt:
    say("reviewer", verdict(True))
elif "found problems" in prompt:
    say("worker", "report")
elif "You review" in prompt or "updated report" in prompt:
    n = bump("verdicts")
    ids = re.findall(r"^- ([A-Za-z0-9_.-]+): ", prompt, re.M)
    cov = {i: {"status": "ok", "note": "checked"} for i in ids}
    if n < 5: cov["V1"] = {"status": "issue", "note": "open %d" % n}
    findings = [] if n >= 5 else [{"severity": "high", "id":"same-case","issue": "case %d" % n, "where": "a"}]
    say("reviewer", "```verdict\n" + json.dumps({"approved": n >= 5, "findings": findings, "coverage": cov}) + "\n```", False)
else:
    say("worker", "report")
"#,
    );
    let conversation = ask(&core, dir.path(), "fix");

    let task = &core.store.latest_plan(&conversation).unwrap()[0];
    assert_eq!(task.status, "approved", "{:?}", task.report);
    assert_eq!(task.round, 5);
}

#[test]
fn independently_reviewed_progress_continues_past_many_rounds() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        r#"answer = "done" if "The plan finished. Results of each task follow." in prompt and "You are the planner" not in prompt else plan(task("t1", "Fix"))"#,
        r#"
if "You review a plan" in prompt or "You audit" in prompt:
    say("reviewer", verdict(True))
elif "found problems" in prompt:
    say("worker", "report")
elif "You review" in prompt or "updated report" in prompt:
    n = bump("verdicts")
    ids = re.findall(r"^- ([A-Za-z0-9_.-]+): ", prompt, re.M)
    cov = {i: {"status": "ok", "note": "checked"} for i in ids}
    if n < 14: cov["V1" if n % 2 else "V2"] = {"status": "issue", "note": "open %d" % n}
    findings = [{"severity": "high", "id":"case-%d" % n,"issue": "case %d" % n, "where": "a%d" % n}]
    say("reviewer", "```verdict\n" + json.dumps({"approved": n >= 14, "findings": [] if n >= 14 else findings, "coverage": cov}) + "\n```", False)
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
if "The plan finished. Results of each task follow." in prompt:
    answer = "done"
else:
    answer = plan(dict(task("t1", "Revenue"), reviewer={"focus": "numbers", "risks": ["units of money"]}))
"#;

#[test]
fn a_verdict_that_skips_an_obligation_is_asked_again_and_never_approves() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        RISKY_PLAN,
        r#"
if "You review a plan" in prompt or "You audit" in prompt:
    say("reviewer", verdict(True))
elif "You review" in prompt or "does not account" in prompt:
    say("reviewer", json.dumps({"type":"review","report":"Skipped obligations","verdict":{"approved":True,"findings":[],"coverage":{}}}), covered=False)
else:
    say("worker", "report")
"#,
    );
    let conversation = support::fixture_conversation(&core, &dir.path().join("project"))
        .unwrap()
        .id;
    core.send(&conversation, "calculate revenue").unwrap();
    let deadline = Instant::now() + Duration::from_secs(120);
    while std::fs::read_to_string(dir.path().join("agy.rejections"))
        .map_or(0, |text| text.lines().count())
        < 5
    {
        assert!(
            Instant::now() < deadline,
            "invalid review stopped correcting"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    core.cancel(&conversation);
    wait_idle(&core, &conversation);

    let task = &core.store.latest_plan(&conversation).unwrap()[0];
    assert_ne!(task.status, "approved");
    assert!(
        std::fs::read_to_string(dir.path().join("agy.rejections"))
            .unwrap()
            .contains("give a status and note for every listed check")
    );
    assert!(task.verdict.is_none());
}

#[test]
fn an_obligation_marked_issue_sends_its_note_to_the_worker() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        RISKY_PLAN,
        r#"
import re
ids = re.findall(r"^- ([A-Za-z0-9_.-]+): ", prompt, re.M)
if "You review a plan" in prompt or "You audit" in prompt:
    say("reviewer", verdict(True))
elif "You review" in prompt and "fixed: converted" not in prompt:
    # The units check, as an issue: the rest is asked for again.
    lines = re.findall(r"^- ([A-Za-z0-9_.-]+): (.*)$", prompt, re.M)
    uid = next(i for i, text in lines if "units, denominators" in text)
    coverage = {i:{"status":"ok","note":"checked"} for i in ids}
    coverage[uid] = {"status":"issue","note":"EUR added to PLN"}
    say("reviewer", "```verdict\n" + json.dumps({"approved": False, "findings": [], "coverage": coverage}) + "\n```", covered=False)
elif "does not account" in prompt:
    assert "R1" not in ids, ids
    say("reviewer", verdict(True))
elif "found problems" in prompt:
    say("worker", "fixed: " + ("EUR added to PLN" in prompt and "converted" or "?"))
elif "updated report" in prompt or ("You review" in prompt and "fixed: converted" in prompt):
    say("reviewer", verdict(True))
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
        r#"
if "The plan finished. Results of each task follow." in prompt or "user question marker" in prompt:
    answer = "answer"
else:
    answer = plan(task("t1", "Analyse"))
"#,
        r#"
if "You review" in prompt or "You review a plan" in prompt or "You audit" in prompt:
    say("reviewer", verdict(True))
else:
    say("worker", "report")
"#,
    );
    let conversation = ask_plan(&core, dir.path(), "analysis");

    let plan = core.store.latest_plan(&conversation).unwrap()[0]
        .plan_id
        .clone();
    core.resume_plan(&conversation, &plan).unwrap();
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
fn runtime_refuses_a_cyclic_persisted_graph_before_starting_workers() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        r#"answer = plan(task("t1", "Scout"), task("t2", "Consumer", ["t1"]))"#,
        r#"
if "You review a plan" in prompt:
    say("reviewer", verdict(True))
else:
    raise RuntimeError("no worker may start for an invalid graph")
"#,
    );
    let conversation = ask_plan(&core, dir.path(), "inventory and review");
    let mut tasks = core.store.latest_plan(&conversation).unwrap();
    tasks[0].spec["depends_on"] = serde_json::json!(["t2"]);
    let db = rusqlite::Connection::open(dir.path().join("state/core.db")).unwrap();
    db.execute(
        "UPDATE tasks SET spec=?1 WHERE id=?2",
        rusqlite::params![tasks[0].spec.to_string(), tasks[0].id],
    )
    .unwrap();
    let error = core
        .resume_plan(&conversation, &tasks[0].plan_id)
        .unwrap_err();
    assert!(error.to_string().contains("not an earlier task"), "{error}");
    assert!(
        core.store
            .latest_plan(&conversation)
            .unwrap()
            .iter()
            .all(|task| task.status != "approved" && task.round <= 1)
    );
}

#[test]
fn plan_corrections_continue_beyond_three_rejections_when_findings_close() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        r#"answer=plan(task('t1','Focused plan'))"#,
        r#"
if 'You review a plan' in prompt or 'new plan:' in prompt:
    n=bump('plan-review-count')
    findings=[] if n>=6 else [{'id':'plan-%d'%i,'severity':'high','issue':'Missing dependency %d'%i,'where':'t1'} for i in range(n,6)]
    say('plan',verdict(n>=6,findings))
"#,
    );
    let conversation = ask_plan(&core, dir.path(), "Plan the local work");
    let proposed = events(&core, &conversation, "plan_proposed");
    assert_eq!(proposed.len(), 1);
    assert_eq!(proposed[0]["review"]["approved"], true);
    assert_eq!(events(&core, &conversation, "plan_review").len(), 5);
}

#[test]
fn reworded_plan_findings_do_not_impose_a_retry_budget() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        r#"answer="done" if "The plan finished. Results of each task follow." in prompt else plan(task('t1','Focused plan'))"#,
        r#"
if 'You review a plan' in prompt or 'new plan:' in prompt:
    n=bump('plan-review-count')
    say('plan',verdict(n>=5,[] if n>=5 else [{'id':'missing-input','severity':'high','issue':'Missing input, wording %d'%n,'where':'t1'}]))
"#,
    );
    let conversation = ask_plan(&core, dir.path(), "Plan the local work");
    let proposed = events(&core, &conversation, "plan_proposed");
    assert_eq!(proposed.len(), 1);
    assert_eq!(proposed[0]["review"]["approved"], true);
    let starts = events(&core, &conversation, "turn_started");
    assert_eq!(
        starts
            .iter()
            .filter(|e| e["role"] == "plan-reviewer")
            .count(),
        5
    );
    assert!(events(&core, &conversation, "notice").iter().any(|e| {
        e["text"]
            .as_str()
            .is_some_and(|s| s.contains("closure of a previous finding"))
    }));
}

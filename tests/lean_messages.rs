mod common;
mod support;
use antislop::{config::Config, lean::Core};
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
    ids = re.findall(r"^- ([A-Za-z0-9_.-]+): ", prompt, re.M)
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
        support::install_ledger_protocol(&path);
        std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        common::interactive(&path);
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
    let text = replacements
        .iter()
        .fold(text, |text, (from, to)| text.replace(from, to));
    let project = dir.join("project");
    std::fs::create_dir_all(&project).unwrap();
    // The requirements audit fingerprints the integrated tree, which needs a repository.
    for args in [
        &["init", "--quiet", "-b", "main"][..],
        &["commit", "--quiet", "--allow-empty", "-m", "init"][..],
    ] {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(&project)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@test.com")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@test.com")
            .status()
            .unwrap();
        assert!(status.success());
    }
    Core::open(&dir.join("state"), Config::from_toml(&text).unwrap()).unwrap()
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(120);
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
    let conversation = support::fixture_conversation(core, &dir.join("project"))
        .unwrap()
        .id;
    core.send(&conversation, "zbadaj").unwrap();
    wait_until("plan proposed", || core.is_idle(&conversation));
    let plan = core.store.latest_plan(&conversation).unwrap()[0]
        .plan_id
        .clone();
    core.accept_plan(&conversation, &plan).unwrap();
    conversation
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
    let sent = core.submit(&conversation, "@t1 add a UTF-8 case").unwrap();
    assert_eq!(sent["recipient_finished"], false);
    wait_until("plan finished", || core.is_idle(&conversation));

    let task = &core.store.latest_plan(&conversation).unwrap()[0];
    assert_eq!(task.report.as_deref(), Some("report with UTF-8 case"));
    let all = prompts(dir.path());
    let extra = all
        .iter()
        .find(|(_, prompt)| {
            prompt.contains("add a UTF-8 case") && prompt.contains("Task role: worker")
        })
        .expect("the worker got a turn for the message");
    assert!(
        extra
            .1
            .contains("- from user (agent_work): add a UTF-8 case")
    );
    assert!(
        core.store
            .mail(&conversation)
            .unwrap()
            .iter()
            .any(|m| m.text == "add a UTF-8 case" && m.delivered_turn.is_some())
    );
}

#[test]
fn a_comment_reaches_the_orchestrator_and_stays_until_the_user_resolves_it() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        r#"
answer = "noted" if "[runtime]" in prompt or "[user comment" in prompt or "[user]\ncontinue" in prompt else plan(task("t1", "Parser"))
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
    core.send(&conversation, "continue").unwrap();
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
    core.send(&conversation, "continue").unwrap();
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
fn a_turn_stores_its_whole_answer_and_no_mirror_of_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        r#"
answer = "First sentence.\nSecond line of the answer that is kept whole."
"#,
        "    answer = 'unused'",
    );
    // The orchestrator answers in words while the interview is open.
    let conversation = core
        .store
        .create_conversation(dir.path().join("project").to_str().unwrap())
        .unwrap()
        .id;
    core.send(&conversation, "hello there").unwrap();
    wait_until("answered", || core.is_idle(&conversation));
    let finals = events(&core, &conversation, "agent_final");
    assert_eq!(finals.len(), 1);
    assert_eq!(
        finals[0]["text"],
        "First sentence.\nSecond line of the answer that is kept whole."
    );
    // What the agent read, thought and ran stays in its own session.
    assert!(events(&core, &conversation, "agent").is_empty());
}

fn plan_with_two_draft_comments(core: &Arc<Core>, dir: &Path) -> (String, String, Vec<u64>) {
    let conversation = support::fixture_conversation(core, &dir.join("project"))
        .unwrap()
        .id;
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

// Start with a persisted draft, so the comment revision is the first proposal
// to encounter the global requirements/options gate (not per-task options).
fn deferred_comment_revision(current_list: bool, new_user_input: bool) {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        &r#"
import re
ids = re.findall(r"user comment #(\d+) on", prompt)
regenerated = "Regenerate the task plan from the verified ledger" in _ledger_prompt
def claim(n):
    return "Revised.\n```comments\n" + json.dumps({"addressed": [{"id": int(ids[n]), "how": "changed"}]}) + "\n```\n" + plan(task("t1", "Revised parser"))
if not ids:
    answer = "Noted."
elif regenerated:
    # The regenerated plan restates the claim only when the agent lists comments again.
    answer = claim(1) if CURRENT_LIST else plan(task("t1", "Revised parser"))
else:
    answer = claim(0)
"#
        .replace("CURRENT_LIST", if current_list { "True" } else { "False" }),
        "    answer = verdict(True)",
    );
    // The existing ledger fixture supplies real protocol responses. Only gate
    // the regenerated plan the ledger mapping asks for (the first proposal
    // carries no requirement ids); requirements still run in production.
    let codex = dir.path().join("codex");
    let source = std::fs::read_to_string(&codex).unwrap();
    let source = source.replace(
        "if len(_ledger_context)==2:\n    prompt = _ledger_context[0].rstrip()",
        r#"if 'Regenerate the task plan from the verified ledger' in _ledger_prompt:
    open(_lo.path.join(_lo.path.dirname(sys.argv[0]), 'next-plan-started'), 'w').close()
    import time
    deadline = time.monotonic() + 15
    while not _lo.path.exists(_lo.path.join(_lo.path.dirname(sys.argv[0]), 'release-next-plan')):
        if time.monotonic() >= deadline:
            raise RuntimeError('next plan was not released by test within 15 seconds')
        time.sleep(0.01)
if len(_ledger_context)==2:
    prompt = _ledger_context[0].rstrip()"#,
    );
    assert!(source.contains("next-plan-started"));
    std::fs::write(&codex, source).unwrap();
    let conversation = support::fixture_conversation(&core, &dir.path().join("project"))
        .unwrap()
        .id;
    core.store
        .add_message(&conversation, "user", "plan it")
        .unwrap();
    let original = core
        .store
        .add_plan(
            &conversation,
            &[serde_json::json!({
                "id":"t1", "title":"Parser", "goal":"g", "kind":"analysis",
                "verifier":{"focus":"f"}
            })],
        )
        .unwrap()[0]
        .plan_id
        .clone();
    let ids: Vec<u64> = ["first remark", "second remark"]
        .iter()
        .enumerate()
        .map(|(n, text)| {
            core.comment_scoped(
                &conversation,
                Some(&original),
                &format!("plan:{original}:line:{}", n + 1),
                text,
            )
            .unwrap()
            .id
        })
        .collect();
    core.review_comments(&conversation, &original).unwrap();
    wait_until("the ledger mapping must defer the comment proposal", || {
        dir.path().join("next-plan-started").exists() || core.is_idle(&conversation)
    });
    assert!(
        dir.path().join("next-plan-started").exists(),
        "{:#?}",
        core.store.events_since(&conversation, 0, 1000).unwrap()
    );
    let claims = events(&core, &conversation, "plan_comments_addressed");
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0]["ids"], serde_json::json!([ids[0]]));
    let boundary = claims[0]["upto_message"].as_u64().unwrap();
    assert_eq!(
        boundary,
        core.store
            .last_orchestration_answered(&conversation)
            .unwrap()
    );
    let comments = core.store.comments(&conversation, "open").unwrap();
    assert_eq!(comments.iter().map(|c| c.id).collect::<Vec<_>>(), ids);
    assert!(comments.iter().all(|c| c.message <= boundary));
    assert_eq!(
        core.store.pending_plan_comment_ids(&conversation).unwrap(),
        Some(vec![ids[0]])
    );
    assert!(events(&core, &conversation, "plan_proposed").is_empty());
    // The rejected first proposal stays as a revised row; no new plan is live.
    let live: Vec<String> = core
        .store
        .plans_page(&conversation, 0, 20)
        .unwrap()
        .plans
        .into_iter()
        .filter(|plan| plan.status != "revised")
        .map(|plan| plan.id)
        .collect();
    assert_eq!(live, std::slice::from_ref(&original));
    let options_messages = core
        .store
        .messages_by(&conversation, "requirements-planner", 0)
        .unwrap();
    assert_eq!(
        options_messages.len(),
        1,
        "the ledger mapping requested a fresh plan"
    );
    assert!(options_messages[0].seq > boundary);
    if new_user_input {
        // Persist actual user input while the next agent is already running.
        // This must invalidate the earlier selective claim immediately.
        core.store
            .add_message(&conversation, "user", "new planning instruction")
            .unwrap();
        assert_eq!(
            core.store.pending_plan_comment_ids(&conversation).unwrap(),
            None
        );
    }
    std::fs::write(dir.path().join("release-next-plan"), "continue").unwrap();
    wait_until("checked replacement plan", || core.is_idle(&conversation));
    assert_ne!(
        core.store.latest_plan(&conversation).unwrap()[0].plan_id,
        original
    );
    assert!(!events(&core, &conversation, "plan_proposed").is_empty());
    let resolved: Vec<u64> = core
        .store
        .comments(&conversation, "resolved")
        .unwrap()
        .iter()
        .map(|c| c.id)
        .collect();
    let open: Vec<u64> = core
        .store
        .comments(&conversation, "open")
        .unwrap()
        .iter()
        .map(|c| c.id)
        .collect();
    if new_user_input {
        // Without a selective claim, the normal checked-plan rule applies.
        assert_eq!(resolved, ids);
        assert!(open.is_empty());
    } else {
        let selected = usize::from(current_list);
        assert_eq!(resolved, vec![ids[selected]]);
        assert_eq!(open, vec![ids[1 - selected]]);
    }
    assert_eq!(
        core.store.pending_plan_comment_ids(&conversation).unwrap(),
        None
    );
}

#[test]
fn deferred_options_proposal_preserves_selective_comment_claim() {
    deferred_comment_revision(false, false);
}

#[test]
fn deferred_options_proposal_prefers_the_current_explicit_comment_list() {
    deferred_comment_revision(true, false);
}

#[test]
fn deferred_options_comment_claim_expires_on_new_user_input() {
    deferred_comment_revision(false, true);
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
    let conversation = support::fixture_conversation(&core, &dir.join("project"))
        .unwrap()
        .id;
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
    let conversation = support::fixture_conversation(&core, &dir.path().join("project"))
        .unwrap()
        .id;
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
    let conversation = support::fixture_conversation(&core, &dir.path().join("project"))
        .unwrap()
        .id;
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
    let conversation = support::fixture_conversation(&core, &dir.path().join("project"))
        .unwrap()
        .id;
    core.send(&conversation, "change a.rs").unwrap();
    wait_until("plan proposed", || core.is_idle(&conversation));
    assert!(
        events(&core, &conversation, "notice")
            .iter()
            .all(|n| !n["text"].as_str().unwrap().contains("required_gates"))
    );
}

#[test]
fn a_dependent_gets_the_summary_of_its_dependency_not_its_report() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        r#"
answer = "done" if "[runtime]" in prompt else plan(task("t1", "Audit"), task("t2", "Gate", ["t1"]), task("t3", "Docs"))
"#,
        r#"    answer = json.dumps({"type": "completed", "summary": "audit summary S1", "report": "audit found F1 and F2"}) if "Task id: t1" in prompt else "report""#,
    );
    let conversation = run(&core, dir.path());
    wait_until("plan finished", || core.is_idle(&conversation));

    let worker_prompts: Vec<String> = prompts(dir.path())
        .into_iter()
        .filter(|(bin, prompt)| bin == "agy" && prompt.contains("Task id:"))
        .map(|(_, prompt)| prompt)
        .collect();
    let gate = worker_prompts
        .iter()
        .find(|prompt| prompt.contains("Task id: t2"))
        .unwrap();
    assert!(gate.contains("audit summary S1"), "{gate}");
    assert!(!gate.contains("audit found F1 and F2"), "{gate}");
    let docs = worker_prompts
        .iter()
        .find(|prompt| prompt.contains("Task id: t3"))
        .unwrap();
    assert!(
        !docs.contains("audit summary") && !docs.contains("audit found"),
        "only dependents get it"
    );
}

#[test]
fn direct_agent_messages_resume_work_and_recheck_without_receipt_only_turns() {
    use antislop::lean::store::AgentMessage;
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        "answer = 'done' if '[runtime]' in prompt else plan(task('t1', 'Audit'))",
        r#"    answer = verdict(True) if "The worker's updated report:" in prompt else ('changed report' if 'make correction' in prompt else 'initial report')"#,
    );
    let conversation = run(&core, dir.path());
    wait_until("initial approval", || core.is_idle(&conversation));
    let first = core.store.latest_plan(&conversation).unwrap().remove(0);
    let mut message = AgentMessage {
        conversation: conversation.clone(),
        plan: Some(first.plan_id.clone()),
        recipient: format!("worker:{}", first.id),
        operation_id: uuid::Uuid::new_v4().to_string(),
        expected_round: Some(first.round),
        expected_run: None,
        text: "make correction".into(),
    };
    let receipt = core.send_to_agent(&message).unwrap();
    assert_eq!(
        core.send_to_agent(&message).unwrap()["mail"],
        receipt["mail"]
    );
    wait_until("corrected and verified", || {
        core.is_idle(&conversation)
            && core
                .store
                .task_by_id(&first.id)
                .unwrap()
                .is_some_and(|task| task.round > first.round && task.status == "approved")
    });
    let corrected = core.store.task_by_id(&first.id).unwrap().unwrap();
    assert_eq!(corrected.report.as_deref(), Some("changed report"));
    assert_eq!(
        core.store.task_round(&first.id, first.round).unwrap()["report"],
        "initial report"
    );
    let before = prompts(dir.path()).len();
    message.recipient = format!("verifier:{}", first.id);
    message.operation_id = uuid::Uuid::new_v4().to_string();
    message.expected_round = Some(corrected.round);
    message.text = "recheck correction".into();
    core.send_to_agent(&message).unwrap();
    wait_until("rechecked", || {
        core.is_idle(&conversation)
            && core
                .store
                .task_by_id(&first.id)
                .unwrap()
                .is_some_and(|task| task.round > corrected.round && task.status == "approved")
    });
    let rechecked = core.store.task_by_id(&first.id).unwrap().unwrap();
    assert_eq!(rechecked.report, corrected.report);
    let later = prompts(dir.path());
    assert!(
        !later[before..]
            .iter()
            .any(|(_, prompt)| prompt.contains("receipt-only"))
    );
    assert!(
        later[before..]
            .iter()
            .any(|(_, prompt)| prompt.contains("recheck correction")
                && prompt.contains("Your obligations (the task's goal"))
    );
}

#[test]
fn direct_worker_correction_invalidates_and_recomputes_the_dependency_chain() {
    use antislop::lean::store::AgentMessage;
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        "answer = 'done' if '[runtime]' in prompt else plan(dict(task('t1', 'Source'),outputs=['api']),dict(task('t2', 'Consumer', ['t1']),inputs=[{'name':'api','task':'t1','output':'api'}]))",
        r#"    if "The worker's updated report:" in prompt:
        answer = verdict(True)
    elif "make correction" in prompt:
        answer = 'new upstream result'
    elif "new upstream result" in prompt:
        answer = 'consumer used new result'
    else:
        answer = 'initial report'"#,
    );
    let conversation = run(&core, dir.path());
    wait_until("initial chain", || core.is_idle(&conversation));
    let initial = core.store.latest_plan(&conversation).unwrap();
    let source = &initial[0];
    core.send_to_agent(&AgentMessage {
        conversation: conversation.clone(),
        plan: Some(source.plan_id.clone()),
        recipient: format!("worker:{}", source.id),
        operation_id: uuid::Uuid::new_v4().to_string(),
        expected_round: Some(source.round),
        expected_run: None,
        text: "make correction".into(),
    })
    .unwrap();
    wait_until("recomputed chain", || {
        core.is_idle(&conversation)
            && core
                .store
                .latest_plan(&conversation)
                .unwrap()
                .iter()
                .zip(&initial)
                .all(|(task, old)| task.status == "approved" && task.round > old.round)
    });
    let updated = core.store.latest_plan(&conversation).unwrap();
    assert_eq!(
        updated[1].report.as_deref(),
        Some("consumer used new result")
    );
    assert!(updated[1].input_generation > initial[1].input_generation);
}

#[test]
fn a_huge_answer_and_a_long_request_are_kept_whole() {
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        r#"
answer = "ąe\u0301👩‍💻" * 300000 + "\n```plan\n" + json.dumps({"tasks": [task("t1", "Audit")]}) + "\n```"
"#,
        "    answer = 'unused'",
    );
    let conversation = support::fixture_conversation(&core, &dir.path().join("project"))
        .unwrap()
        .id;
    let request = "full request 👩‍💻 ".repeat(3500);
    core.send(&conversation, &request).unwrap();
    wait_until("large response", || core.is_idle(&conversation));
    let expected = "ąe\u{301}👩‍💻".repeat(300000);
    assert!(expected.len() > antislop::transport::MAX_FRAME);
    assert_eq!(
        core.store
            .messages_by(&conversation, "orchestrator", 0)
            .unwrap()
            .last()
            .unwrap()
            .text,
        expected
    );
    let events = core.store.events_since(&conversation, 0, 500).unwrap();
    // The plan's words travel inside the submitted result, whole.
    let submitted: serde_json::Value = events
        .iter()
        .filter(|event| event.kind == "agent_final")
        .filter_map(|event| serde_json::from_str(event.payload["text"].as_str()?).ok())
        .find(|result: &serde_json::Value| result["type"] == "plan")
        .unwrap();
    assert_eq!(submitted["document"], expected);
    // The CLI was typed the whole request, whatever its size.
    let typed = prompts(dir.path());
    assert!(typed.iter().any(|(_, prompt)| prompt.contains(&request)));
}

#[test]
fn worker_message_during_verification_requires_another_round_before_approval() {
    use antislop::lean::store::AgentMessage;
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        "answer = 'done' if '[runtime]' in prompt else plan(task('t1', 'Audit'))",
        r#"    answer = verdict(True) if "The worker's updated report:" in prompt else ('corrected report' if 'correct during verification' in prompt else 'first report')"#,
    );
    let binary = dir.path().join("agy");
    let script = std::fs::read_to_string(&binary).unwrap().replace(
        "if review:\n    answer = verdict(True)",
        "if review:\n    time.sleep(0.4)\n    answer = verdict(True)",
    );
    std::fs::write(binary, script).unwrap();
    let conversation = run(&core, dir.path());
    wait_until("verifier started", || {
        core.store.latest_plan(&conversation).unwrap()[0].status == "verifying"
    });
    let task = core.store.latest_plan(&conversation).unwrap().remove(0);
    core.send_to_agent(&AgentMessage {
        conversation: conversation.clone(),
        plan: Some(task.plan_id.clone()),
        recipient: format!("worker:{}", task.id),
        operation_id: uuid::Uuid::new_v4().to_string(),
        expected_round: Some(task.round),
        expected_run: None,
        text: "correct during verification".into(),
    })
    .unwrap();
    wait_until("correction verified", || {
        core.is_idle(&conversation)
            && core
                .store
                .task_by_id(&task.id)
                .unwrap()
                .is_some_and(|updated| updated.status == "approved" && updated.round > task.round)
    });
    assert_eq!(
        core.store
            .task_by_id(&task.id)
            .unwrap()
            .unwrap()
            .report
            .as_deref(),
        Some("corrected report")
    );
}

#[test]
fn direct_orchestrator_message_uses_normal_workflow_and_validates_the_run() {
    use antislop::lean::store::AgentMessage;
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        "answer = 'response: ' + prompt.splitlines()[-1]",
        "    answer = 'unused'",
    );
    // The orchestrator answers in words while the interview is open.
    let conversation = core
        .store
        .create_conversation(dir.path().join("project").to_str().unwrap())
        .unwrap()
        .id;
    core.send(&conversation, "first question").unwrap();
    wait_until("first answer", || core.is_idle(&conversation));
    let first = core
        .store
        .last_turn(&conversation, "orchestrator")
        .unwrap()
        .unwrap();
    let message = AgentMessage {
        conversation: conversation.clone(),
        plan: None,
        recipient: "orchestrator".into(),
        operation_id: uuid::Uuid::new_v4().to_string(),
        expected_round: None,
        expected_run: Some(first.id.clone()),
        text: "next question".into(),
    };
    core.send_to_agent(&message).unwrap();
    wait_until("direct answer", || core.is_idle(&conversation));
    assert_eq!(
        core.store
            .messages_by(&conversation, "orchestrator", 0)
            .unwrap()
            .last()
            .unwrap()
            .text,
        "response: next question"
    );
    assert!(
        core.store.mail(&conversation).unwrap()[0]
            .delivered_turn
            .is_some()
    );
    assert!(
        !prompts(dir.path())
            .iter()
            .any(|(_, prompt)| prompt.contains("receipt-only"))
    );
    core.send_to_agent(&message).unwrap();
    assert_eq!(
        core.store
            .messages_by(&conversation, "user", 0)
            .unwrap()
            .len(),
        2
    );
    let stale = AgentMessage {
        operation_id: uuid::Uuid::new_v4().to_string(),
        ..message
    };
    assert!(
        core.send_to_agent(&stale)
            .unwrap_err()
            .to_string()
            .contains("stale run")
    );
}

#[test]
fn direct_verifier_recheck_keeps_unchanged_dependents_and_rejection_recomputes_them() {
    use antislop::lean::store::AgentMessage;
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        "answer = 'done' if '[runtime]' in prompt else plan(dict(task('t1', 'Source'),outputs=['api']),dict(task('t2', 'Consumer', ['t1']),inputs=[{'name':'api','task':'t1','output':'api'}]))",
        r#"    if "The worker's updated report:" in prompt or "You verify another agent's work." in prompt:
        answer = verdict(False) if 'reject the old report' in prompt else verdict(True)
    else:
        answer = 'worker report'"#,
    );
    // Let this fixture's task verifier decide both fresh and continued turns;
    // plan reviewers still use the helper's automatic approval.
    let binary = dir.path().join("agy");
    let script = std::fs::read_to_string(&binary).unwrap();
    std::fs::write(
        &binary,
        script.replace(
            "\"You review a plan\", \"You audit\", \"You verify\"",
            "\"You review a plan\", \"You audit\"",
        ),
    )
    .unwrap();
    let conversation = run(&core, dir.path());
    wait_until("initial chain", || core.is_idle(&conversation));
    let initial = core.store.latest_plan(&conversation).unwrap();
    let mut message = AgentMessage {
        conversation: conversation.clone(),
        plan: Some(initial[0].plan_id.clone()),
        recipient: format!("verifier:{}", initial[0].id),
        operation_id: uuid::Uuid::new_v4().to_string(),
        expected_round: Some(initial[0].round),
        expected_run: None,
        text: "please check again".into(),
    };
    core.send_to_agent(&message).unwrap();
    wait_until("unchanged recheck", || {
        core.is_idle(&conversation)
            && core
                .store
                .task_by_id(&initial[0].id)
                .unwrap()
                .unwrap()
                .round
                > initial[0].round
    });
    let rechecked = core.store.latest_plan(&conversation).unwrap();
    assert_eq!(rechecked[1].round, initial[1].round);
    assert_eq!(rechecked[1].input_generation, initial[1].input_generation);
    assert_eq!(rechecked[1].status, "approved");
    message.operation_id = uuid::Uuid::new_v4().to_string();
    message.expected_round = Some(rechecked[0].round);
    message.text = "reject the old report".into();
    core.send_to_agent(&message).unwrap();
    wait_until("corrected recheck", || {
        core.is_idle(&conversation)
            && core
                .store
                .task_by_id(&initial[1].id)
                .unwrap()
                .unwrap()
                .round
                > initial[1].round
    });
    let corrected = core.store.latest_plan(&conversation).unwrap();
    assert_eq!(corrected[0].status, "approved");
    assert_eq!(corrected[1].status, "approved");
    assert!(corrected[1].input_generation > initial[1].input_generation);
    let rejected_round = core
        .store
        .task_round(&initial[0].id, rechecked[0].round + 1)
        .unwrap();
    assert_eq!(rejected_round["verdict"]["approved"], false);
    assert!(prompts(dir.path()).iter().any(|(_, prompt)| {
        prompt.contains("Task id: t1")
            && prompt.contains(&format!("Round: {}", rechecked[0].round + 2))
            && prompt.contains("Latest worker report")
            && prompt.contains("Your verifier found problems:")
            && prompt.contains("The fixture rejected the work")
    }));
}

#[test]
fn direct_worker_failure_remains_queued_and_is_never_consumed_by_a_receipt_turn() {
    use antislop::lean::store::AgentMessage;
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("failed-once");
    let agent = format!(
        r#"    import pathlib
    marker=pathlib.Path({marker:?})
    if 'fail direct delivery' in prompt and not marker.exists():
        marker.touch()
        print('authentication required', file=sys.stderr)
        sys.exit(1)
    answer = verdict(True) if "The worker's updated report:" in prompt else 'worker report'"#,
        marker = marker.to_string_lossy()
    );
    let core = open(
        dir.path(),
        "answer = 'done' if '[runtime]' in prompt else plan(task('t1', 'Source'))",
        &agent,
    );
    let conversation = run(&core, dir.path());
    wait_until("initial approval", || core.is_idle(&conversation));
    let first = core.store.latest_plan(&conversation).unwrap().remove(0);
    let mut message = AgentMessage {
        conversation: conversation.clone(),
        plan: Some(first.plan_id.clone()),
        recipient: format!("worker:{}", first.id),
        operation_id: uuid::Uuid::new_v4().to_string(),
        expected_round: Some(first.round),
        expected_run: None,
        text: "fail direct delivery".into(),
    };
    let receipt = core.send_to_agent(&message).unwrap();
    wait_until("failed direct turn", || {
        core.is_idle(&conversation)
            && matches!(
                core.store
                    .task_by_id(&first.id)
                    .unwrap()
                    .unwrap()
                    .status
                    .as_str(),
                "failed" | "waiting"
            )
    });
    assert_eq!(
        core.store
            .pending_mail(&conversation, &message.recipient)
            .unwrap()
            .len(),
        1
    );
    let events = core.store.events_since(&conversation, 0, 1000).unwrap();
    assert!(
        events
            .iter()
            .any(|event| event.kind == "mail_delivery_failed"
                && event.payload["ids"][0] == receipt["mail"])
    );
    let failed = core.store.task_by_id(&first.id).unwrap().unwrap();
    message.operation_id = uuid::Uuid::new_v4().to_string();
    message.expected_round = Some(failed.round);
    message.text = "retry the correction".into();
    core.send_to_agent(&message).unwrap();
    wait_until("retried and verified", || {
        core.is_idle(&conversation)
            && core.store.task_by_id(&first.id).unwrap().unwrap().status == "approved"
    });
    assert!(
        core.store
            .pending_mail(&conversation, &message.recipient)
            .unwrap()
            .is_empty()
    );
    assert!(
        !prompts(dir.path())
            .iter()
            .any(|(_, prompt)| prompt.contains("receipt-only"))
    );
}

#[test]
fn direct_reviewers_receive_review_context_instead_of_receipt_only_instructions() {
    use antislop::lean::store::AgentMessage;
    let dir = tempfile::tempdir().unwrap();
    let core = open(
        dir.path(),
        "answer = 'done' if '[runtime]' in prompt else plan(task('t1', 'Source'))",
        "    answer = verdict(True) if \"The worker's updated report:\" in prompt else 'worker report'",
    );
    let conversation = run(&core, dir.path());
    wait_until("initial reviews", || core.is_idle(&conversation));
    let task = core.store.latest_plan(&conversation).unwrap().remove(0);
    for role in ["plan-verifier", "auditor"] {
        let sessions = core.store.sessions(&conversation).unwrap();
        let run = sessions
            .iter()
            .rev()
            .find(|session| session.role == role)
            .unwrap()
            .turn_id
            .clone();
        let before = prompts(dir.path()).len();
        core.send_to_agent(&AgentMessage {
            conversation: conversation.clone(),
            plan: Some(task.plan_id.clone()),
            recipient: role.into(),
            operation_id: uuid::Uuid::new_v4().to_string(),
            expected_round: None,
            expected_run: Some(run.clone()),
            text: "review this concern again".into(),
        })
        .unwrap();
        wait_until("direct reviewer", || {
            core.is_idle(&conversation)
                && core
                    .store
                    .sessions(&conversation)
                    .unwrap()
                    .iter()
                    .any(|session| {
                        session.role == role
                            && session.turn_id != run
                            && !sessions.iter().any(|old| old.turn_id == session.turn_id)
                    })
        });
        let later = prompts(dir.path());
        let review = &later[before..];
        assert!(
            review
                .iter()
                .any(|(_, prompt)| prompt.contains("review this concern again"))
        );
        assert!(
            !review
                .iter()
                .any(|(_, prompt)| prompt.contains("receipt-only"))
        );
    }
}

#[test]
fn the_orchestrator_hands_the_whole_plan_back_to_the_planner() {
    let dir = tempfile::tempdir().unwrap();
    // Planner and orchestrator share the fake codex; the planner's prompt says so.
    let core = open(
        dir.path(),
        r#"
import os,urllib.request
if 'Draft the complete plan for review' in prompt:
    answer = plan(task('t2', 'Seam two')) if '[replan request]' in prompt else plan(task('t1', 'Seam one'))
elif 'please replan' in prompt:
    body={'jsonrpc':'2.0','id':1,'method':'tools/call','params':{'name':'request_replan','arguments':{'turn':os.environ['FIXTURE_TURN'],'reason':'split the work along another seam'}}}
    req=urllib.request.Request(os.environ['ANTISLOP_MCP_URL'],json.dumps(body).encode(),{'Authorization':'Bearer '+os.environ['ANTISLOP_MCP_TOKEN'],'Content-Type':'application/json'})
    reply=json.load(urllib.request.urlopen(req,timeout=10))
    assert not reply['result'].get('isError'),reply
    answer='Asked the planner to redraft the plan.'
else:
    answer='done'
"#,
        "    answer = 'unused'",
    );
    let conversation = support::fixture_conversation(&core, &dir.path().join("project"))
        .unwrap()
        .id;
    core.send(&conversation, "plan it").unwrap();
    wait_until("first plan", || core.is_idle(&conversation));
    let first = core.store.latest_plan(&conversation).unwrap();
    assert_eq!(first[0].spec["id"], "t1");

    core.send(&conversation, "please replan").unwrap();
    wait_until("replacement plan", || {
        core.is_idle(&conversation)
            && core
                .store
                .latest_plan(&conversation)
                .unwrap()
                .first()
                .is_some_and(|task| task.spec["id"] == "t2")
    });
    let second = core.store.latest_plan(&conversation).unwrap();
    assert_ne!(second[0].plan_id, first[0].plan_id);
    let reasons = core
        .store
        .messages_by(&conversation, antislop::lean::REPLAN, 0)
        .unwrap();
    assert_eq!(reasons.len(), 1);
    assert!(reasons[0].text.contains("another seam"));
    let drafts = core
        .store
        .sessions(&conversation)
        .unwrap()
        .iter()
        .filter(|turn| turn.role == antislop::lean::PLANNER)
        .count();
    assert_eq!(drafts, 2, "the planner drafted both plans");
    // The planner saw the plan it replaces and the reason.
    let redraft = prompts(dir.path())
        .into_iter()
        .map(|(_, prompt)| prompt)
        .find(|prompt| prompt.contains("[replan request]"))
        .expect("the planner was asked to redraft");
    assert!(redraft.contains("another seam") && redraft.contains("Seam one"));
}

#[test]
fn isolated_worker_can_ask_the_orchestrator_with_the_only_task_slot_occupied() {
    let dir = tempfile::tempdir().unwrap();
    let core = open_limited(
        dir.path(),
        r#"
import os,re,urllib.request
if 'Agent question ' in prompt:
    question=re.findall(r'Agent question ([a-f0-9-]+)',prompt)[-1]
    body={'jsonrpc':'2.0','id':1,'method':'tools/call','params':{'name':'answer_agent','arguments':{'turn':os.environ['FIXTURE_TURN'],'request':question,'answer':'Use UTF-8'}}}
    req=urllib.request.Request(os.environ['ANTISLOP_MCP_URL'],json.dumps(body).encode(),{'Authorization':'Bearer '+os.environ['ANTISLOP_MCP_TOKEN'],'Content-Type':'application/json'})
    reply=json.load(urllib.request.urlopen(req,timeout=10))
    assert not reply['result'].get('isError'),reply
    answer='Answered the local question.'
else:
    answer='done' if '[runtime]' in prompt else plan(task('t1','Encoding inspection'))
"#,
        r#"    import os,urllib.request
    def call(name,**arguments):
        arguments['turn']=os.environ['FIXTURE_TURN']
        body={'jsonrpc':'2.0','id':1,'method':'tools/call','params':{'name':name,'arguments':arguments}}
        req=urllib.request.Request(os.environ['ANTISLOP_MCP_URL'],json.dumps(body).encode(),{'Authorization':'Bearer '+os.environ['ANTISLOP_MCP_TOKEN'],'Content-Type':'application/json'})
        reply=json.load(urllib.request.urlopen(req,timeout=30))['result']
        assert not reply.get('isError'),reply
        return json.loads(reply['content'][0]['text'])
    response=call('ask',question='Which encoding should this local report use?')
    if response['state']=='pending':
        response=call('ask',question='Which encoding should this local report use?')
    assert response['state']=='answered',response
    assert response['answer']=='Use UTF-8',response
    answer='Inspected encoding: UTF-8'"#,
        1,
    );
    let conversation = run(&core, dir.path());
    wait_until(
        "worker received live answer and passed independent verification",
        || core.is_idle(&conversation),
    );
    let tasks = core.store.latest_plan(&conversation).unwrap();
    assert_eq!(tasks[0].status, "approved", "{:?}", tasks[0].report);
    assert_eq!(
        tasks[0].report.as_deref(),
        Some("Inspected encoding: UTF-8")
    );
    assert_eq!(
        prompts(dir.path())
            .iter()
            .filter(|(bin, p)| bin == "agy" && p.contains("Task role: worker"))
            .count(),
        1
    );
    let worker = prompts(dir.path())
        .into_iter()
        .find(|(bin, p)| bin == "agy" && p.contains("Task role: worker"))
        .unwrap()
        .1;
    assert!(!worker.contains("Team:"));
    assert!(!worker.contains("The user's request:"));
}

#[test]
fn the_orchestrator_retries_a_failed_task_and_it_finishes() {
    let dir = tempfile::tempdir().unwrap();
    let crashed = dir.path().join("worker-crashed");
    let agent = format!(
        r#"    import os
    if 'Task role: worker' in prompt and not os.path.exists({crashed:?}):
        open({crashed:?},'w').close()
        sys.exit(3)
    answer='inspected after retry'"#,
        crashed = crashed.to_string_lossy()
    );
    let core = open(
        dir.path(),
        r#"
import os,urllib.request
def call(name,**arguments):
    arguments['turn']=os.environ['FIXTURE_TURN']
    body={'jsonrpc':'2.0','id':1,'method':'tools/call','params':{'name':name,'arguments':arguments}}
    req=urllib.request.Request(os.environ['ANTISLOP_MCP_URL'],json.dumps(body).encode(),{'Authorization':'Bearer '+os.environ['ANTISLOP_MCP_TOKEN'],'Content-Type':'application/json'})
    return json.load(urllib.request.urlopen(req,timeout=30))['result']
if '"status":"failed"' in prompt:
    reply=call('retry_task',task='t1',guidance='check the worktree first')
    assert not reply.get('isError'),reply
    again=call('retry_task',task='t1',guidance='again')
    assert again.get('isError'),again
    answer='retried t1'
else:
    answer='done' if '[runtime]' in prompt else plan(task('t1','Inspection'))
"#,
        &agent,
    );
    let conversation = run(&core, dir.path());
    wait_until("the retried task finished", || {
        core.is_idle(&conversation)
            && core.store.latest_plan(&conversation).unwrap()[0].status == "approved"
    });
    let recoveries = events(&core, &conversation, "task_recovery");
    assert_eq!(recoveries.len(), 1, "{recoveries:?}");
    assert_eq!(recoveries[0]["actor"], "orchestrator");
    assert_eq!(recoveries[0]["message"], "check the worktree first");
    let retried = prompts(dir.path())
        .into_iter()
        .rfind(|(_, p)| p.contains("Task role: worker"))
        .unwrap()
        .1;
    assert!(retried.contains("check the worktree first"), "{retried}");
}

#[test]
fn a_dependency_correction_cancels_the_old_consumer_and_releases_its_only_slot() {
    struct ReleaseOnDrop(std::path::PathBuf);
    impl Drop for ReleaseOnDrop {
        fn drop(&mut self) {
            let _ = std::fs::write(&self.0, "release");
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let entered = dir.path().join("old-consumer-entered");
    let release = dir.path().join("release-old-consumer");
    let _release_on_drop = ReleaseOnDrop(release.clone());
    let agent = format!(
        r#"    import os
    if 'Task id: t2\n' in prompt:
        if not os.path.exists({entered:?}):
            open({entered:?},'w').close()
            while not os.path.exists({release:?}): time.sleep(0.01)
        answer='consumer recomputed'
    else:
        answer='source corrected' if 'make correction' in prompt else 'source initial'"#,
        entered = entered.to_string_lossy(),
        release = release.to_string_lossy()
    );
    let core = open_limited(
        dir.path(),
        "answer='done' if '[runtime]' in prompt else plan(task('t1','Source'),task('t2','Consumer',['t1']))",
        &agent,
        1,
    );
    let conversation = run(&core, dir.path());
    wait_until("consumer occupied the only scheduler slot", || {
        entered.exists()
    });
    let before = core.store.latest_plan(&conversation).unwrap();
    core.send_to_agent(&antislop::lean::store::AgentMessage {
        conversation: conversation.clone(),
        plan: Some(before[0].plan_id.clone()),
        recipient: format!("worker:{}", before[0].id),
        operation_id: antislop::id(),
        expected_round: Some(before[0].round),
        expected_run: None,
        text: "make correction".into(),
    })
    .unwrap();
    wait_until(
        "obsolete consumer cancelled and both tasks recomputed",
        || {
            core.is_idle(&conversation)
                && core
                    .store
                    .latest_plan(&conversation)
                    .unwrap()
                    .iter()
                    .zip(&before)
                    .all(|(new, old)| new.status == "approved" && new.round > old.round)
        },
    );
    let after = core.store.latest_plan(&conversation).unwrap();
    assert!(after[1].input_generation > before[1].input_generation);
    assert!(
        !release.exists(),
        "the old worker must be cancelled, not manually released"
    );
    let role = format!("worker:{}", before[1].id);
    assert!(
        core.store
            .sessions(&conversation)
            .unwrap()
            .iter()
            .any(|s| s.role == role && s.status == "cancelled")
    );
}

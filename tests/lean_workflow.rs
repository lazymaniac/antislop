mod common;
mod support;

use antislop::{config::Config, lean::Core};
use std::time::{Duration, Instant};

fn idle(core: &Core, conversation: &str) {
    let deadline = Instant::now() + Duration::from_secs(120);
    while !core.is_idle(conversation) {
        assert!(Instant::now() < deadline, "session did not become idle");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Fake agents for a full guided session: interview, plan, review, work.
fn guided_session_source(dir: &std::path::Path) -> String {
    for name in ["codex", "agy"] {
        let script = dir.join(name);
        std::fs::write(&script, r#"#!/usr/bin/env python3
import json, sys, os, re
prompt = sys.argv[-1]
plan = '```plan\n' + json.dumps({'tasks':[{'id':'t1','title':'Inspect','goal':'Inspect the project','kind':'analysis','change_kind':'none','reviewer':{'summary':'Check the observations'}}]}) + '\n```'
if 'SESSION PHASE: INTERVIEW.' in prompt:
    # A premature plan from an agent must never bypass the interview gate.
    users = re.findall(r'\[user\]\n([^\n]+)', prompt)
    if users and users[-1].startswith('Interview answer ('):
        answer = '```requirements\nGoal: inspect the project.\nScope: read only.\nAcceptance: report observations.\n```\n' + plan
    else:
        answer = '```question\n' + json.dumps({'id':'scope','context':'The scope determines what we inspect.','question':'What should we inspect?','options':[{'label':'Source code','description':'Inspect code quality first'},{'label':'Documentation','description':'Inspect documentation first'}]}) + '\n```\n' + plan
elif any(marker in prompt for marker in ['You review a plan', 'new plan:', 'You review', 'You audit', 'Your reviewer found']):
    ids = re.findall(r'^- ([A-Za-z0-9_.-]+): ', prompt, re.M)
    answer = '```verdict\n' + json.dumps({'approved':True,'findings':[], 'coverage':{i:{'status':'ok','note':'checked'} for i in ids}}) + '\n```'
elif 'Task: ' in prompt:
    answer = 'Project inspected.'
elif 'The plan finished. Results of each task follow.' in prompt and 'SESSION PHASE: planning' not in prompt:
    answer = 'Implementation results received.'
else:
    answer = '## Inspection\nReview the inspection task.\n' + plan
if 'Address each new user comment' in prompt:
    ids = sorted(set(int(i) for i in re.findall(r'\[user comment #(\d+)', prompt)))
    answer += '\n```comments\n' + json.dumps({'addressed':[{'id':i,'how':'Clarified inspection scope'} for i in ids]}) + '\n```'
if os.path.basename(sys.argv[0]) == 'codex':
    print(json.dumps({'type':'thread.started','thread_id':'workflow'}))
    print(json.dumps({'type':'item.completed','item':{'type':'agent_message','text':answer}}))
    print(json.dumps({'type':'turn.completed'}))
else:
    print(json.dumps({'event':'init','conversation_id':'workflow-review'}))
    print(json.dumps({'event':'result','result':{'conversation_id':'workflow-review','status':'SUCCESS','response':answer,'usage':{}}}))
"#).unwrap();
        support::install_ledger_protocol(&script);
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        common::interactive(&script);
    }
    include_str!("../examples/antislop.toml")
        .replace(
            "binary = \"codex\"",
            &format!("binary = {:?}", dir.join("codex")),
        )
        .replace(
            "binary = \"agy\"",
            &format!("binary = {:?}", dir.join("agy")),
        )
}

fn finish_fixture_plan(core: &std::sync::Arc<Core>, conversation: &str) {
    // This fixture checks workflow execution. Its final audit deliberately
    // remains negative because it supplies no integrated acceptance evidence.
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let tasks = core.store.latest_plan(conversation).unwrap();
        if !tasks.is_empty() && tasks.iter().all(|task| task.status == "approved") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "checked fixture plan did not finish: {:?}",
            tasks
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    core.cancel(conversation);
    idle(core, conversation);
}

#[test]
fn restart_finishes_current_interview_and_runs_reviewed_plan_without_user_approval() {
    let dir = tempfile::tempdir().unwrap();
    let source = guided_session_source(dir.path());
    let core = Core::open(
        &dir.path().join("state"),
        Config::from_toml(&source).unwrap(),
    )
    .unwrap();
    let waiting = core.start(dir.path()).unwrap();
    let message = core
        .store
        .add_message(&waiting.id, "user", "Inspect the project")
        .unwrap();
    core.store
        .interview_summary(
            &waiting.id,
            "Goal: inspect the project. Acceptance: report observations.",
            message.seq,
        )
        .unwrap();
    drop(core);
    let core = Core::open(
        &dir.path().join("state"),
        Config::from_toml(&source).unwrap(),
    )
    .unwrap();
    finish_fixture_plan(&core, &waiting.id);
    assert_eq!(
        core.store.workflow(&waiting.id).unwrap().phase,
        "implementation"
    );
    assert_eq!(
        core.store
            .messages_by(&waiting.id, "user", 0)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn checked_plan_starts_automatically_after_the_interview() {
    let dir = tempfile::tempdir().unwrap();
    let source = guided_session_source(dir.path());
    let core = Core::open(
        &dir.path().join("state"),
        Config::from_toml(&source).unwrap(),
    )
    .unwrap();
    let conversation = core.start(dir.path()).unwrap();
    core.send(&conversation.id, "Inspect the project").unwrap();
    idle(&core, &conversation.id);
    let event = core
        .store
        .events_since(&conversation.id, 0, 100)
        .unwrap()
        .into_iter()
        .find(|e| e.kind == "interview_question")
        .unwrap();
    let answer = format!(
        "Interview answer ({})\nAnswer: Source code",
        event.payload["id"].as_str().unwrap()
    );
    core.send(&conversation.id, &answer).unwrap();
    finish_fixture_plan(&core, &conversation.id);
    // Planning, the plan check and acceptance all happened without the user.
    let planner = core
        .store
        .last_turn(&conversation.id, antislop::lean::PLANNER)
        .unwrap()
        .expect("the planner drafted the plan");
    assert_eq!(planner.status, antislop::lean::store::TurnStatus::Done);
    let proposed = core
        .store
        .events_since(&conversation.id, 0, 1000)
        .unwrap()
        .into_iter()
        .find(|e| e.kind == "plan_proposed")
        .expect("a checked plan was proposed");
    assert_eq!(proposed.payload["automatic"], true);
    assert_eq!(
        core.store.workflow(&conversation.id).unwrap().phase,
        "implementation"
    );
    assert!(
        core.store
            .sessions(&conversation.id)
            .unwrap()
            .iter()
            .any(|s| s.role.starts_with("worker:"))
    );
}

#[test]
fn orchestrator_requests_a_user_decision_during_execution_without_reopening_worker_questions() {
    for phase in ["planning", "implementation"] {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("codex");
        std::fs::write(&script, r#"#!/usr/bin/env python3
import json, sys
prompt = sys.argv[-1]
if 'Interview answer (' in prompt:
    answer = 'Decision received; existing gates remain unchanged.'
else:
    answer = '```question\n' + json.dumps({'id':'metadata-scope','context':'Allow read-only access to the current project worktree metadata. No writes or historical evidence reconstruction.','question':'Approve this metadata scope?','options':[{'label':'Approve'},{'label':'Reject'}]}) + '\n```'
print(json.dumps({'type':'thread.started','thread_id':'execution-decision'}))
print(json.dumps({'type':'item.completed','item':{'type':'agent_message','text':answer}}))
print(json.dumps({'type':'turn.completed'}))
"#).unwrap();
        common::interactive(&script);
        let source = include_str!("../examples/antislop.toml")
            .replace("binary = \"codex\"", &format!("binary = {:?}", script));
        let core = Core::open(
            &dir.path().join("state"),
            Config::from_toml(&source).unwrap(),
        )
        .unwrap();
        let conversation = support::fixture_conversation(&core, dir.path()).unwrap();
        let mut task = core.store.add_plan(&conversation.id, &[serde_json::json!({"id":"context","title":"Context","goal":"Inspect source","change_kind": "none", "kind":"analysis","reviewer":{"summary": "Inspect evidence"}})]).unwrap().remove(0);
        if phase == "implementation" {
            core.store
                .queue_plan(&conversation.id, &task.plan_id)
                .unwrap();
        }
        task.status = "coordinating".into();
        core.store.update_task(&task).unwrap();
        core.send(&conversation.id, "Present the scope decision")
            .unwrap();
        idle(&core, &conversation.id);
        let events = core.store.events_since(&conversation.id, 0, 1000).unwrap();
        let question = events
            .iter()
            .find(|event| event.kind == "user_decision_requested")
            .expect("execution question is delivered");
        // The runtime names the question; the agent gives only its content.
        assert_eq!(question.payload["question"], "Approve this metadata scope?");
        let id = question.payload["id"].as_str().unwrap().to_owned();
        assert!(!id.is_empty());
        assert!(!events.iter().any(|event| event.kind == "planning_progress"
            && event.payload["text"] == "Plan draft requires correction"));
        assert_eq!(core.store.workflow(&conversation.id).unwrap().phase, phase);
        assert_eq!(
            core.store.task_by_id(&task.id).unwrap().unwrap().status,
            "coordinating"
        );
        core.send(
            &conversation.id,
            &format!(
                "Interview answer ({id})\nQuestion: Approve this metadata scope?\nAnswer: Reject"
            ),
        )
        .unwrap();
        idle(&core, &conversation.id);
        assert_eq!(
            core.store.task_by_id(&task.id).unwrap().unwrap().status,
            "coordinating"
        );
        assert!(
            !core
                .store
                .events_since(&conversation.id, 0, 1000)
                .unwrap()
                .iter()
                .any(|event| event.kind == "agent_tool_rejected")
        );
    }
}

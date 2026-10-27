mod common;
mod support;

use antislop::{config::Config, lean::Core};
use std::time::{Duration, Instant};

fn idle(core: &Core, conversation: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
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
plan = '```plan\n' + json.dumps({'tasks':[{'id':'t1','title':'Inspect','goal':'Inspect the project','kind':'analysis','scope':['.'],'verifier':{'focus':'Check the observations','risks':[]}}]}) + '\n```'
if 'SESSION PHASE: INTERVIEW.' in prompt:
    # A premature plan from an agent must never bypass the interview gate.
    if 'Interview answer (' in prompt:
        answer = '```requirements\nGoal: inspect the project.\nScope: read only.\nAcceptance: report observations.\n```\n' + plan
    else:
        answer = '```question\n' + json.dumps({'id':'scope','context':'The scope determines what we inspect.','question':'What should we inspect?','options':[{'label':'Source code','description':'Inspect code quality first'},{'label':'Documentation','description':'Inspect documentation first'}]}) + '\n```\n' + plan
elif any(marker in prompt for marker in ['You review a plan', 'new plan:', 'You verify', 'Your verifier found']):
    ids = re.findall(r'^- ([A-Za-z0-9_.-]+): ', prompt, re.M)
    answer = '```verdict\n' + json.dumps({'approved':True,'findings':[], 'coverage':{i:{'status':'ok','note':'checked'} for i in ids}}) + '\n```'
elif 'Task: ' in prompt:
    answer = 'Project inspected.'
elif '[runtime]' in prompt and 'SESSION PHASE: planning' not in prompt:
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

#[test]
fn approving_requirements_starts_planning_and_implementation_requires_plan_approval() {
    let dir = tempfile::tempdir().unwrap();
    let source = guided_session_source(dir.path());
    let core = Core::open(
        &dir.path().join("state"),
        Config::from_toml(&source).unwrap(),
    )
    .unwrap();
    let conversation = core.start(dir.path()).unwrap();
    assert!(core.send(&conversation.id, "/planning").is_err());
    core.send(&conversation.id, "Inspect the project").unwrap();
    idle(&core, &conversation.id);
    assert!(core.store.latest_plan(&conversation.id).unwrap().is_empty());
    let workflow = core.store.workflow(&conversation.id).unwrap();
    assert_eq!(workflow.phase, "interview");
    assert!(workflow.summary.is_empty());
    assert!(core.send(&conversation.id, "/planning").is_err());
    let event = core
        .store
        .events_since(&conversation.id, 0, 100)
        .unwrap()
        .into_iter()
        .find(|e| e.kind == "interview_question")
        .unwrap();
    let question = serde_json::from_value(event.payload).unwrap();
    let mut form = antislop::lean::tui::interview::Form::new(event.seq, question);
    let answer = form
        .key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        ))
        .unwrap();
    assert!(answer.contains("Answer: Source code"));
    core.send(&conversation.id, &answer).unwrap();
    idle(&core, &conversation.id);
    let workflow = core.store.workflow(&conversation.id).unwrap();
    assert!(workflow.summary.contains("Acceptance:"));
    assert!(core.store.latest_plan(&conversation.id).unwrap().is_empty());

    // Approving the requirements schedules the plan; it still waits for acceptance.
    core.send(&conversation.id, "/planning").unwrap();
    idle(&core, &conversation.id);
    let tasks = core.store.latest_plan(&conversation.id).unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].status, "proposed");
    // The planner, not the orchestrator, drafted it; the user still hears one voice.
    let planner = core
        .store
        .last_turn(&conversation.id, antislop::lean::PLANNER)
        .unwrap()
        .expect("the planner drafted the first plan");
    assert_eq!(planner.status, antislop::lean::store::TurnStatus::Done);
    assert!(
        core.store
            .messages_by(&conversation.id, "planner", 0)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        core.store.workflow(&conversation.id).unwrap().phase,
        "planning"
    );
    assert!(
        core.store
            .last_turn(&conversation.id, "worker:t1")
            .unwrap()
            .is_none()
    );

    let initial_plan = tasks[0].plan_id.clone();
    let comment = core
        .comment_scoped(
            &conversation.id,
            Some(&initial_plan),
            &format!("plan:{initial_plan}:line:1"),
            "Clarify inspection scope",
        )
        .unwrap();
    assert_eq!(comment.status, "draft");
    assert!(core.accept_plan(&conversation.id, &initial_plan).is_err());
    assert_eq!(
        core.review_comments(&conversation.id, &initial_plan)
            .unwrap(),
        1
    );
    idle(&core, &conversation.id);
    assert!(
        core.store
            .comments(&conversation.id, "resolved")
            .unwrap()
            .iter()
            .any(|c| c.id == comment.id)
    );
    let revised = core.store.latest_plan(&conversation.id).unwrap();
    assert_ne!(revised[0].plan_id, initial_plan);
    assert_eq!(revised[0].status, "proposed");
    core.accept_plan(&conversation.id, &revised[0].plan_id)
        .unwrap();
    assert_eq!(
        core.store.workflow(&conversation.id).unwrap().phase,
        "implementation"
    );
    idle(&core, &conversation.id);
    assert!(
        core.store
            .sessions(&conversation.id)
            .unwrap()
            .iter()
            .any(|s| s.role.starts_with("worker:"))
    );
    // A completed interview left by a stopped daemon resumes without /planning.
    // Autonomous conversations then carry on to the checked plan's acceptance;
    // supervised ones wait for explicit requirements approval.
    let waiting = core.start(dir.path()).unwrap();
    core.set_mode(&waiting.id, true).unwrap();
    let message = core
        .store
        .add_message(&waiting.id, "user", "Inspect the project")
        .unwrap();
    core.store
        .interview_summary(
            &waiting.id,
            "Goal: inspect the project. Scope: read only. Acceptance: report observations.",
            message.seq,
        )
        .unwrap();
    drop(core);
    let core = Core::open(
        &dir.path().join("state"),
        Config::from_toml(&source).unwrap(),
    )
    .unwrap();
    idle(&core, &waiting.id);
    assert!(!core.store.latest_plan(&waiting.id).unwrap().is_empty());
    assert_eq!(
        core.store.workflow(&waiting.id).unwrap().phase,
        "implementation"
    );
}

#[test]
fn autonomous_mode_accepts_the_checked_plan_after_the_interview() {
    let dir = tempfile::tempdir().unwrap();
    let source = guided_session_source(dir.path());
    let core = Core::open(
        &dir.path().join("state"),
        Config::from_toml(&source).unwrap(),
    )
    .unwrap();
    let conversation = core.start(dir.path()).unwrap();
    core.set_mode(&conversation.id, true).unwrap();
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
    idle(&core, &conversation.id);
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

    // The finished plan ends the cycle. The next request interviews again and
    // the planner drafts its plan too, though an older plan exists.
    core.send(&conversation.id, "Inspect the documentation too")
        .unwrap();
    idle(&core, &conversation.id);
    assert!(
        core.store
            .workflow(&conversation.id)
            .unwrap()
            .interviewing()
    );
    let question = core
        .store
        .events_since(&conversation.id, 0, 2000)
        .unwrap()
        .into_iter()
        .rev()
        .find(|e| e.kind == "interview_question")
        .unwrap();
    let answer = format!(
        "Interview answer ({})\nAnswer: Documentation",
        question.payload["id"].as_str().unwrap()
    );
    core.send(&conversation.id, &answer).unwrap();
    idle(&core, &conversation.id);
    let planner_turns = core
        .store
        .sessions(&conversation.id)
        .unwrap()
        .iter()
        .filter(|s| s.role == antislop::lean::PLANNER)
        .count();
    assert_eq!(planner_turns, 2, "one draft per planning cycle");
}

#[test]
fn orchestrator_requests_a_user_decision_during_execution_without_reopening_worker_questions() {
    for phase in ["planning", "implementation"] {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("codex");
        std::fs::write(&script, r#"#!/usr/bin/env python3
import json, sys
prompt = sys.argv[-1]
if 'Interview answer (metadata-scope)' in prompt:
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
        let mut task = core.store.add_plan(&conversation.id, &[serde_json::json!({"id":"context","title":"Context","goal":"Inspect source","kind":"analysis","verifier":{"focus":"Inspect evidence"}})]).unwrap().remove(0);
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
        assert_eq!(question.payload["id"], "metadata-scope");
        assert!(!events.iter().any(|event| event.kind == "planning_progress"
            && event.payload["text"] == "Plan draft requires correction"));
        assert_eq!(core.store.workflow(&conversation.id).unwrap().phase, phase);
        assert_eq!(
            core.store.task_by_id(&task.id).unwrap().unwrap().status,
            "coordinating"
        );
        core.send(
        &conversation.id,
        "Interview answer (metadata-scope)\nQuestion: Approve this metadata scope?\nAnswer: Reject",
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

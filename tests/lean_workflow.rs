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

#[test]
fn interview_starts_planning_automatically_but_implementation_requires_approval() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["codex", "agy"] {
        let script = dir.path().join(name);
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
    ids = re.findall(r'^- ([RCV][0-9]+): ', prompt, re.M)
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
    }
    let source = include_str!("../examples/antislop.toml")
        .replace(
            "binary = \"codex\"",
            &format!("binary = {:?}", dir.path().join("codex")),
        )
        .replace(
            "binary = \"agy\"",
            &format!("binary = {:?}", dir.path().join("agy")),
        );
    let core = Core::open(
        &dir.path().join("state"),
        Config::from_toml(&source).unwrap(),
    )
    .unwrap();
    let conversation = core.start(dir.path()).unwrap();
    core.set_mode(&conversation.id, true).unwrap();
    assert!(core.send(&conversation.id, "/planning").is_err());
    core.send(&conversation.id, "Inspect the project").unwrap();
    idle(&core, &conversation.id);
    assert!(core.store.latest_plan(&conversation.id).unwrap().is_empty());
    let workflow = core.store.workflow(&conversation.id).unwrap().unwrap();
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
    let workflow = core.store.workflow(&conversation.id).unwrap().unwrap();
    assert!(workflow.summary.contains("Acceptance:"));

    // No /planning command: completing the interview schedules the plan itself.
    let tasks = core.store.latest_plan(&conversation.id).unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].status, "proposed");
    assert_eq!(
        core.store
            .workflow(&conversation.id)
            .unwrap()
            .unwrap()
            .phase,
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
        core.store
            .workflow(&conversation.id)
            .unwrap()
            .unwrap()
            .phase,
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
    // A completed interview left by an older daemon must resume without /planning.
    let waiting = core.start(dir.path()).unwrap();
    // Match the first conversation's autonomous interview policy. Supervised
    // conversations correctly wait for explicit requirements approval.
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
    assert_eq!(
        core.store.workflow(&waiting.id).unwrap().unwrap().phase,
        "planning"
    );
    assert!(!core.store.latest_plan(&waiting.id).unwrap().is_empty());
    assert!(
        core.store
            .last_turn(&waiting.id, "worker:t1")
            .unwrap()
            .is_none()
    );
}

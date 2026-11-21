mod common;
mod support;

use antislop::{
    config::Config,
    lean::{
        Core,
        daemon::{Client, ControlRequest, HistoryRequest, Request},
        tui::{App, Focus, PlanAction, Screen, Update, UpdateReceiver, planview::BottomButton},
    },
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::{Child, Command as ProcessCommand, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const SOURCE: &str = include_str!("../examples/antislop.toml");

struct Daemon {
    child: Child,
    client: Client,
}

impl Daemon {
    fn start(project: &Path, config: &Path, state: &Path, registry: &Path) -> Self {
        let mut child = ProcessCommand::new(env!("CARGO_BIN_EXE_antislop"))
            .args(["__core"])
            .arg(project)
            .arg(config)
            .arg(state)
            .env("ANTISLOP_CONFIG_DIR", registry)
            .env("ANTISLOP_STATE_ROOT", state.join("runtime"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let client = Client::at(state);
        let deadline = Instant::now() + Duration::from_secs(120);
        while Instant::now() < deadline {
            if client
                .call(&Request::Control(ControlRequest::Identity))
                .is_ok()
            {
                return Self { child, client };
            }
            if let Some(status) = child.try_wait().unwrap() {
                panic!("fixture daemon exited during startup: {status}");
            }
            thread::sleep(Duration::from_millis(25));
        }
        let _ = child.kill();
        let _ = child.wait();
        panic!("fixture daemon did not become ready within 120 seconds");
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self
            .client
            .call(&Request::Control(ControlRequest::ForceShutdown));
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct RunDir(PathBuf);

impl RunDir {
    fn new() -> Self {
        let root = PathBuf::from(
            std::env::var_os("ANTISLOP_TASK_STATE")
                .expect("set ANTISLOP_TASK_STATE to the allocated durable task-state directory"),
        );
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = root.join(format!(
            "runs/tui-plan-consent-{}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

#[test]
fn plan_review_acceptance_is_tokened_and_keeps_tui_state_correlated() {
    let run = RunDir::new();
    let project = run.path().join("project");
    let state = run.path().join("state");
    let registry = run.path().join("config-registry");
    std::fs::create_dir_all(&project).unwrap();
    git(&project, &["init", "--quiet", "--initial-branch=main"]);
    git(
        &project,
        &[
            "-c",
            "user.name=Antislop Test",
            "-c",
            "user.email=antislop-test@example.invalid",
            "commit",
            "--quiet",
            "--allow-empty",
            "-m",
            "initialize test project",
        ],
    );
    let codex = fake_cli(run.path(), "codex");
    let agy = fake_cli(run.path(), "agy");
    let config_text = SOURCE
        .replace(
            "binary = \"codex\"",
            &format!("binary = {:?}", codex.to_string_lossy()),
        )
        .replace(
            "binary = \"agy\"",
            &format!("binary = {:?}", agy.to_string_lossy()),
        );
    let config = run.path().join("antislop.toml");
    std::fs::write(&config, config_text).unwrap();
    let core = Core::open(
        &state,
        Config::from_toml(&std::fs::read_to_string(&config).unwrap()).unwrap(),
    )
    .unwrap();
    let conversation = support::fixture_conversation(&core, &project).unwrap();
    let rejection_conversation = support::fixture_conversation(&core, &project).unwrap();
    let conversation_id = conversation.id.clone();
    let rejection_id = rejection_conversation.id.clone();
    drop(core);

    let daemon = Daemon::start(&project, &config, &state, &registry);
    let (mut app, updates) = App::test_app_with_client(daemon.client.clone());
    app.project = project.to_string_lossy().into_owned();
    app.conversation = conversation_id.clone();
    app.focus = Focus::Chat;
    app.navigation.screen = Screen::Graph;

    let mut dialog_path = codex.as_os_str().to_owned();
    dialog_path.push(".dialog.turn");
    let mut key_path = codex.as_os_str().to_owned();
    key_path.push(".keys");
    std::fs::write(
        &dialog_path,
        include_str!("fixtures/approvals/codex/command.txt"),
    )
    .unwrap();

    app.input = antislop::lean::tui::InputBuffer::with_text("Review storage and the TUI");
    app.key(key(KeyCode::Enter));
    wait_until("fake CLI approval was not requested", || {
        events(&daemon.client, &conversation_id)
            .iter()
            .any(|event| event["kind"] == "approval_requested")
    });
    pump_until_plan(&mut app, &updates, &conversation_id, |app| {
        !app.view.approvals.is_empty()
    });
    let approval_request = events(&daemon.client, &conversation_id)
        .into_iter()
        .find(|event| event["kind"] == "approval_requested")
        .unwrap()["payload"]["request"]
        .as_str()
        .unwrap()
        .to_owned();
    app.key(key(KeyCode::F(2)));
    let inbox = draw(&mut app);
    assert!(inbox.contains("Yes, proceed [1]"), "{inbox}");
    assert!(inbox.contains("don't ask again"), "{inbox}");
    app.key(key(KeyCode::Char('d')));
    assert!(
        !PathBuf::from(&key_path).exists(),
        "deferring must leave the CLI dialog unanswered"
    );
    app.key(key(KeyCode::F(2)));
    let reopened = draw(&mut app);
    assert!(reopened.contains("CLI approval"), "{reopened}");
    app.key(key(KeyCode::Enter));
    wait_until("TUI approval answer did not resolve", || {
        events(&daemon.client, &conversation_id)
            .iter()
            .any(|event| {
                event["kind"] == "approval_resolved"
                    && event["payload"]["request"] == approval_request
            })
    });
    wait_until(
        "the fake CLI did not receive its original approval key",
        || std::fs::read(&key_path).is_ok_and(|keys| keys == b"y"),
    );
    std::fs::remove_file(&dialog_path).unwrap();
    pump_until_plan(&mut app, &updates, &conversation_id, |app| {
        app.view.plan_proposed() && app.view.plan_revision_token.is_some()
    });
    let original_plan = app.view.plan.clone().unwrap();
    let original_token = app.view.plan_revision_token.clone().unwrap();
    assert_eq!(latest_statuses(&state, &conversation_id), vec!["proposed"]);
    assert!(
        events(&daemon.client, &conversation_id)
            .iter()
            .any(|event| event["kind"] == "plan_proposed"
                && event["payload"]["review"]["approved"] == true),
        "only an independently reviewed proposal is eligible for this flow"
    );

    app.key(key(KeyCode::Esc));
    app.key(key_ctrl('l'));
    assert!(app.plan_view.is_some());
    draw(&mut app);
    let action = app
        .plan_view
        .as_mut()
        .unwrap()
        .key(key(KeyCode::Char('C')), &app.view);
    assert_eq!(action, PlanAction::CommentWholePlan);
    app.handle_plan_action(action);
    assert_eq!(
        app.editor.as_ref().unwrap().target,
        format!("plan:{original_plan}")
    );
    app.editor.as_mut().unwrap().input =
        antislop::lean::tui::InputBuffer::with_text("Clarify the acceptance evidence");
    assert_eq!(
        app.editor.as_ref().unwrap().input.text,
        "Clarify the acceptance evidence"
    );
    assert!(draw(&mut app).contains("Clarify the acceptance evidence"));

    // Keep an unsent comment draft across a conversation switch and restore it.
    let comment_draft = app.editor.take().unwrap();
    app.save_comment_draft(&comment_draft.target, comment_draft.input);
    assert!(
        app.comment_drafts
            .values()
            .any(|text| text == "Clarify the acceptance evidence")
    );
    app.open("other-session".into());
    assert_eq!(app.input.text, "");
    app.open(conversation_id.clone());
    assert_eq!(app.conversation, conversation_id);
    pump_until_plan(&mut app, &updates, &conversation_id, |app| {
        app.view.plan.as_deref() == Some(original_plan.as_str())
    });
    app.key(key_ctrl('l'));
    let action = app
        .plan_view
        .as_mut()
        .unwrap()
        .key(key(KeyCode::Char('C')), &app.view);
    app.handle_plan_action(action);
    assert_eq!(
        app.editor.as_ref().unwrap().input.text,
        "Clarify the acceptance evidence"
    );
    app.submit_comment();
    pump_until_plan(&mut app, &updates, &conversation_id, |app| {
        !app.sending_comment && !app.view.draft_plan_comments().is_empty()
    });
    assert_eq!(latest_statuses(&state, &conversation_id), vec!["proposed"]);

    // The plan view must hold the draft until Review is submitted. Acceptance
    // through its keyboard action is blocked locally while the comment is open.
    app.key(key(KeyCode::Char('a')));
    assert!(app.view.plan_proposed());
    assert!(
        latest_statuses(&state, &conversation_id)
            .iter()
            .all(|s| s == "proposed")
    );
    app.key(key(KeyCode::Char('R')));
    pump_until_plan(&mut app, &updates, &conversation_id, |app| {
        app.view.plan.as_deref() != Some(original_plan.as_str())
            && app.view.plan_proposed()
            && app.view.plan_revision_token.as_deref() != Some(original_token.as_str())
    });
    let revised_plan = app.view.plan.clone().unwrap();
    let revised_token = app.view.plan_revision_token.clone().unwrap();
    assert_ne!(revised_plan, original_plan);
    assert_ne!(revised_token, original_token);
    let comment_events = events(&daemon.client, &conversation_id);
    assert!(comment_events.iter().any(|event| {
        event["kind"] == "comment"
            && event["payload"]["target"] == format!("plan:{original_plan}")
            && event["payload"]["text"] == "Clarify the acceptance evidence"
    }));
    assert!(
        comment_events
            .iter()
            .any(|event| event["kind"] == "plan_proposed"
                && event["payload"]["plan"] == revised_plan
                && event["payload"]["review"]["approved"] == true),
        "the changed version must have its own independent review"
    );

    // The stale-token request is rejected by the real daemon handler and must
    // leave the newer proposal unaccepted.
    let stale = daemon.client.call(&Request::Conversation(
        antislop::lean::daemon::ConversationRequest::Resume {
            conversation: conversation_id.clone(),
            operation_id: Some(uuid::Uuid::new_v4().to_string()),
            plan: revised_plan.clone(),
            revision_token: Some(original_token.clone()),
        },
    ));
    assert!(
        stale
            .as_ref()
            .is_err_and(|error| error.to_string().contains("stale")),
        "{stale:?}"
    );
    assert_eq!(app.view.plan.as_deref(), Some(revised_plan.as_str()));
    assert_eq!(
        app.view.plan_revision_token.as_deref(),
        Some(revised_token.as_str())
    );
    assert_eq!(latest_statuses(&state, &conversation_id), vec!["proposed"]);

    // Keyboard focus in the plan action bar is observable, and the valid key
    // goes through App::key -> daemon Resume with the latest token.
    for _ in 0..4 {
        if app.plan_view.as_ref().unwrap().button == BottomButton::Accept {
            break;
        }
        let plan_view = app.plan_view.as_mut().unwrap();
        plan_view.key(key(KeyCode::Tab), &app.view);
    }
    assert_eq!(app.plan_view.as_ref().unwrap().button, BottomButton::Accept);
    app.key(key(KeyCode::Enter));
    pump_until_plan(&mut app, &updates, &conversation_id, |app| {
        !app.view.plan_proposed() || app.notice.as_deref() == Some("plan accepted")
    });
    wait_until("plan was not accepted by the daemon", || {
        latest_statuses(&state, &conversation_id)
            .iter()
            .any(|status| status != "proposed")
    });
    assert!(
        events(&daemon.client, &conversation_id)
            .iter()
            .any(|event| event["kind"] == "workflow"
                && event["payload"]["phase"] == "implementation"),
        "successful TUI feedback must match a persisted Core activation"
    );
    wait_until("accepted plan did not settle", || {
        daemon_idle(&daemon.client, &conversation_id)
    });

    // Start a real TUI comment request, retain its authentic daemon response,
    // then deliver it only after opening a different real conversation.
    app.editor = Some(antislop::lean::tui::Editor {
        target: format!("plan:{revised_plan}"),
        input: antislop::lean::tui::InputBuffer::with_text(
            "Late response must stay with the originating conversation",
        ),
    });
    app.submit_comment();
    let late_response = receive_comment_result(
        &mut app,
        &updates,
        &conversation_id,
        "Late response must stay with the originating conversation",
    );
    let generation_before_switch = app.generation;
    app.open(rejection_id.clone());
    assert_eq!(app.conversation, rejection_id);
    assert!(app.generation > generation_before_switch);
    app.editor = Some(antislop::lean::tui::Editor {
        target: "session".into(),
        input: antislop::lean::tui::InputBuffer::with_text(
            "Current conversation note remains pending",
        ),
    });
    app.submit_comment();
    assert!(app.sending_comment);
    assert_eq!(
        app.pending_comment.as_ref().unwrap().input.text,
        "Current conversation note remains pending"
    );
    let active_notice = Some("active conversation remains untouched".to_owned());
    app.notice = active_notice.clone();
    app.apply_update(late_response);
    assert_eq!(app.conversation, rejection_id);
    assert_eq!(app.notice, active_notice);
    assert!(app.sending_comment);
    assert_eq!(
        app.pending_comment.as_ref().unwrap().input.text,
        "Current conversation note remains pending"
    );
    let active_response = receive_comment_result(
        &mut app,
        &updates,
        &rejection_id,
        "Current conversation note remains pending",
    );
    app.apply_update(active_response);
    assert!(!app.sending_comment);

    let (mut rejected, rejected_updates) = App::test_app_with_client(daemon.client.clone());
    rejected.project = project.to_string_lossy().into_owned();
    rejected.conversation = rejection_id.clone();
    rejected.focus = Focus::Chat;
    rejected.navigation.screen = Screen::Graph;
    pump_until_plan(&mut rejected, &rejected_updates, &rejection_id, |app| {
        !app.view.approvals.is_empty()
    });
    rejected.key(key(KeyCode::F(2)));
    let second_inbox = draw(&mut rejected);
    assert!(second_inbox.contains("Yes, proceed [1]"), "{second_inbox}");
    rejected.key(key(KeyCode::Enter));
    wait_until("second CLI approval answer did not resolve", || {
        events(&daemon.client, &rejection_id)
            .iter()
            .any(|event| event["kind"] == "approval_resolved")
    });
    wait_until("second fake CLI did not receive its approval key", || {
        std::fs::read(&key_path).is_ok_and(|keys| keys == b"yy")
    });
    pump_until_plan(&mut rejected, &rejected_updates, &rejection_id, |app| {
        app.view.plan_proposed()
    });
    let rejected_plan = rejected.view.plan.clone().unwrap();
    assert!(
        rejected.view.plan_proposed(),
        "rejection proposal is no longer proposed: {:?}",
        latest_statuses(&state, &rejection_id)
    );
    rejected.key(key(KeyCode::Esc));
    rejected.key(key_ctrl('l'));
    let action = rejected
        .plan_view
        .as_mut()
        .unwrap()
        .key(key(KeyCode::Char('x')), &rejected.view);
    assert_eq!(action, PlanAction::Reject);
    rejected.handle_plan_action(action);
    assert!(
        rejected
            .editor
            .as_ref()
            .unwrap()
            .target
            .starts_with("reject:")
    );
    rejected.editor.as_mut().unwrap().input =
        antislop::lean::tui::InputBuffer::with_text("The proposal omits required evidence");
    rejected.submit_comment();
    wait_until("TUI rejection did not reach the daemon", || {
        events(&daemon.client, &rejection_id)
            .iter()
            .any(|event| event["kind"] == "plan_rejected")
    });
    let rejected_event = events(&daemon.client, &rejection_id)
        .into_iter()
        .find(|event| event["kind"] == "plan_rejected")
        .unwrap();
    assert_eq!(rejected_event["payload"]["plan"], rejected_plan);
    assert_eq!(
        rejected_event["payload"]["reason"],
        "The proposal omits required evidence"
    );
    assert!(events(&daemon.client, &rejection_id).iter().all(|event| {
        event["kind"] != "workflow" || event["payload"]["phase"] != "implementation"
    }));

    assert!(run.path().starts_with(PathBuf::from(
        std::env::var_os("ANTISLOP_TASK_STATE").unwrap()
    )));
}

fn fake_cli(root: &Path, name: &str) -> PathBuf {
    let path = root.join(name);
    let is_codex = name == "codex";
    let script = if is_codex {
        r####"#!/usr/bin/env python3
import json, sys
prompt = sys.argv[-1]
if "Address each new user comment" in prompt:
    answer = "revision checked\n```plan\n{\"tasks\":[{\"id\":\"storage\",\"title\":\"Storage\",\"goal\":\"Assess storage\",\"change_kind\":\"none\",\"kind\":\"analysis\",\"acceptance_criteria\":[{\"id\":\"C1\",\"text\":\"Evidence is recorded\"}]}]}\n```"
else:
    answer = "draft\n```plan\n{\"tasks\":[{\"id\":\"storage\",\"title\":\"Storage\",\"goal\":\"Assess storage\",\"change_kind\":\"none\",\"kind\":\"analysis\",\"acceptance_criteria\":[{\"id\":\"C1\",\"text\":\"Evidence is recorded\"}]}]}\n```"
print(json.dumps({"type":"thread.started","thread_id":"codex-orchestrator"}))
print(json.dumps({"type":"item.completed","item":{"type":"agent_message","text":answer}}))
print(json.dumps({"type":"turn.completed","usage":{}}))
"####
    } else {
        r####"#!/usr/bin/env python3
import json, sys
import re
prompt = sys.argv[-1]
def cover(answer):
    ids = re.findall(r"^- ([A-Za-z0-9_.-]+): ", prompt, re.M)
    if not ids or "```verdict" not in answer:
        return answer
    head, body = answer.rsplit("```verdict", 1)
    data = json.loads(body.split("```")[0])
    data["coverage"] = {task: {"status":"ok", "note":"checked"} for task in ids}
    return head + "```verdict\n" + json.dumps(data) + "\n```"
if "You review a plan" in prompt or "new plan:" in prompt or "next plan:" in prompt:
    answer = "```verdict\n{\"approved\":true,\"findings\":[]}\n```"
else:
    answer = "Assessment complete.\n```verdict\n{\"approved\":true,\"findings\":[]}\n```"
answer = cover(answer)
print(json.dumps({"event":"init","conversation_id":"agy-reviewer"}))
print(json.dumps({"event":"result","result":{"conversation_id":"agy-reviewer","status":"SUCCESS","response":answer,"usage":{}}}))
"####
    };
    std::fs::write(&path, script).unwrap();
    support::install_task_protocol(&path);
    common::interactive(&path);
    path.canonicalize().unwrap()
}

fn pump_until_plan(
    app: &mut App,
    updates: &UpdateReceiver,
    conversation: &str,
    mut done: impl FnMut(&App) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(120);
    while !done(app) {
        assert!(
            Instant::now() < deadline,
            "TUI plan state did not arrive; notice={:?}",
            app.notice
        );
        match updates.recv_timeout(Duration::from_millis(100)) {
            Ok(update) => app.apply_update(update),
            Err(_) => fetch_events(app, conversation),
        }
    }
}

fn receive_comment_result(
    app: &mut App,
    updates: &UpdateReceiver,
    conversation: &str,
    expected_text: &str,
) -> Update {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        assert!(
            Instant::now() < deadline,
            "TUI comment response did not arrive"
        );
        match updates.recv_timeout(Duration::from_millis(100)) {
            Ok(update) => match &update {
                Update::CommentResult {
                    conversation: response_conversation,
                    editor,
                    result,
                    ..
                } if response_conversation == conversation => {
                    assert!(result.is_ok(), "daemon rejected TUI comment: {result:?}");
                    assert_eq!(editor.input.text, expected_text);
                    assert!(events(&app.client, conversation).iter().any(|event| {
                        event["kind"] == "comment" && event["payload"]["text"] == expected_text
                    }));
                    return update;
                }
                _ => app.apply_update(update),
            },
            Err(_) => fetch_events(app, conversation),
        }
    }
}

fn fetch_events(app: &mut App, conversation: &str) {
    let reply = app
        .client
        .call(&Request::History(HistoryRequest::Events {
            conversation: conversation.into(),
            after: app.view.after,
            wait_ms: 100,
        }))
        .unwrap();
    let events = reply["events"].as_array().cloned().unwrap_or_default();
    app.apply_update(Update::Events {
        conversation: conversation.into(),
        generation: app.generation,
        events,
        idle: reply["idle"].as_bool().unwrap_or(false),
        observer: reply["observer"].as_bool().unwrap_or(false),
    });
}

fn events(client: &Client, conversation: &str) -> Vec<Value> {
    let reply = client
        .call(&Request::History(HistoryRequest::Events {
            conversation: conversation.into(),
            after: 0,
            wait_ms: 0,
        }))
        .unwrap();
    reply["events"].as_array().cloned().unwrap_or_default()
}

fn daemon_idle(client: &Client, conversation: &str) -> bool {
    client
        .call(&Request::History(HistoryRequest::Events {
            conversation: conversation.into(),
            after: 0,
            wait_ms: 0,
        }))
        .is_ok_and(|reply| reply["idle"].as_bool().unwrap_or(false))
}

fn latest_statuses(state: &Path, conversation: &str) -> Vec<String> {
    let store = antislop::lean::store::Store::open(&state.join("core.db")).unwrap();
    store
        .latest_plan(conversation)
        .unwrap()
        .into_iter()
        .map(|task| task.status)
        .collect()
}

fn wait_until(message: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(120);
    while !ready() {
        assert!(Instant::now() < deadline, "{message}");
        thread::sleep(Duration::from_millis(20));
    }
}

fn git(project: &Path, args: &[&str]) {
    let output = ProcessCommand::new("git")
        .args(args)
        .current_dir(project)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn key_ctrl(ch: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)
}

fn draw(app: &mut App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(120, 32)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>()
}

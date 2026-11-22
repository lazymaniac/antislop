mod common;
mod support;

use antislop::{
    config::Config,
    lean::{
        Core,
        daemon::{Client, ControlRequest, Request, TerminalRequest},
        tui::{App, Command, InputBuffer, UpdateReceiver},
    },
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, Stdio},
    thread,
    time::{Duration, Instant},
};
use unicode_width::UnicodeWidthStr;

struct Fixture {
    run: PathBuf,
    child: common::ChildGuard,
    client: Client,
    conversation: String,
    _directory: common::TestDir,
}

impl Fixture {
    fn start(hold_worker: bool) -> Self {
        Self::start_variant(hold_worker, false)
    }

    fn start_with_dag(hold_worker: bool) -> Self {
        Self::start_variant(hold_worker, true)
    }

    fn start_variant(hold_worker: bool, with_dependent: bool) -> Self {
        let directory = common::tempdir();
        let run = directory.path().to_path_buf();
        let project = run.join("project");
        fs::create_dir(&project).unwrap();
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
                "initialize isolated fixture",
            ],
        );
        fs::create_dir_all(run.join("bin")).unwrap();
        let codex = run.join("bin/codex");
        let agy = run.join("bin/agy");
        install_cli(&codex, CODEX);
        install_cli(&agy, AGY);
        if with_dependent {
            fs::write(run.join("bin/dag-plan"), "enabled\n").unwrap();
        }
        support::install_task_protocol(&codex);
        support::install_task_protocol(&agy);
        common::interactive(&codex);
        common::interactive(&agy);
        let config = run.join("antislop.toml");
        let example = fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/examples/antislop.toml"
        ))
        .unwrap();
        let config_text = example
            .replace("binary = \"codex\"", &format!("binary = {:?}", codex))
            .replace("binary = \"agy\"", &format!("binary = {:?}", agy))
            .replace("required_gates = true", "required_gates = false")
            .replace("local_options = true", "local_options = false");
        fs::write(&config, config_text).unwrap();

        let state = run.join("state");
        antislop::lean::daemon::private_directory(&state).unwrap();
        let registry = run.join("config-registry");
        antislop::lean::daemon::private_directory(&registry).unwrap();
        let core = Core::open(
            &state,
            Config::load(&config).expect("fixture config parses"),
        )
        .expect("fixture Core opens");
        let conversation = support::fixture_conversation_with_requirements(
            &core,
            &project,
            "Goal: review a manual fix. Scope: isolated fixture files. Acceptance: reviewer approves the fix.",
        )
        .unwrap()
        .id;
        core.send(
            &conversation,
            if with_dependent {
                "manual recovery fixture with dependent DAG"
            } else {
                "manual recovery fixture"
            },
        )
        .expect("fixture plan request is accepted");
        let deadline = Instant::now() + Duration::from_secs(120);
        let plan = loop {
            if let Some(task) = core.store.latest_plan(&conversation).unwrap().first()
                && core
                    .store
                    .plan(&task.plan_id)
                    .unwrap()
                    .iter()
                    .all(|task| task.status == "proposed")
                && core
                    .store
                    .plan_review_approved(&conversation, &task.plan_id)
                    .unwrap()
            {
                break task.plan_id.clone();
            }
            assert!(Instant::now() < deadline, "fixture plan review timed out");
            thread::sleep(Duration::from_millis(20));
        };
        fs::write(run.join("plan-id.txt"), &plan).unwrap();
        drop(core);
        let daemon_stderr = fs::File::create(run.join("daemon.stderr")).unwrap();
        let mut child = ProcessCommand::new(env!("CARGO_BIN_EXE_antislop"))
            .args([
                "__core",
                project.to_str().unwrap(),
                config.to_str().unwrap(),
                state.to_str().unwrap(),
            ])
            .env("ANTISLOP_CONFIG_DIR", &registry)
            .env("ANTISLOP_SKIP_AGY_BRIDGE_CHECK", "1")
            .env("FIXTURE_HOLD_WORKER", if hold_worker { "1" } else { "0" })
            .env(
                "FIXTURE_HOLD_ORCHESTRATOR",
                if hold_worker { "1" } else { "0" },
            )
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(daemon_stderr))
            .spawn()
            .map(common::ChildGuard::new)
            .unwrap();
        let client = Client::at(&state);
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                panic!(
                    "isolated daemon exited {status}: {}",
                    fs::read_to_string(run.join("daemon.stderr")).unwrap_or_default()
                );
            }
            if client
                .call(&Request::Control(ControlRequest::Identity))
                .is_ok()
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for isolated daemon identity: {}",
                fs::read_to_string(run.join("daemon.stderr")).unwrap_or_default()
            );
            thread::sleep(Duration::from_millis(20));
        }
        Self {
            _directory: directory,
            run,
            child,
            client,
            conversation,
        }
    }

    fn app(&self) -> (App, UpdateReceiver) {
        App::test_app_with_client(self.client.clone())
    }

    fn new_conversation(&self, app: &mut App, updates: &UpdateReceiver) -> String {
        app.open(self.conversation.clone());
        pump_until(app, updates, "pre-reviewed isolated plan", |app| {
            app.view.plan.is_some() && !app.view.tasks.is_empty()
        });
        app.conversation.clone()
    }

    fn project(&self) -> PathBuf {
        self.run.join("project")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        common::stop_daemon(&self.client);
        self.child.stop();
    }
}

fn install_cli(path: &Path, source: &str) {
    use std::os::unix::fs::PermissionsExt;
    fs::write(path, source).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn git(project: &Path, args: &[&str]) -> String {
    let output = ProcessCommand::new("git")
        .arg("-C")
        .arg(project)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn wait_for(label: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(120);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {label}");
        thread::sleep(Duration::from_millis(20));
    }
}

fn pump_until(
    app: &mut App,
    updates: &UpdateReceiver,
    label: &str,
    mut done: impl FnMut(&App) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(120);
    while !done(app) {
        assert!(Instant::now() < deadline, "timed out waiting for {label}");
        if let Ok(update) = updates.recv_timeout(Duration::from_millis(50)) {
            app.apply_update(update);
        }
    }
}

fn pump_screen_until(app: &mut App, updates: &UpdateReceiver, label: &str, marker: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let screen = capture(app);
        if screen.contains(marker) {
            return screen;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {label}: {screen}"
        );
        if let Ok(update) = updates.recv_timeout(Duration::from_millis(50)) {
            app.apply_update(update);
        }
    }
}

fn capture(app: &mut App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(500, 40)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    (0..40)
        .map(|row| {
            (0..500)
                .map(|column| terminal.backend().buffer()[(column, row)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn click_workflow_control(app: &mut App, label: &str) {
    let cancel = label == "Cancel";
    let label = match label {
        "Pause" => "space pause",
        "Continue" => "space continue",
        "Cancel" => "ctrl p actions",
        label => label,
    };
    let mut terminal = Terminal::new(TestBackend::new(500, 40)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let (row, text, start) = (0..40)
        .find_map(|row| {
            let text = (0..500)
                .map(|column| terminal.backend().buffer()[(column, row)].symbol())
                .collect::<String>();
            text.find(label).map(|start| (row, text, start))
        })
        .unwrap_or_else(|| panic!("workflow control {label:?} absent: {}", capture(app)));
    let column = UnicodeWidthStr::width(&text[..start]) as u16;
    app.mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    });
    if cancel {
        assert!(app.picker.is_some());
        for ch in "Cancel current work".chars() {
            app.key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    }
}

const CODEX: &str = r##"#!/usr/bin/env python3
import json, os, sys, time
prompt = sys.argv[-1]
if os.environ.get('FIXTURE_HOLD_ORCHESTRATOR') == '1' and 'hold-for-cancel' in prompt:
    here = os.path.dirname(sys.argv[0])
    open(os.path.join(here, 'orchestrator-started'), 'w').close()
    while not os.path.exists(os.path.join(here, 'release-orchestrator')):
        time.sleep(0.02)
tasks = [{'id':'manual','title':'Manual recovery fixture','goal':'Review the user manual fix','kind':'implementation','change_kind':'code','acceptance_criteria':[{'id':'C1','text':'The manual fix is reviewed'}],'reviewer':{'summary':'Check the manual fix'}}]
if 'manual recovery fixture with dependent DAG' in prompt:
    tasks.append({'id':'followup','title':'Dependent follow-up','goal':'Complete the step after the manual recovery is integrated','kind':'implementation','change_kind':'code','depends_on':['manual'],'acceptance_criteria':[{'id':'C1','text':'The dependent step is reviewed'}],'reviewer':{'summary':'Check the dependent step'}})
plan = json.dumps({'tasks':tasks})
if 'SESSION PHASE: planning.' in prompt:
    answer = 'The isolated manual recovery plan is ready.' + chr(10) + '```plan' + chr(10) + plan + chr(10) + '```'
elif 'SESSION PHASE: implementation.' in prompt:
    answer = 'The stopped task is waiting for the user to choose a recovery action.'
else:
    answer = 'The reviewed recovery result is recorded.'
print(json.dumps({'type':'thread.started','thread_id':'recovery-fixture'}))
print(json.dumps({'type':'item.completed','item':{'type':'agent_message','text':answer}}))
print(json.dumps({'type':'turn.completed','usage':{}}))
"##;

const AGY: &str = r##"#!/usr/bin/env python3
import json, os, sys
prompt = sys.argv[-1]
here = os.path.dirname(sys.argv[0])
dag_plan = os.path.exists(os.path.join(here, 'dag-plan'))
if 'You review' in prompt or 'You audit' in prompt or 'Review:' in prompt:
    if dag_plan and 'You review' in prompt and os.path.exists(os.path.join(here, 'fail-manual-review')):
        os.remove(os.path.join(here, 'fail-manual-review'))
        print('fixture raw reviewer failure: deterministic stop after structured worker result', file=sys.stderr)
        sys.exit(18)
    answer = '```verdict' + chr(10) + '{"approved":true,"findings":[]}' + chr(10) + '```'
elif 'Task: Manual recovery fixture' in prompt:
    if dag_plan and os.path.exists(os.path.join(here, 'retry-manual')):
        with open('worker-result.txt', 'w') as result:
            result.write('structured worker result before manual recovery\n')
        open(os.path.join(here, 'fail-manual-review'), 'w').close()
        answer = json.dumps({'type':'completed','summary':'The worker created worker-result.txt before review stopped.','report':'Created worker-result.txt as the first reviewed attempt.','commit_message':'feat: add the first recovery result'})
    elif os.environ.get('FIXTURE_HOLD_WORKER') == '1':
        import time
        open(os.path.join(here, 'worker-started'), 'w').close()
        while not os.path.exists(os.path.join(here, 'release-worker')):
            time.sleep(0.02)
        print('fixture raw worker failure: deterministic stop before result', file=sys.stderr)
        sys.exit(17)
    elif not dag_plan:
        print('fixture raw worker failure: deterministic stop before result', file=sys.stderr)
        sys.exit(17)
    else:
        print('fixture raw worker failure: deterministic stop before result', file=sys.stderr)
        sys.exit(17)
elif 'Task: Dependent follow-up' in prompt:
    with open('dependent-result.txt', 'w') as result:
        result.write('dependent task ran after manual recovery integration\n')
    answer = json.dumps({'type':'completed','summary':'The dependent follow-up created dependent-result.txt.','report':'Created dependent-result.txt after the manually integrated predecessor.','commit_message':'feat: add the dependent recovery result'})
else:
    answer = 'Fixture response.'
print(json.dumps({'event':'init','conversation_id':'fixture'}))
print(json.dumps({'event':'result','result':{'conversation_id':'fixture','status':'SUCCESS','response':answer,'usage':{}}}))
"##;

#[test]
fn real_tui_failed_recovery_keeps_raw_error_and_rejects_stale_freshness_token() {
    let fixture = Fixture::start(false);
    let (mut app, updates) = fixture.app();
    let conversation = fixture.new_conversation(&mut app, &updates);
    let plan = app.view.plan.clone().unwrap();
    let task_id = app.view.tasks.first().unwrap().id.clone();
    let stale = fixture
        .client
        .call(&Request::Conversation(
            antislop::lean::daemon::ConversationRequest::Task {
                conversation: Some(conversation.clone()),
                plan: plan.clone(),
                id: task_id.clone(),
            },
        ))
        .unwrap()["freshness_token"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        app.view.plan.is_some(),
        "plan is proposed before acceptance"
    );
    app.run_workflow(Command::AcceptPlan);
    pump_until(&mut app, &updates, "naturally failed worker", |app| {
        app.view.tasks.iter().any(|task| task.status == "failed")
    });
    // Close the independent rigor recommendation so the next key reaches
    // task recovery.
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    let task = app
        .view
        .tasks
        .iter()
        .find(|task| task.id == task_id && task.status == "failed")
        .unwrap();
    let raw_failure = task.reason.clone().unwrap_or_default();
    assert!(
        raw_failure.contains("fixture raw worker failure"),
        "raw cause: {raw_failure}"
    );
    let original_head = git(&fixture.project(), &["rev-parse", "HEAD"]);
    let stale_reply = fixture.client.call(&Request::Conversation(
        antislop::lean::daemon::ConversationRequest::Recover(antislop::lean::TaskRecoveryRequest {
            conversation: conversation.clone(),
            plan: plan.clone(),
            task: task_id.clone(),
            action: antislop::lean::RecoveryAction::RetryWithGuidance,
            consent_files: None,
            freshness_token: stale,
            message: None,
        }),
    ));
    let stale_error = stale_reply
        .expect_err("stale recovery token is rejected")
        .to_string();
    assert!(
        stale_error.to_lowercase().contains("fresh")
            || stale_error.to_lowercase().contains("stale"),
        "{stale_error}"
    );
    assert_eq!(
        git(&fixture.project(), &["rev-parse", "HEAD"]),
        original_head
    );

    // Retry through the recovery modal. The deterministic provider fails again;
    // the task must retain the raw cause and must never be projected as success.
    app.selected = format!("worker:{task_id}");
    app.key(KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE));
    let before_retry = fixture
        .client
        .call(&Request::Conversation(
            antislop::lean::daemon::ConversationRequest::Task {
                conversation: Some(conversation.clone()),
                plan: plan.clone(),
                id: task_id.clone(),
            },
        ))
        .unwrap();
    let before_retry_round = before_retry["round"].as_u64().unwrap_or_default();
    assert_eq!(
        before_retry_round, 1,
        "the first natural worker failure is round one"
    );
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(
        app.editor.is_some(),
        "retry opens its scoped guidance draft"
    );
    for ch in "retry after raw failure".chars() {
        app.key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    wait_for("retry to fail naturally", || {
        fixture
            .client
            .call(&Request::Conversation(
                antislop::lean::daemon::ConversationRequest::Task {
                    conversation: Some(conversation.clone()),
                    plan: plan.clone(),
                    id: task_id.clone(),
                },
            ))
            .is_ok_and(|task| {
                task["status"] == "failed"
                    && task["round"].as_u64().unwrap_or_default() >= 2
                    && task["feedback"]["current_reason"]
                        .as_str()
                        .unwrap_or_default()
                        .contains("fixture raw worker failure")
            })
    });
    let retried = fixture
        .client
        .call(&Request::Conversation(
            antislop::lean::daemon::ConversationRequest::Task {
                conversation: Some(conversation.clone()),
                plan: plan.clone(),
                id: task_id.clone(),
            },
        ))
        .unwrap();
    assert_eq!(retried["status"], "failed");
    assert!(
        retried["feedback"]["current_reason"]
            .as_str()
            .unwrap()
            .contains("fixture raw worker failure")
    );
    assert!(!["approved", "done", "integrated"].contains(&retried["status"].as_str().unwrap()));
    pump_until(
        &mut app,
        &updates,
        "retry failure in TUI projection",
        |app| {
            app.view.tasks.iter().any(|task| {
                task.id == task_id && task.status == "failed" && task.round > before_retry_round
            })
        },
    );

    fs::write(
        fixture.project().join("manual.txt"),
        "manual fix approved\n",
    )
    .unwrap();
    fs::write(
        fixture.project().join("user-notes.txt"),
        "keep this user change\n",
    )
    .unwrap();

    // F5 opens the actual recovery overlay for the naturally failed writer.
    app.selected = format!("worker:{task_id}");
    app.key(KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE));
    let screen = capture(&mut app);
    assert!(
        screen.contains("Task actions"),
        "recovery modal missing: {screen}"
    );
    assert!(
        screen.contains("Retry with a hint"),
        "retry action missing: {screen}"
    );
    assert!(
        screen.contains("I fixed it myself: review my working tree"),
        "{screen}"
    );
    assert!(
        screen.contains("Review integration readiness and choose files"),
        "{screen}"
    );

    // Select the real ReviewManualSnapshot action through the modal, then
    // wait for the daemon's independent reviewer to approve that snapshot.
    for _ in 0..3 {
        app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    }
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    pump_until(&mut app, &updates, "manual snapshot review", |app| {
        app.view
            .tasks
            .iter()
            .any(|task| task.id == task_id && task.status == "needs_user")
    });
    let reviewed = fixture
        .client
        .call(&Request::Conversation(
            antislop::lean::daemon::ConversationRequest::Task {
                conversation: Some(conversation.clone()),
                plan: plan.clone(),
                id: task_id.clone(),
            },
        ))
        .unwrap();
    let reviewed_snapshot = reviewed["result"]
        .as_str()
        .expect("reviewed snapshot id")
        .to_owned();
    let reviewed_round = reviewed["round"].as_u64().expect("review round");
    assert_eq!(reviewed["status"], "needs_user");
    assert_eq!(reviewed["verdict"]["approved"], true);
    assert_eq!(reviewed["verdict"]["result"], reviewed_snapshot);
    assert_eq!(reviewed["verdict"]["round"], reviewed_round);
    assert_eq!(
        git(&fixture.project(), &["rev-parse", "HEAD"]),
        original_head,
        "snapshot review alone must not commit user files"
    );
    assert_eq!(
        fs::read_to_string(fixture.project().join("manual.txt")).unwrap(),
        "manual fix approved\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.project().join("user-notes.txt")).unwrap(),
        "keep this user change\n"
    );

    // Open the consent form through F5 and choose only the reviewed manual fix.
    app.key(KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let consent_screen = pump_screen_until(
        &mut app,
        &updates,
        "recovery consent form",
        "Recovery integration consent",
    );
    assert!(
        consent_screen.contains("project working tree"),
        "{consent_screen}"
    );
    assert!(
        consent_screen.contains(&reviewed_snapshot),
        "{consent_screen}"
    );
    assert!(consent_screen.contains("manual.txt"), "{consent_screen}");
    assert!(
        consent_screen.contains("user-notes.txt"),
        "{consent_screen}"
    );
    let diff = fixture
        .client
        .call(&Request::Conversation(
            antislop::lean::daemon::ConversationRequest::Diff {
                conversation: conversation.clone(),
                plan: plan.clone(),
                id: task_id.clone(),
            },
        ))
        .unwrap();
    let changed_files = diff["changed_files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file.as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(changed_files, ["manual.txt", "user-notes.txt"]);
    let manual_index = changed_files
        .iter()
        .position(|path| path == "manual.txt")
        .unwrap();
    for _ in 0..manual_index {
        app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    }
    app.key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let confirmation = capture(&mut app);
    assert!(
        confirmation.contains("Confirm recovery integration"),
        "{confirmation}"
    );
    assert!(confirmation.contains("manual.txt"), "{confirmation}");
    assert!(
        !confirmation
            .lines()
            .skip(6)
            .take(28)
            .map(
                |row| unicode_segmentation::UnicodeSegmentation::graphemes(row, true)
                    .skip(208)
                    .take(84)
                    .collect::<String>()
            )
            .collect::<Vec<_>>()
            .join("\n")
            .contains("user-notes.txt"),
        "unselected user file must not be authorized: {confirmation}"
    );
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    wait_for("first consented file integration", || {
        fixture
            .client
            .call(&Request::Conversation(
                antislop::lean::daemon::ConversationRequest::Task {
                    conversation: Some(conversation.clone()),
                    plan: plan.clone(),
                    id: task_id.clone(),
                },
            ))
            .is_ok_and(|task| {
                task["feedback"]["manual_snapshot"]["integrated_files"]
                    .as_array()
                    .is_some_and(|files| files.iter().any(|file| file == "manual.txt"))
            })
    });
    let partial = fixture
        .client
        .call(&Request::Conversation(
            antislop::lean::daemon::ConversationRequest::Task {
                conversation: Some(conversation.clone()),
                plan: plan.clone(),
                id: task_id.clone(),
            },
        ))
        .unwrap();
    assert_eq!(partial["status"], "needs_user");
    assert_eq!(
        partial["feedback"]["manual_snapshot"]["integrated_files"],
        json!(["manual.txt"])
    );
    assert_eq!(
        git(&fixture.project(), &["show", "HEAD:manual.txt"]),
        "manual fix approved"
    );
    let integrated_head = git(&fixture.project(), &["rev-parse", "HEAD"]);
    assert_ne!(
        integrated_head, original_head,
        "consented file must be committed"
    );
    assert_eq!(
        fs::read_to_string(fixture.project().join("user-notes.txt")).unwrap(),
        "keep this user change\n"
    );
    let status = git(&fixture.project(), &["status", "--short"]);
    assert!(
        status.contains("user-notes.txt"),
        "unselected user file must remain uncommitted: {status}"
    );
    let show_unselected = ProcessCommand::new("git")
        .arg("-C")
        .arg(fixture.project())
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .args(["show", "HEAD:user-notes.txt"])
        .output()
        .unwrap();
    assert!(
        !show_unselected.status.success(),
        "unselected user file entered the commit unexpectedly"
    );

    // Explicitly authorize the remaining reviewed file in a second consent
    // round; only then may the task and its dependent completion settle.
    app.key(KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let second_consent_screen = pump_screen_until(
        &mut app,
        &updates,
        "second recovery consent form",
        "Recovery integration consent",
    );
    assert!(
        second_consent_screen.contains("user-notes.txt"),
        "{second_consent_screen}"
    );
    let remaining_diff = fixture
        .client
        .call(&Request::Conversation(
            antislop::lean::daemon::ConversationRequest::Diff {
                conversation: conversation.clone(),
                plan: plan.clone(),
                id: task_id.clone(),
            },
        ))
        .unwrap();
    let remaining_files = remaining_diff["changed_files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file.as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    let notes_index = remaining_files
        .iter()
        .position(|path| path == "user-notes.txt")
        .expect("remaining reviewed user file is available for explicit consent");
    for _ in 0..notes_index {
        app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    }
    app.key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    wait_for("consented recovery completion", || {
        fixture
            .client
            .call(&Request::Conversation(
                antislop::lean::daemon::ConversationRequest::Task {
                    conversation: Some(conversation.clone()),
                    plan: plan.clone(),
                    id: task_id.clone(),
                },
            ))
            .is_ok_and(|task| task["status"] == "approved")
    });
    let integrated = fixture
        .client
        .call(&Request::Conversation(
            antislop::lean::daemon::ConversationRequest::Task {
                conversation: Some(conversation.clone()),
                plan: plan.clone(),
                id: task_id.clone(),
            },
        ))
        .unwrap();
    assert_eq!(integrated["status"], "approved");
    assert_eq!(
        integrated["feedback"]["manual_snapshot"]["integrated_files"],
        json!(["manual.txt", "user-notes.txt"])
    );
    assert_eq!(
        git(&fixture.project(), &["show", "HEAD:user-notes.txt"]),
        "keep this user change"
    );
    let final_head = git(&fixture.project(), &["rev-parse", "HEAD"]);

    pump_until(&mut app, &updates, "completed session event", |app| {
        app.view.cards.iter().any(|card| {
            matches!(
                card,
                antislop::lean::tui::view::TranscriptCard::Completion { .. }
            )
        })
    });
    app.selected = "conversation".into();
    app.focus = antislop::lean::tui::Focus::Graph;
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    // The reference split keeps the PTY visible. Expand content to inspect the
    // complete completion result, including the text below its action list.
    app.key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
    let final_panel = capture(&mut app);
    assert!(final_panel.contains("Session completion"), "{final_panel}");
    assert!(final_panel.contains("Final branch:"), "{final_panel}");
    assert!(final_panel.contains("Completion result:"), "{final_panel}");
    assert!(final_panel.contains("Create PR"), "{final_panel}");
    let (completion_branch, completion_summary, has_create_pr) = app
        .view
        .cards
        .iter()
        .find_map(|card| match card {
            antislop::lean::tui::view::TranscriptCard::Completion {
                branch,
                summary,
                actions,
                ..
            } => Some((
                branch
                    .clone()
                    .expect("completion contains the actual session branch"),
                summary.clone(),
                actions.iter().any(|action| {
                    action.kind == antislop::lean::daemon::SessionActionKind::CreatePr
                }),
            )),
            _ => None,
        })
        .expect("rendered transcript contains the completion card");
    assert!(final_panel.contains(&format!("Final branch: {completion_branch}")));
    assert!(
        !completion_summary.trim().is_empty(),
        "completion result should be populated from settled task outcomes"
    );
    assert!(
        has_create_pr,
        "the completion card exposes its real Create PR action"
    );
    let result_marker = completion_summary
        .split_whitespace()
        .take(4)
        .collect::<Vec<_>>()
        .join(" ");
    assert!(!result_marker.is_empty());
    let visible_result = final_panel.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        visible_result.contains(&result_marker),
        "missing {result_marker:?}: {final_panel}"
    );
    let sequence = json!({
        "conversation": conversation,
        "plan": plan,
        "task": task_id,
        "stale_token_error": stale_error,
        "available_actions": screen,
        "raw_worker_failure": raw_failure,
        "initial_task_status": "failed",
        "retried_task": retried,
        "head_before_recovery": original_head,
        "head_after_partial_integration": integrated_head,
        "head_after_recovery": final_head,
        "manual_user_file": fs::read_to_string(fixture.project().join("manual.txt")).unwrap(),
        "reviewed_snapshot": reviewed_snapshot,
        "reviewed_round": reviewed_round,
        "partial_integration": partial,
        "integrated_task": integrated,
        "final_panel": final_panel,
        "completion_branch": completion_branch,
        "completion_summary": completion_summary,
        "completion_has_create_pr": has_create_pr,
    });
    fs::write(
        fixture.run.join("recovery-sequence.json"),
        serde_json::to_vec_pretty(&sequence).unwrap(),
    )
    .unwrap();
}

#[test]
fn real_recovery_controls_pause_continue_and_cancel_preserve_reviewed_decision() {
    let fixture = Fixture::start(true);
    let (mut app, updates) = fixture.app();
    let conversation = fixture.new_conversation(&mut app, &updates);
    let plan = app.view.plan.clone().unwrap();
    let task_id = app.view.tasks.first().unwrap().id.clone();
    app.run_workflow(Command::AcceptPlan);
    wait_for("recovery scenario worker hold", || {
        fixture.run.join("bin/worker-started").exists()
    });
    pump_until(
        &mut app,
        &updates,
        "held recovery worker projection",
        |app| {
            app.view
                .tasks
                .iter()
                .any(|task| task.id == task_id && task.status == "working")
        },
    );
    let worker_uuid = app
        .view
        .tasks
        .first()
        .and_then(|task| task.uuid.as_deref())
        .expect("worker node has a terminal role id")
        .to_owned();
    let worker_role = format!("worker:{worker_uuid}");
    let terminal_open = fixture
        .client
        .call(&Request::Terminal(TerminalRequest::TerminalOpen {
            conversation: conversation.clone(),
            role: worker_role.clone(),
            columns: 80,
            rows: 24,
            retry: false,
        }))
        .expect("isolated worker terminal opens before recovery");
    let terminal_handle = terminal_open["handle"].as_str().unwrap().to_owned();
    fs::write(fixture.run.join("bin/release-worker"), "release\n").unwrap();
    pump_until(
        &mut app,
        &updates,
        "naturally failed recovery task",
        |app| {
            app.view
                .tasks
                .iter()
                .any(|task| task.id == task_id && task.status == "failed")
        },
    );
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    fs::write(
        fixture.project().join("manual.txt"),
        "recovery decision remains\n",
    )
    .unwrap();
    app.selected = format!("worker:{task_id}");
    app.key(KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE));
    for _ in 0..3 {
        app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    }
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    pump_until(
        &mut app,
        &updates,
        "reviewed manual recovery state",
        |app| {
            app.view
                .tasks
                .iter()
                .any(|task| task.id == task_id && task.status == "needs_user")
        },
    );
    let reviewed = fixture
        .client
        .call(&Request::Conversation(
            antislop::lean::daemon::ConversationRequest::Task {
                conversation: Some(conversation.clone()),
                plan: plan.clone(),
                id: task_id.clone(),
            },
        ))
        .unwrap();
    let reviewed_snapshot = reviewed["result"].as_str().unwrap().to_owned();
    assert_eq!(reviewed["verdict"]["approved"], true);
    let head_before_controls = git(&fixture.project(), &["rev-parse", "HEAD"]);

    // Use the command palette while the task is in needs_user recovery. Both
    // toggles must preserve the reviewed snapshot and its manual decision.
    app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    for ch in "pause".chars() {
        app.key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    pump_until(&mut app, &updates, "recovery session paused", |app| {
        app.view.paused
    });
    assert_eq!(
        app.view
            .tasks
            .iter()
            .find(|task| task.id == task_id)
            .unwrap()
            .status,
        "needs_user"
    );
    app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    for ch in "pause".chars() {
        app.key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    pump_until(&mut app, &updates, "recovery session continued", |app| {
        !app.view.paused
    });

    // Hold an unrelated orchestration turn so Cancel remains actionable while
    // the reviewed manual decision stays visible in recovery.
    app.key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
    app.input = InputBuffer::with_text("hold-for-cancel");
    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    wait_for("recovery scenario orchestrator hold", || {
        fixture.run.join("bin/orchestrator-started").exists()
    });
    pump_until(&mut app, &updates, "active turn beside recovery", |app| {
        !app.view.idle
    });
    // Cancel through the visible toolbar in the same recovery state. This
    // isolated fixture cannot address a user's active session.
    click_workflow_control(&mut app, "Cancel");
    wait_for("recovery cancellation event", || {
        fixture
            .client
            .call(&Request::History(
                antislop::lean::daemon::HistoryRequest::Events {
                    conversation: conversation.clone(),
                    after: 0,
                    wait_ms: 0,
                },
            ))
            .is_ok_and(|events| events.to_string().contains("cancelled"))
    });
    let after_cancel = fixture
        .client
        .call(&Request::Conversation(
            antislop::lean::daemon::ConversationRequest::Task {
                conversation: Some(conversation.clone()),
                plan: plan.clone(),
                id: task_id.clone(),
            },
        ))
        .unwrap();
    assert_eq!(after_cancel["status"], "needs_user");
    assert_eq!(after_cancel["result"], reviewed_snapshot);
    assert_eq!(after_cancel["verdict"]["approved"], true);
    assert_eq!(
        git(&fixture.project(), &["rev-parse", "HEAD"]),
        head_before_controls
    );
    assert_eq!(
        fs::read_to_string(fixture.project().join("manual.txt")).unwrap(),
        "recovery decision remains\n"
    );
    let terminal_after_cancel = fixture
        .client
        .call(&Request::Terminal(TerminalRequest::TerminalRead {
            conversation: conversation.clone(),
            role: worker_role.clone(),
            handle: terminal_handle.clone(),
            after: 0,
        }))
        .expect("recovery cancellation must preserve the worker terminal");
    assert_eq!(terminal_after_cancel["handle"], terminal_handle);
    let screen = capture(&mut app);
    let sequence = json!({
        "conversation": conversation,
        "task": task_id,
        "reviewed_snapshot": reviewed_snapshot,
        "controls": ["palette /pause", "toolbar Cancel"],
        "terminal_role": worker_role,
        "terminal_handle": terminal_handle,
        "task_after_cancel": after_cancel,
        "terminal_after_cancel": terminal_after_cancel,
        "head_before_controls": head_before_controls,
        "screen": screen,
    });
    fs::write(
        fixture.run.join("recovery-controls-sequence.json"),
        serde_json::to_vec_pretty(&sequence).unwrap(),
    )
    .unwrap();
}

#[test]
fn real_tui_manual_snapshot_consent_runs_dependent_dag_to_completion() {
    let fixture = Fixture::start_with_dag(false);
    let (mut app, updates) = fixture.app();
    let conversation = fixture.new_conversation(&mut app, &updates);
    let plan = app.view.plan.clone().unwrap();
    let task_id = app.view.tasks.first().unwrap().id.clone();
    let dependent_id = app
        .view
        .tasks
        .iter()
        .find(|task| task.title == "Dependent follow-up")
        .expect("the fixture plan contains a dependent task")
        .id
        .clone();
    app.run_workflow(Command::AcceptPlan);
    pump_until(&mut app, &updates, "natural manual task failure", |app| {
        app.view
            .tasks
            .iter()
            .any(|task| task.id == task_id && task.status == "failed")
    });
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    fs::write(
        fixture.run.join("bin/retry-manual"),
        "retry with structured result\n",
    )
    .unwrap();
    app.selected = format!("worker:{task_id}");
    app.key(KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(
        app.editor.is_some(),
        "retry opens its scoped guidance draft"
    );
    for ch in "retry with a structured result before reviewer stop".chars() {
        app.key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    wait_for(
        "structured worker result followed by reviewer failure",
        || {
            fixture
                .client
                .call(&Request::Conversation(
                    antislop::lean::daemon::ConversationRequest::Task {
                        conversation: Some(conversation.clone()),
                        plan: plan.clone(),
                        id: task_id.clone(),
                    },
                ))
                .is_ok_and(|task| {
                    task["status"] == "failed"
                        && task["round"].as_u64().unwrap_or_default() >= 2
                        && task["feedback"]["current_reason"]
                            .as_str()
                            .unwrap_or_default()
                            .contains("fixture raw reviewer failure")
                })
        },
    );
    let stopped = fixture
        .client
        .call(&Request::Conversation(
            antislop::lean::daemon::ConversationRequest::Task {
                conversation: Some(conversation.clone()),
                plan: plan.clone(),
                id: task_id.clone(),
            },
        ))
        .unwrap();
    assert!(
        stopped["result"].as_str().is_some(),
        "the worker result precedes the reviewer failure: {stopped}"
    );
    assert_eq!(stopped["status"], "failed");
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    let dependent_before_consent = fixture
        .client
        .call(&Request::Conversation(
            antislop::lean::daemon::ConversationRequest::Task {
                conversation: Some(conversation.clone()),
                plan: plan.clone(),
                id: dependent_id.clone(),
            },
        ))
        .unwrap();
    assert_eq!(dependent_before_consent["status"], "queued");

    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    fs::write(
        fixture.project().join("manual.txt"),
        "DAG recovery result\n",
    )
    .unwrap();
    app.selected = format!("worker:{task_id}");
    app.key(KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE));
    for _ in 0..3 {
        app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    }
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    pump_until(
        &mut app,
        &updates,
        "manual snapshot review for DAG",
        |app| {
            app.view
                .tasks
                .iter()
                .any(|task| task.id == task_id && task.status == "needs_user")
        },
    );
    let reviewed = fixture
        .client
        .call(&Request::Conversation(
            antislop::lean::daemon::ConversationRequest::Task {
                conversation: Some(conversation.clone()),
                plan: plan.clone(),
                id: task_id.clone(),
            },
        ))
        .unwrap();
    let snapshot = reviewed["result"].as_str().unwrap().to_owned();
    let before_consent_head = git(&fixture.project(), &["rev-parse", "HEAD"]);
    app.key(KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let consent = pump_screen_until(
        &mut app,
        &updates,
        "DAG recovery consent form",
        "Recovery integration consent",
    );
    assert!(consent.contains("manual.txt"), "{consent}");
    app.key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(capture(&mut app).contains("Confirm recovery integration"));
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    wait_for("manual task consent integration", || {
        fixture
            .client
            .call(&Request::Conversation(
                antislop::lean::daemon::ConversationRequest::Task {
                    conversation: Some(conversation.clone()),
                    plan: plan.clone(),
                    id: task_id.clone(),
                },
            ))
            .is_ok_and(|task| task["status"] == "approved")
    });
    let integrated_head = git(&fixture.project(), &["rev-parse", "HEAD"]);
    assert_ne!(integrated_head, before_consent_head);
    assert_eq!(
        git(&fixture.project(), &["show", "HEAD:manual.txt"]),
        "DAG recovery result"
    );
    wait_for("dependent DAG task to be reviewed after consent", || {
        fixture
            .client
            .call(&Request::Conversation(
                antislop::lean::daemon::ConversationRequest::Task {
                    conversation: Some(conversation.clone()),
                    plan: plan.clone(),
                    id: dependent_id.clone(),
                },
            ))
            .is_ok_and(|task| task["status"] == "approved")
    });
    let dependent = fixture
        .client
        .call(&Request::Conversation(
            antislop::lean::daemon::ConversationRequest::Task {
                conversation: Some(conversation.clone()),
                plan: plan.clone(),
                id: dependent_id.clone(),
            },
        ))
        .unwrap();
    assert_eq!(
        dependent["status"], "approved",
        "dependent cascade result: {dependent}"
    );
    let dependent_commit = dependent["result"].as_str().unwrap();
    assert!(
        git(
            &fixture.project(),
            &["show", &format!("{dependent_commit}:dependent-result.txt"),]
        )
        .contains("dependent task ran after manual recovery integration")
    );
    assert_eq!(reviewed["verdict"]["approved"], true);
    assert_eq!(reviewed["result"], snapshot);
    pump_until(&mut app, &updates, "DAG session completion event", |app| {
        app.view.cards.iter().any(|card| {
            matches!(
                card,
                antislop::lean::tui::view::TranscriptCard::Completion { .. }
            )
        })
    });
    app.selected = "conversation".into();
    app.focus = antislop::lean::tui::Focus::Graph;
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let final_panel = capture(&mut app);
    assert!(final_panel.contains("Session completion"), "{final_panel}");
    assert!(final_panel.contains("Final branch:"), "{final_panel}");
    assert!(final_panel.contains("Dependent follow-up"), "{final_panel}");
    assert!(final_panel.contains("Create PR"), "{final_panel}");

    let sequence = json!({
        "conversation": conversation,
        "plan": plan,
        "manual_task": task_id,
        "dependent_task": dependent_id,
        "reviewed_snapshot": snapshot,
        "before_consent_head": before_consent_head,
        "integrated_head": integrated_head,
        "manual_task_status": "approved",
        "raw_reviewer_failure": stopped["feedback"]["current_reason"],
        "dependent_before_consent": dependent_before_consent,
        "dependent_after_consent": dependent,
        "dependent_result": dependent["result"],
        "consent_screen": consent,
        "final_panel": final_panel,
    });
    fs::write(
        fixture.run.join("dag-cascade-sequence.json"),
        serde_json::to_vec_pretty(&sequence).unwrap(),
    )
    .unwrap();
}

#[test]
fn real_daemon_pause_continue_and_cancel_keep_draft_and_raw_failure_is_not_success() {
    let fixture = Fixture::start(true);
    let (mut app, updates) = fixture.app();
    let conversation = fixture.new_conversation(&mut app, &updates);
    app.run_workflow(Command::AcceptPlan);
    wait_for("fake worker hold point", || {
        fixture.run.join("bin/worker-started").exists()
    });
    pump_until(&mut app, &updates, "working task projection", |app| {
        app.view.tasks.iter().any(|task| task.status == "working")
    });
    let worker_role = format!(
        "worker:{}",
        app.view.tasks.first().unwrap().uuid.as_deref().unwrap()
    );
    let worker_terminal = fixture
        .client
        .call(&Request::Terminal(TerminalRequest::TerminalOpen {
            conversation: conversation.clone(),
            role: worker_role.clone(),
            columns: 80,
            rows: 24,
            retry: false,
        }))
        .expect("the running worker's native terminal can be attached");
    let terminal_handle = worker_terminal["handle"]
        .as_str()
        .expect("terminal handle")
        .to_owned();
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    // Keep a real orchestration turn active alongside the held task worker so
    // the session toolbar exposes its pause/cancel controls.
    app.key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
    app.input = InputBuffer::with_text("hold-for-cancel");
    app.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    wait_for("fake orchestrator hold point", || {
        fixture.run.join("bin/orchestrator-started").exists()
    });
    pump_until(
        &mut app,
        &updates,
        "active orchestration projection",
        |app| !app.view.idle,
    );

    // Exercise the visible header action route.
    click_workflow_control(&mut app, "Pause");
    pump_until(&mut app, &updates, "paused session state", |app| {
        app.view.paused
    });
    app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    for ch in "pause".chars() {
        app.key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    pump_until(&mut app, &updates, "continued session state", |app| {
        !app.view.paused
    });
    app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    for ch in "pause".chars() {
        app.key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    pump_until(
        &mut app,
        &updates,
        "paused again through command palette",
        |app| app.view.paused,
    );
    click_workflow_control(&mut app, "Continue");
    pump_until(&mut app, &updates, "continued through header", |app| {
        !app.view.paused
    });
    app.input = InputBuffer::with_text("draft survives pause and cancel 👩‍💻");
    assert_eq!(app.input.text, "draft survives pause and cancel 👩‍💻");

    // Click the visible header cancel button while the fake worker is held.
    click_workflow_control(&mut app, "Cancel");
    wait_for("cancel event in isolated daemon", || {
        let Ok(events) = fixture.client.call(&Request::History(
            antislop::lean::daemon::HistoryRequest::Events {
                conversation: conversation.clone(),
                after: 0,
                wait_ms: 0,
            },
        )) else {
            return false;
        };
        events.to_string().contains("cancelled")
    });
    let terminal_after_cancel = fixture
        .client
        .call(&Request::Terminal(TerminalRequest::TerminalRead {
            conversation: conversation.clone(),
            role: worker_role.clone(),
            handle: terminal_handle.clone(),
            after: 0,
        }))
        .expect("cancellation must retain the attached worker terminal");
    assert_eq!(terminal_after_cancel["handle"], terminal_handle);
    assert_eq!(app.input.text, "draft survives pause and cancel 👩‍💻");
    fs::write(fixture.run.join("bin/release-worker"), "release\n").unwrap();
    wait_for("cancelled held task state", || {
        fixture
            .client
            .call(&Request::Conversation(
                antislop::lean::daemon::ConversationRequest::Task {
                    conversation: Some(conversation.clone()),
                    plan: app.view.plan.clone().unwrap(),
                    id: "manual".into(),
                },
            ))
            .is_ok_and(|task| task["status"] != "working")
    });
    let task = fixture
        .client
        .call(&Request::Conversation(
            antislop::lean::daemon::ConversationRequest::Task {
                conversation: Some(conversation.clone()),
                plan: app.view.plan.clone().unwrap(),
                id: "manual".into(),
            },
        ))
        .unwrap();
    let task_status = task["status"].as_str().unwrap_or_default().to_owned();
    assert_eq!(
        task_status, "queued",
        "cancellation must leave work retryable: {task}"
    );
    assert!(
        task["report"]
            .as_str()
            .unwrap_or_default()
            .contains("cancelled"),
        "the task must retain its cancellation result: {task}"
    );
    assert_ne!(
        task_status, "approved",
        "cancel cannot report success: {task}"
    );
    assert_ne!(task_status, "done", "cancel cannot report success: {task}");
    assert_ne!(
        task_status, "integrated",
        "cancel cannot report success: {task}"
    );
    let cancellation_reason = task["feedback"]["current_reason"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(
        cancellation_reason.contains("cancelled"),
        "the cancelled worker's actual result must remain recorded: {task}"
    );
    app.key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
    let summary = capture(&mut app);
    assert!(
        summary.contains("draft survives pause"),
        "screen must retain draft: {summary}"
    );
    let sequence = json!({
        "conversation": conversation,
        "controls": ["header Pause", "slash palette /pause continue", "slash palette /pause pause", "header Continue", "header Cancel"],
        "draft": app.input.text,
        "terminal_role": worker_role,
        "terminal_handle": terminal_handle,
        "terminal_after_cancel": terminal_after_cancel,
        "task_after_cancel_and_release": task,
        "cancellation_reason": cancellation_reason,
        "rendered_screen": summary,
    });
    fs::write(
        fixture.run.join("control-sequence.json"),
        serde_json::to_vec_pretty(&sequence).unwrap(),
    )
    .unwrap();
}

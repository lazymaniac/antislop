//! End-to-end navigation checks for the Workbench using the real app and daemon.
mod common;
mod support;

use antislop::lean::{
    daemon::{
        Client, ControlRequest, ConversationRequest, HistoryRequest, Request, SessionRequest,
    },
    tui::{App, Command, Focus, Screen, WorkbenchTarget, update_channel},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
#[cfg(target_os = "macos")]
use std::io::{IsTerminal, Write};
use std::{
    path::PathBuf,
    process::{Child, Command as ProcessCommand, Stdio},
    time::{Duration, Instant},
};

struct NavigationFixture {
    child: Child,
    client: Client,
    project: PathBuf,
    state: PathBuf,
    cli: PathBuf,
    _root: tempfile::TempDir,
}

impl NavigationFixture {
    fn start() -> Self {
        let root_guard = tempfile::Builder::new()
            .prefix("anti-nav-")
            .tempdir()
            .unwrap();
        let root = root_guard.path().to_path_buf();
        let project = root.join("p");
        std::fs::create_dir_all(&project).unwrap();
        common::initialize_git_project(&project);
        let fixture = project.join("codex");
        std::fs::write(
            &fixture,
            "#!/usr/bin/env python3\nimport json\nprint(json.dumps({'type':'thread.started','thread_id':'nav-fixture'}))\nprint(json.dumps({'type':'item.completed','item':{'type':'agent_message','text':'navigation fixture response'}}))\nprint(json.dumps({'type':'turn.completed','usage':{'input_tokens':2,'output_tokens':3}}))\n",
        )
        .unwrap();
        common::interactive(&fixture);
        std::fs::write(
            fixture.with_extension("dialog.after_work"),
            "NATIVE PTY READY — press one key to confirm CLI input",
        )
        .unwrap();
        let config = project.join("antislop.toml");
        let content = include_str!("../examples/antislop.toml").replace(
            "binary = \"codex\"",
            &format!("binary = {:?}", fixture.to_string_lossy()),
        );
        std::fs::write(&config, content).unwrap();
        let state = root.join("s");
        std::fs::create_dir_all(&state).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let child = ProcessCommand::new(env!("CARGO_BIN_EXE_antislop"))
            .args(["__core"])
            .arg(&project)
            .arg(&config)
            .arg(&state)
            .env("ANTISLOP_CONFIG_DIR", root.join("g"))
            .env("ANTISLOP_STATE_ROOT", root.join("runtime"))
            .env("ANTISLOP_SKIP_AGY_BRIDGE_CHECK", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let client = Client::at(&state);
        wait_for("fixture daemon identity", || {
            client
                .call(&Request::Control(ControlRequest::Identity))
                .is_ok()
        });
        Self {
            child,
            client,
            project,
            state,
            cli: fixture,
            _root: root_guard,
        }
    }

    fn start_delivery(capture_dir: &std::path::Path) -> Self {
        let root_guard = tempfile::Builder::new()
            .prefix("anti-nav-delivery-")
            .tempdir()
            .unwrap();
        let root = root_guard.path();
        let project = root.join("project");
        std::fs::create_dir_all(&project).unwrap();
        common::initialize_git_project(&project);
        std::fs::write(project.join("navigation-artifact.txt"), "baseline\n").unwrap();
        let baseline = ProcessCommand::new("git")
            .args(["add", "navigation-artifact.txt"])
            .current_dir(&project)
            .output()
            .unwrap();
        assert!(
            baseline.status.success(),
            "git add failed: {}",
            String::from_utf8_lossy(&baseline.stderr)
        );
        let baseline = ProcessCommand::new("git")
            .args([
                "-c",
                "user.name=Antislop Test",
                "-c",
                "user.email=antislop-test@example.invalid",
                "commit",
                "--quiet",
                "-m",
                "seed navigation artifact",
            ])
            .current_dir(&project)
            .output()
            .unwrap();
        assert!(
            baseline.status.success(),
            "baseline commit failed: {}",
            String::from_utf8_lossy(&baseline.stderr)
        );
        let codex = root.join("codex");
        std::fs::write(&codex, r##"#!/usr/bin/env python3
import json, sys
prompt = sys.argv[-1]
plan = {"tasks": [{"id":"t1","title":"Update navigation artifact","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Change navigation-artifact.txt from baseline to the exact text 'from real worker'","change_kind":"code","kind":"implementation","reviewer":{"summary":"file content"}}]}
if "SESSION PHASE: INTERVIEW." in prompt:
    answer = "```requirements\nGoal, scope and acceptance: exercise a real navigation artifact.\n```"
elif "You are the planner" in prompt or "SESSION PHASE: planning" in prompt:
    answer = "Plan.\n```plan\n" + json.dumps(plan) + "\n```"
else:
    answer = "fixture orchestration response"
print(json.dumps({"type":"thread.started","thread_id":"navigation-orchestrator"}))
print(json.dumps({"type":"item.completed","item":{"type":"agent_message","text":answer}}))
print(json.dumps({"type":"turn.completed","usage":{}}))
"##)
        .unwrap();
        support::install_task_protocol(&codex);
        common::interactive(&codex);
        std::fs::set_permissions(&codex, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();

        let agy = root.join("agy");
        std::fs::write(&agy, r##"#!/usr/bin/env python3
import json, os, re, sys
prompt = sys.argv[-1]
ids = re.findall(r"^- ([A-Za-z0-9_.-]+): ", prompt, re.M)
if "You review another agent's work" in prompt:
    with open(os.path.join(os.path.dirname(__file__), "agy.dialog.after_work"), "w") as dialog:
        dialog.write("NATIVE PTY READY — press one key to confirm reviewer input")
    answer = "```verdict\n" + json.dumps({"approved":True,"findings":[],"coverage":{i:{"status":"ok","note":"fixture reviewed"} for i in ids}}) + "\n```"
elif any(marker in prompt for marker in ("You review a plan", "You review a code change", "You audit")):
    answer = "```verdict\n" + json.dumps({"approved":True,"findings":[],"coverage":{i:{"status":"ok","note":"fixture reviewed"} for i in ids}}) + "\n```"
else:
    with open("navigation-artifact.txt", "w") as output:
        output.write("from real worker\n")
    answer = "created navigation-artifact.txt"
print(json.dumps({"event":"init","conversation_id":"navigation-worker"}))
print(json.dumps({"event":"result","result":{"conversation_id":"navigation-worker","status":"SUCCESS","response":answer,"usage":{}}}))
"##)
        .unwrap();
        support::install_task_protocol(&agy);
        support::install_task_options_protocol(&agy);
        common::interactive(&agy);
        instrument_local_native_key_reader(&agy);
        std::fs::set_permissions(&agy, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();

        let config = project.join("antislop.toml");
        let content = include_str!("../examples/antislop.toml")
            .replace(
                "binary = \"codex\"",
                &format!("binary = {:?}", codex.to_string_lossy()),
            )
            .replace(
                "binary = \"agy\"",
                &format!("binary = {:?}", agy.to_string_lossy()),
            )
            .replace("required_gates = true", "required_gates = false")
            .replace("local_options = true", "local_options = false");
        std::fs::write(&config, content).unwrap();
        let state = root.join("state");
        std::fs::create_dir_all(&state).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o700)).unwrap();
        let child = ProcessCommand::new(env!("CARGO_BIN_EXE_antislop"))
            .args(["__core"])
            .arg(&project)
            .arg(&config)
            .arg(&state)
            .env("ANTISLOP_CONFIG_DIR", root.join("global"))
            .env("ANTISLOP_STATE_ROOT", root.join("runtime"))
            .env("ANTISLOP_NAV_CAPTURE_DIR", capture_dir)
            .env("ANTISLOP_SKIP_AGY_BRIDGE_CHECK", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let client = Client::at(&state);
        wait_for("delivery daemon identity", || {
            client
                .call(&Request::Control(ControlRequest::Identity))
                .is_ok()
        });
        Self {
            child,
            client,
            project,
            state,
            cli: agy,
            _root: root_guard,
        }
    }
}

impl Drop for NavigationFixture {
    fn drop(&mut self) {
        let _ = self
            .client
            .call(&Request::Control(ControlRequest::ForceShutdown));
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn wait_for(label: &str, mut ready: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(40);
    while !ready() {
        assert!(Instant::now() < until, "timed out waiting for {label}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn next_update(app: &mut App, updates: &antislop::lean::tui::UpdateReceiver) {
    app.apply_update(updates.recv_timeout(Duration::from_secs(10)).unwrap());
}

fn pump_until(
    app: &mut App,
    updates: &antislop::lean::tui::UpdateReceiver,
    ready: impl Fn(&App) -> bool,
    label: &str,
) {
    let until = Instant::now() + Duration::from_secs(40);
    while !ready(app) {
        assert!(Instant::now() < until, "timed out waiting for {label}");
        if let Ok(update) = updates.recv_timeout(Duration::from_millis(50)) {
            app.apply_update(update);
        }
    }
}

fn instrument_local_native_key_reader(cli: &std::path::Path) {
    let mut shim = cli.as_os_str().to_owned();
    shim.push(".shim.py");
    let shim = PathBuf::from(shim);
    let source = std::fs::read_to_string(&shim).unwrap();
    let original = "        dialog(\"after_work\", fd)\n";
    let instrumented = concat!(
        "        if os.path.exists(here + '.capture_native_terminal') and 'You review another agent\\'s work' in prompt:\n",
        "            sys.stdout.write('\\x1b[2J\\x1b[H' + 'NATIVE PTY READY — key capture is active')\n",
        "            sys.stdout.flush()\n",
        "            open(here + '.native_terminal_ready', 'w').close()\n",
        "            while not os.path.exists(here + '.release_native_terminal'):\n",
        "                chunk = os.read(fd, 65536)\n",
        "                if not chunk:\n                    return 0\n",
        "                capture_dir = os.environ.get('ANTISLOP_NAV_CAPTURE_DIR')\n",
        "                if capture_dir and os.path.exists(here + '.capture_native_keys'):\n",
        "                    with open(os.path.join(capture_dir, 'native-key-bytes.bin'), 'ab') as record:\n",
        "                        record.write(chunk)\n",
        "        else:\n",
        "            dialog(\"after_work\", fd)\n"
    );
    assert_eq!(
        source.matches(original).count(),
        1,
        "fixture after-work dialog location changed"
    );
    std::fs::write(&shim, source.replacen(original, instrumented, 1)).unwrap();
    std::fs::write(
        format!("{}.capture_native_terminal", cli.display()),
        b"capture",
    )
    .unwrap();
}

fn open_reviewer_workbench(
    fixture: &NavigationFixture,
    conversation: &str,
    task_id: &str,
    owner: &str,
    expected_rounds: &[u64],
) -> (App, antislop::lean::tui::UpdateReceiver) {
    let (updates, rx) = update_channel(512);
    let mut app = App::test_app_with_client_and_updates(fixture.client.clone(), updates);
    app.project = fixture.project.to_string_lossy().into_owned();
    app.open(conversation.to_owned());
    pump_until(
        &mut app,
        &rx,
        |app| app.view.tasks.iter().any(|task| task.id == task_id),
        "task graph node for a fresh native-key case",
    );
    draw(&mut app);
    for _ in 0..3 {
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    }
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.focus, Focus::Graph);
    app.key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
    assert!(app.picker.is_some());
    for character in task_id.chars() {
        app.key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
    }
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let graph_agent = format!("reviewer:{task_id}");
    assert_eq!(app.selected, graph_agent);
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.navigation.screen, Screen::Workbench);
    assert_eq!(
        app.navigation.target,
        WorkbenchTarget::Agent(graph_agent.clone())
    );
    assert_eq!(app.details.as_ref().unwrap().key, graph_agent);
    assert_eq!(app.details.as_ref().unwrap().task_id(), Some(task_id));
    pump_until(
        &mut app,
        &rx,
        |app| {
            app.details
                .as_ref()
                .is_some_and(|details| details.available_rounds.as_slice() == expected_rounds)
        },
        "same reviewer task round in fresh Workbench",
    );
    app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE));
    assert!(app.navigation.presentation().pinned);
    assert_eq!(
        app.navigation.presentation().terminal_owner.as_deref(),
        Some(owner)
    );
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(
        app.navigation.focus,
        antislop::lean::tui::NavigationFocus::WorkbenchTerminal
    );
    draw(&mut app);
    app.key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
    assert!(app.navigation.presentation().native_input);
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        let ready = app
            .details
            .as_ref()
            .and_then(|details| details.native.get(owner))
            .and_then(|view| view.screen.as_ref())
            .is_some_and(|screen| screen.error.is_none() && screen.columns > 0 && screen.rows > 0);
        if ready {
            break;
        }
        assert!(
            Instant::now() < until,
            "same terminal did not reopen for {owner}"
        );
        if let Ok(update) = rx.recv_timeout(Duration::from_millis(50)) {
            app.apply_update(update);
        }
        draw(&mut app);
    }
    assert!(app.navigation.presentation().split);
    assert!(draw(&mut app).contains("navigation-artifact.txt"));
    (app, rx)
}

fn screen_text(screen: &antislop::lean::terminal::Screen) -> String {
    let mut cells = screen.cells.iter().collect::<Vec<_>>();
    cells.sort_by_key(|cell| (cell.y, cell.x));
    cells.into_iter().map(|cell| cell.text.as_str()).collect()
}

fn draw(app: &mut App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[test]
fn graph_workbench_history_native_pty_and_a_b_a_preserve_context() {
    let fixture = NavigationFixture::start();
    let (mut app, updates) = App::test_app_with_client(fixture.client.clone());
    app.project = fixture.project.to_string_lossy().into_owned();
    app.run_workflow(Command::NewConversation);
    next_update(&mut app, &updates);
    let conversation_a = app.conversation.clone();
    assert!(!conversation_a.is_empty());

    app.input.insert_str("first conversation draft");
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    pump_until(
        &mut app,
        &updates,
        |app| app.view.agents.contains_key("orchestrator"),
        "graph agent",
    );
    assert_eq!(app.focus, Focus::Chat);
    if app
        .interview_forms
        .get(&conversation_a)
        .is_some_and(|form| form.visible && !form.sent)
    {
        assert!(
            !app.interview_forms[&conversation_a]
                .question
                .options
                .is_empty(),
            "fixture interview must offer an answer selectable through App::key"
        );
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        pump_until(
            &mut app,
            &updates,
            |app| app.interview_forms[&conversation_a].sent,
            "interview answer acknowledgment",
        );
        assert!(app.interview_forms[&conversation_a].sent);
    }
    for _ in 0..3 {
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    }
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.focus, Focus::Graph);
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.navigation.screen, Screen::Workbench);
    assert_eq!(
        app.navigation.target,
        WorkbenchTarget::Agent("orchestrator".into())
    );
    assert_eq!(app.details.as_ref().unwrap().key, "orchestrator");
    assert_eq!(app.input.text, "");

    // The conversation tab is fed by the real daemon event/history response.
    assert_eq!(
        app.navigation.presentation().tab,
        antislop::lean::tui::WorkbenchTab::Conversation
    );
    app.key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
    draw(&mut app);
    assert!(app.navigation.presentation().zoomed);
    assert_eq!(app.details.as_ref().unwrap().selected_round, None);
    app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE));
    pump_until(
        &mut app,
        &updates,
        |app| app.navigation.presentation().terminal_owner.as_deref() == Some("orchestrator"),
        "pinned terminal owner",
    );
    assert!(app.navigation.presentation().pinned);
    assert_eq!(
        app.navigation.presentation().terminal_owner.as_deref(),
        Some("orchestrator")
    );
    assert!(app.navigation.presentation().split);

    // Attach the real fixture PTY after the app has rendered its actual panel.
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(
        app.navigation.focus,
        antislop::lean::tui::NavigationFocus::WorkbenchTerminal
    );
    app.key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
    draw(&mut app);
    let until = Instant::now() + Duration::from_secs(40);
    loop {
        if app
            .details
            .as_ref()
            .and_then(|d| d.native.get("orchestrator"))
            .and_then(|s| s.screen.as_ref())
            .is_some_and(|s| screen_text(s).contains("NATIVE PTY READY"))
        {
            break;
        }
        assert!(
            Instant::now() < until,
            "PTY fixture marker absent; details={:#?}; sessions={:?}",
            app.details,
            fixture.client.call(&Request::Session(
                antislop::lean::daemon::SessionRequest::Sessions {
                    conversation: conversation_a.clone()
                }
            ))
        );
        if let Ok(update) = updates.recv_timeout(Duration::from_millis(100)) {
            app.apply_update(update);
        }
        draw(&mut app);
    }
    assert_eq!(
        app.navigation.presentation().terminal_owner.as_deref(),
        Some("orchestrator")
    );
    assert!(app.navigation.presentation().native_input);
    let before = app.input.text.clone();
    app.key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));
    wait_for("fixture CLI key log", || {
        fixture.cli.with_extension("keys").exists()
    });
    assert_eq!(
        std::fs::read(fixture.cli.with_extension("keys")).unwrap(),
        b"k"
    );
    app.key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::CONTROL));
    assert!(!app.navigation.presentation().native_input);
    assert_eq!(
        app.input.text, before,
        "native keys must not enter the frontend draft"
    );
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!app.navigation.presentation().zoomed);
    assert!(app.navigation.presentation().split);
    pump_until(
        &mut app,
        &updates,
        |app| {
            app.view
                .transcript
                .iter()
                .any(|entry| entry.text.contains("navigation fixture response"))
        },
        "real fixture response",
    );
    let conversation_frame = draw(&mut app);
    assert!(
        conversation_frame.contains("navigation fixture response"),
        "Conversation tab must render the daemon response"
    );
    assert!(app.details.as_ref().unwrap().available_rounds.contains(&0));

    // Preserve a fresh draft in A, then create B through the app's live request handler.
    app.input.insert_str("alpha unsent draft");
    app.run_workflow(Command::NewConversation);
    pump_until(
        &mut app,
        &updates,
        |app| !app.conversation.is_empty() && app.conversation != conversation_a,
        "conversation B creation",
    );
    let conversation_b = app.conversation.clone();
    assert_ne!(conversation_a, conversation_b);
    app.input.insert_str("beta draft");
    app.key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
    wait_for("conversation picker", || app.picker.is_some());
    let limit = app.picker.as_ref().unwrap().conversations.len();
    let mut selected_a = false;
    for _ in 0..limit {
        let picker = app.picker.as_ref().unwrap();
        if picker
            .conversations
            .get(picker.selected)
            .is_some_and(|item| item.id == conversation_a)
        {
            selected_a = true;
            break;
        }
        app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    }
    assert!(
        selected_a,
        "conversation A must be reachable with picker navigation"
    );
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    wait_for("return to conversation A", || {
        app.conversation == conversation_a
    });
    assert_eq!(app.input.text, "alpha unsent draft");
    assert_eq!(app.navigation.screen, Screen::Workbench);
    assert_eq!(
        app.navigation.target,
        WorkbenchTarget::Agent("orchestrator".into())
    );
    assert_eq!(
        app.navigation.focus,
        antislop::lean::tui::NavigationFocus::WorkbenchTerminal
    );
    assert!(!app.navigation.presentation().native_input);

    let _ = fixture;
    assert!(app.conversation_drafts.contains_key(&conversation_b));
    assert_eq!(
        app.conversation_drafts
            .get(&conversation_b)
            .map(String::as_str),
        Some("beta draft")
    );
}

#[test]
fn real_delivery_graph_diff_and_reviewer_pty_share_task_round_owner() {
    let capture_dir = std::env::var_os("ANTISLOP_NAV_CAPTURE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("antislop-nav-capture-{}", std::process::id()))
        });
    std::fs::create_dir_all(&capture_dir).unwrap();
    let raw_log = capture_dir.join("native-key-bytes.bin");
    assert!(
        !raw_log.exists(),
        "capture directory must be a fresh run directory: {}",
        capture_dir.display()
    );
    let fixture = NavigationFixture::start_delivery(&capture_dir);
    let conversation = fixture
        .client
        .call(&Request::Control(ControlRequest::Start))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    fixture
        .client
        .call(&Request::Conversation(ConversationRequest::Send {
            conversation: conversation.clone(),
            operation_id: None,
            text: "Create the navigation artifact through one reviewed implementation task.".into(),
            to: None,
            images: vec![],
        }))
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(120);
    let mut confirmed = false;
    let mut plan_id = None;
    while plan_id.is_none() {
        assert!(
            Instant::now() < deadline,
            "real plan proposal did not arrive"
        );
        let event_reply = fixture
            .client
            .call(&Request::History(HistoryRequest::Events {
                conversation: conversation.clone(),
                after: 0,
                wait_ms: 0,
            }))
            .unwrap();
        let events = event_reply["events"].as_array().unwrap();
        if !confirmed
            && let Some(question) = events
                .iter()
                .rev()
                .find(|event| event["kind"] == "scope_confirmation_requested")
                .and_then(|event| event["payload"]["question"]["id"].as_str())
        {
            fixture
                .client
                .call(&Request::Conversation(ConversationRequest::Send {
                    conversation: conversation.clone(),
                    operation_id: None,
                    text: format!("Interview answer ({question})\nAnswer: Confirm scope"),
                    to: None,
                    images: vec![],
                }))
                .unwrap();
            confirmed = true;
        }
        let plans = fixture
            .client
            .call(&Request::Conversation(ConversationRequest::Plans {
                conversation: conversation.clone(),
                offset: None,
                limit: None,
            }))
            .unwrap();
        plan_id = plans["active_plan_id"].as_str().map(str::to_owned);
        if plan_id.is_none() {
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    let plan_id = plan_id.unwrap();
    let mut accepted = false;
    while !accepted {
        assert!(
            Instant::now() < deadline,
            "real reviewer did not approve the plan"
        );
        accepted = fixture
            .client
            .call(&Request::Conversation(ConversationRequest::Resume {
                conversation: conversation.clone(),
                operation_id: None,
                plan: plan_id.clone(),
                revision_token: None,
            }))
            .is_ok();
        if !accepted {
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    let diff = fixture
        .client
        .call(&Request::Conversation(ConversationRequest::Diff {
            conversation: conversation.clone(),
            plan: plan_id.clone(),
            id: "t1".into(),
        }));
    let mut diff = diff.ok();
    while diff
        .as_ref()
        .is_none_or(|diff| !diff.to_string().contains("from real worker"))
    {
        assert!(
            Instant::now() < deadline,
            "worker did not produce the expected live diff: {diff:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
        diff = fixture
            .client
            .call(&Request::Conversation(ConversationRequest::Diff {
                conversation: conversation.clone(),
                plan: plan_id.clone(),
                id: "t1".into(),
            }))
            .ok();
    }
    let diff = diff.unwrap();
    assert!(diff.to_string().contains("navigation-artifact.txt"));
    assert!(
        diff.to_string().contains("from real worker"),
        "real daemon Diff must contain the written worker output: {diff}"
    );
    wait_for("task reviewer to own the live PTY", || {
        fixture
            .client
            .call(&Request::Conversation(ConversationRequest::Task {
                conversation: Some(conversation.clone()),
                plan: plan_id.clone(),
                id: "t1".into(),
            }))
            .is_ok_and(|task| task["status"] == "reviewing")
    });

    let (updates, rx) = update_channel(512);
    let mut app = App::test_app_with_client_and_updates(fixture.client.clone(), updates);
    app.project = fixture.project.to_string_lossy().into_owned();
    app.open(conversation.clone());
    pump_until(
        &mut app,
        &rx,
        |app| {
            app.view.tasks.iter().any(|task| task.id == "t1")
                && app.view.agents.contains_key("worker:t1")
        },
        "real worker graph node",
    );
    draw(&mut app);
    for _ in 0..3 {
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    }
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.focus, Focus::Graph);
    app.key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
    assert!(app.picker.is_some(), "graph agent picker must open by key");
    for character in "t1".chars() {
        app.key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
    }
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(
        app.selected, "reviewer:t1",
        "key-filtered graph picker must select the delivered task"
    );
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.navigation.screen, Screen::Workbench);
    assert_eq!(
        app.navigation.target,
        WorkbenchTarget::Agent("reviewer:t1".into())
    );
    assert_eq!(
        app.navigation.presentation().tab,
        antislop::lean::tui::WorkbenchTab::Diff
    );
    assert_eq!(app.details.as_ref().unwrap().key, "reviewer:t1");
    pump_until(
        &mut app,
        &rx,
        |app| {
            app.details
                .as_ref()
                .is_some_and(|details| !details.available_rounds.is_empty())
        },
        "worker run history",
    );
    let rounds = app.details.as_ref().unwrap().available_rounds.clone();
    assert!(
        !rounds.is_empty(),
        "the real writer task must expose run history"
    );
    assert_eq!(app.details.as_ref().unwrap().task_id(), Some("t1"));
    app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE));
    assert!(app.navigation.presentation().pinned);
    let owner = app
        .navigation
        .presentation()
        .terminal_owner
        .clone()
        .unwrap();
    assert!(
        owner.starts_with("reviewer:"),
        "unexpected PTY owner {owner}"
    );
    app.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(
        app.navigation.focus,
        antislop::lean::tui::NavigationFocus::WorkbenchTerminal
    );
    draw(&mut app);
    app.key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
    let until = Instant::now() + Duration::from_secs(40);
    loop {
        if app
            .details
            .as_ref()
            .and_then(|d| d.native.get(&owner))
            .and_then(|snapshot| snapshot.screen.as_ref())
            .is_some_and(|screen| screen_text(screen).contains("NATIVE PTY READY"))
        {
            break;
        }
        assert!(
            Instant::now() < until,
            "reviewer PTY did not attach; owner={owner}; details={:#?}",
            app.details
        );
        if let Ok(update) = rx.recv_timeout(Duration::from_millis(100)) {
            app.apply_update(update);
        }
        draw(&mut app);
    }
    assert!(
        app.navigation.presentation().native_input,
        "native focus did not survive PTY attachment: owner={owner}; presentation={:#?}; details={:#?}",
        app.navigation.presentation(),
        app.details
    );
    assert_eq!(app.conversation, conversation);
    assert_eq!(app.details.as_ref().unwrap().available_rounds, rounds);
    let frame = draw(&mut app);
    assert!(
        frame.contains("Diff"),
        "Diff tab must remain visible beside PTY"
    );
    assert!(frame.contains("navigation-artifact.txt"));
    assert!(
        frame.contains("NATIVE PTY READY"),
        "one rendered Workbench frame must show live PTY and Diff together"
    );
    assert!(app.details.as_ref().unwrap().native[&owner].error.is_none());
    let capture_marker = PathBuf::from(format!("{}.capture_native_keys", fixture.cli.display()));
    std::fs::write(&capture_marker, "baseline k").unwrap();
    app.key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));
    wait_for("raw PTY capture of ordinary native key", || {
        std::fs::metadata(&raw_log).is_ok_and(|metadata| metadata.len() > 0)
    });
    std::fs::remove_file(&capture_marker).unwrap();
    assert_eq!(std::fs::read(&raw_log).unwrap(), b"k");
    drop(app);

    let sessions = fixture
        .client
        .call(&Request::Session(SessionRequest::Sessions {
            conversation: conversation.clone(),
        }))
        .unwrap();
    let session = sessions
        .as_array()
        .unwrap()
        .iter()
        .find(|session| session["role"] == owner && session["plan_id"] == plan_id)
        .expect("the PTY's reviewer session belongs to the selected task plan");
    let session_id = session["turn_id"]
        .as_str()
        .expect("reviewer PTY has a durable turn identity")
        .to_owned();
    let keys = [
        (
            "F5",
            KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE),
            false,
        ),
        (
            "F6",
            KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE),
            false,
        ),
        (
            "Ctrl+P",
            KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL),
            false,
        ),
        (
            "Ctrl+X",
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
            false,
        ),
        (
            "t",
            KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE),
            false,
        ),
        (
            "i",
            KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
            false,
        ),
        (
            "Ctrl+5",
            KeyEvent::new(KeyCode::Char('5'), KeyModifiers::CONTROL),
            false,
        ),
        (
            "Ctrl+]",
            KeyEvent::new(KeyCode::Char(']'), KeyModifiers::CONTROL),
            true,
        ),
    ];
    let mut failures = Vec::new();
    for (name, key, exits_native) in keys {
        let (mut case_app, _case_updates) =
            open_reviewer_workbench(&fixture, &conversation, "t1", &owner, &rounds);
        let session_now = fixture
            .client
            .call(&Request::Session(SessionRequest::Sessions {
                conversation: conversation.clone(),
            }))
            .unwrap();
        assert!(session_now.as_array().unwrap().iter().any(|candidate| {
            candidate["role"] == owner
                && candidate["plan_id"] == plan_id
                && candidate["turn_id"] == session_id
        }));
        assert!(case_app.navigation.presentation().native_input);
        let native_before = case_app.navigation.presentation().native_input;
        let focus_before = format!("{:?}", case_app.navigation.focus);
        let target_before = format!("{:?}", case_app.navigation.target);
        let tab_before = format!("{:?}", case_app.navigation.presentation().tab);
        let pinned_owner = case_app.navigation.presentation().terminal_owner.clone();
        let selected_round = case_app
            .details
            .as_ref()
            .and_then(|details| details.selected_round);
        let terminal_handle = case_app
            .details
            .as_ref()
            .and_then(|details| details.native.get(&owner))
            .and_then(|view| view.screen.as_ref())
            .map(|screen| screen.handle.clone())
            .expect("fresh App has the selected reviewer's actual PTY screen");
        let before_bytes = std::fs::read(&raw_log).unwrap_or_default();
        let capture_marker =
            PathBuf::from(format!("{}.capture_native_keys", fixture.cli.display()));
        std::fs::write(&capture_marker, name).unwrap();
        case_app.key(key);
        let until = Instant::now() + Duration::from_secs(2);
        loop {
            let after_len =
                std::fs::metadata(&raw_log).map_or(0, |metadata| metadata.len() as usize);
            let mode_matches_expectation =
                case_app.navigation.presentation().native_input != exits_native;
            if after_len > before_bytes.len()
                || !mode_matches_expectation
                || Instant::now() >= until
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        std::fs::remove_file(&capture_marker).unwrap();
        let after_bytes = std::fs::read(&raw_log).unwrap_or_default();
        let received = after_bytes[before_bytes.len().min(after_bytes.len())..].to_vec();
        let native_after = case_app.navigation.presentation().native_input;
        let expected = if exits_native {
            !native_after && received.is_empty()
        } else {
            native_after && !received.is_empty()
        };
        let observation = serde_json::json!({
            "key": name,
            "key_event": format!("{key:?}"),
            "conversation": conversation,
            "plan": plan_id,
            "task": "t1",
            "role": owner,
            "terminal_turn": session_id,
            "rounds": rounds,
            "target_before": target_before,
            "tab_before": tab_before,
            "focus_before": focus_before,
            "focus_after": format!("{:?}", case_app.navigation.focus),
            "pinned_owner": pinned_owner,
            "terminal_handle": terminal_handle,
            "selected_round": selected_round,
            "native_before": native_before,
            "native_after": native_after,
            "split_after": case_app.navigation.presentation().split,
            "zoomed_after": case_app.navigation.presentation().zoomed,
            "pty_received_hex": received.iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
            "passes_contract": expected,
        });
        use std::io::Write;
        let mut report = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(capture_dir.join("native-key-observations.jsonl"))
            .unwrap();
        writeln!(report, "{observation}").unwrap();
        if !expected {
            failures.push(observation);
        }
    }
    std::fs::write(
        format!("{}.release_native_terminal", fixture.cli.display()),
        b"release",
    )
    .unwrap();
    let (mut release_app, _release_updates) =
        open_reviewer_workbench(&fixture, &conversation, "t1", &owner, &rounds);
    release_app.key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));
    assert!(release_app.navigation.presentation().native_input);
    std::fs::remove_file(fixture.cli.with_extension("dialog.after_work")).unwrap();

    loop {
        assert!(
            Instant::now() < deadline,
            "worker task was not reviewed and integrated"
        );
        let task = fixture
            .client
            .call(&Request::Conversation(ConversationRequest::Task {
                conversation: Some(conversation.clone()),
                plan: plan_id.clone(),
                id: "t1".into(),
            }))
            .unwrap();
        if task["status"] == "approved" {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let sessions = fixture
        .client
        .call(&Request::Session(SessionRequest::Sessions {
            conversation: conversation.clone(),
        }))
        .unwrap();
    assert!(
        sessions
            .as_array()
            .unwrap()
            .iter()
            .any(|session| session["role"] == owner && session["plan_id"] == plan_id)
    );
    assert!(
        failures.is_empty(),
        "native routing violated the requirement for the recorded fresh App::key cases: {failures:?}; raw PTY bytes: {}",
        raw_log.display()
    );
}

/// Run explicitly in a graphical terminal: `./cargo.sh test --test lean_tui_navigation_sequence -- --ignored --nocapture`.
/// It is deliberately ignored because normal test runs must never replace the host clipboard.
#[test]
#[ignore = "touches the host OS clipboard; run explicitly in a graphical terminal"]
fn host_clipboard_roundtrip_through_tui_copy_and_paste_handlers() {
    #[cfg(not(target_os = "macos"))]
    panic!("host clipboard snapshot/restore adapter is implemented only for macOS");

    #[cfg(target_os = "macos")]
    {
        let _tty = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty")
            .unwrap_or_else(|error| {
                panic!("cannot access controlling graphical terminal /dev/tty: {error}")
            });
        assert!(
            std::io::stdin().is_terminal() && std::io::stdout().is_terminal(),
            "stdin and stdout must remain attached to the foreground graphical terminal; do not pipe or redirect this test"
        );
        let before = macos_clipboard("snapshot", None);
        let mut restore = ClipboardRestore(Some(before.clone()));
        let fixture = NavigationFixture::start();
        let conversation = fixture
            .client
            .call(&Request::Control(ControlRequest::Start))
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        fixture
            .client
            .call(&Request::Conversation(ConversationRequest::Send {
                conversation: conversation.clone(),
                operation_id: None,
                text: "Display the exact copy marker `navigation fixture response`. Host C2 steps: select that exact text with the TUI mouse, press Ctrl+Y, then immediately press Cmd+V in the composer and Enter. The ignored test separately verifies the OS clipboard copy and that the app submitted the pasted marker.".into(),
                to: None,
                images: vec![],
            }))
            .unwrap();
        let events = fixture
            .client
            .call(&Request::History(HistoryRequest::Events {
                conversation: conversation.clone(),
                after: 0,
                wait_ms: 0,
            }))
            .unwrap();
        let after = events["events"]
            .as_array()
            .and_then(|events| events.last())
            .and_then(|event| event["seq"].as_u64())
            .expect("fixture conversation has no sequenced events yet");
        wait_for("clipboard fixture response", || {
            fixture
                .client
                .call(&Request::History(HistoryRequest::Events {
                    conversation: conversation.clone(),
                    after: 0,
                    wait_ms: 0,
                }))
                .is_ok_and(|reply| reply.to_string().contains("navigation fixture response"))
        });
        let marker = "navigation fixture response";
        println!(
            "HOST C2: dedicated terminal required. In the Antislop chat below, select the exact visible text {marker:?} with the TUI mouse, press Ctrl+Y, then immediately press Cmd+V in the composer and Enter."
        );
        let mut command = ProcessCommand::new(env!("CARGO_BIN_EXE_antislop"));
        command
            .args([
                "chat",
                "--state",
                fixture.state.to_str().unwrap(),
                "--conversation",
                &conversation,
            ])
            .current_dir(&fixture.project)
            .env(
                "TERM",
                std::env::var("TERM").unwrap_or_else(|_| "xterm-256color".into()),
            )
            .env("ANTISLOP_CONFIG_DIR", fixture.project.join("../g"))
            .env("ANTISLOP_STATE_ROOT", fixture.project.join("../runtime"))
            // Inherit all three standard streams so chat owns the same controlling TTY
            // that receives the user's real terminal paste.
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        let tui = command.spawn().unwrap_or_else(|error| {
            panic!("cannot start foreground chat on controlling terminal: {error}")
        });
        let mut tui_guard = ChildGuard(Some(tui));

        wait_for_macos_clipboard_text(marker);
        let submit_deadline = Instant::now() + Duration::from_secs(180);
        loop {
            assert!(
                Instant::now() < submit_deadline,
                "timed out waiting for the user's real Cmd+V and Enter in the foreground chat"
            );
            if tui_guard
                .0
                .as_mut()
                .is_some_and(|child| child.try_wait().is_ok_and(|status| status.is_some()))
            {
                panic!("foreground chat exited before the daemon received the pasted marker");
            }
            let received = fixture
                .client
                .call(&Request::History(HistoryRequest::Events {
                    conversation: conversation.clone(),
                    after,
                    wait_ms: 0,
                }))
                .is_ok_and(|reply| {
                    reply["events"].as_array().is_some_and(|events| {
                        events.iter().any(|event| {
                            event["kind"] == "message"
                                && event["payload"]["author"] == "user"
                                && event["payload"]["text"] == marker
                        })
                    })
                });
            if received {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        let child = tui_guard.0.as_mut().unwrap();
        let _ = child.kill();
        let _ = child.wait();
        tui_guard.0 = None;
        restore.restore();
        println!(
            "HOST C2: received exact pasted payload through the app and daemon; clipboard snapshot restored and verified."
        );
    }
}

#[cfg(target_os = "macos")]
struct ChildGuard(Option<std::process::Child>);

#[cfg(target_os = "macos")]
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(target_os = "macos")]
struct ClipboardRestore(Option<serde_json::Value>);

#[cfg(target_os = "macos")]
impl ClipboardRestore {
    fn restore(&mut self) {
        let Some(snapshot) = self.0.as_ref() else {
            return;
        };
        macos_clipboard("restore", Some(snapshot));
        let restored = macos_clipboard("snapshot", None);
        assert_eq!(
            &restored, snapshot,
            "host clipboard restore did not preserve all items, types, and bytes"
        );
        self.0 = None;
    }
}

#[cfg(target_os = "macos")]
impl Drop for ClipboardRestore {
    fn drop(&mut self) {
        if let Err(error) =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.restore()))
        {
            eprintln!("HOST C2: clipboard restore/verification failed during cleanup: {error:?}");
        }
    }
}

#[cfg(target_os = "macos")]
fn macos_clipboard(action: &str, snapshot: Option<&serde_json::Value>) -> serde_json::Value {
    const SCRIPT: &str = r#"
import AppKit
import Foundation
let pasteboard = NSPasteboard.general
if CommandLine.arguments.last == "snapshot" {
    var items: [[String: String]] = []
    for item in pasteboard.pasteboardItems ?? [] {
        var values: [String: String] = [:]
        for type in item.types {
            guard let data = item.data(forType: type) else {
                fputs("cannot snapshot pasteboard item format \(type.rawValue)\n", stderr)
                exit(21)
            }
            values[type.rawValue] = data.base64EncodedString()
        }
        items.append(values)
    }
    let data = try JSONSerialization.data(withJSONObject: items, options: [.sortedKeys])
    FileHandle.standardOutput.write(data)
} else {
    let input = FileHandle.standardInput.readDataToEndOfFile()
    let object = try JSONSerialization.jsonObject(with: input) as! [[String: String]]
    pasteboard.clearContents()
    let items = object.map { values -> NSPasteboardItem in
        let item = NSPasteboardItem()
        for (type, encoded) in values {
            guard let data = Data(base64Encoded: encoded) else { fatalError("invalid saved base64") }
            item.setData(data, forType: NSPasteboard.PasteboardType(type))
        }
        return item
    }
    if !items.isEmpty && !pasteboard.writeObjects(items) { fatalError("pasteboard restore rejected saved items") }
}
"#;
    let mut child = ProcessCommand::new("swift")
        .args(["-e", SCRIPT, action])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("cannot launch Swift NSPasteboard adapter: {error}"));
    if let Some(snapshot) = snapshot {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(snapshot.to_string().as_bytes())
            .unwrap();
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "Swift NSPasteboard {action} failed ({}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    );
    if action == "snapshot" {
        serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|error| panic!("NSPasteboard snapshot was not valid JSON: {error}"))
    } else {
        serde_json::Value::Null
    }
}

#[cfg(target_os = "macos")]
fn wait_for_macos_clipboard_text(expected: &str) -> String {
    let until = Instant::now() + Duration::from_secs(120);
    loop {
        let snapshot = macos_clipboard("snapshot", None);
        let text = snapshot
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| item.get("public.utf8-plain-text"))
            .filter_map(serde_json::Value::as_str)
            .filter_map(|encoded| {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD
                    .decode(encoded)
                    .ok()
                    .and_then(|bytes| String::from_utf8(bytes).ok())
            })
            .find(|text| text == expected);
        if let Some(text) = text {
            return text;
        }
        assert!(
            Instant::now() < until,
            "timed out waiting for Ctrl+Y copy to place {expected:?} on the host OS clipboard"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

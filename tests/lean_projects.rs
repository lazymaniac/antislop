//! Two projects open in one screen: each has its own daemon, state and
//! configuration, and the screen switches between them without mixing them up.

mod common;

use antislop::lean::daemon::{Client, Request};
use antislop::lean::tui::workspace::ProjectConn;
use antislop::lean::tui::{App, UpdateReceiver, update_channel};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn write_executable(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    common::interactive(path);
}

/// A project directory with its own configuration and fake CLIs.
fn project(root: &Path, name: &str) -> PathBuf {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    write_executable(
        &dir.join("codex"),
        &format!(
            r#"#!/usr/bin/env python3
import json, sys
print(json.dumps({{"type": "thread.started", "thread_id": "orch"}}))
print(json.dumps({{"type": "item.completed", "item": {{"type": "agent_message", "text": "{name} answers"}}}}))
print(json.dumps({{"type": "turn.completed", "usage": {{}}}}))
"#
        ),
    );
    write_executable(
        &dir.join("agy"),
        "#!/usr/bin/env python3\nimport json\nprint(json.dumps({'event':'result','result':{'status':'SUCCESS','response':'ok','usage':{}}}))\n",
    );
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/examples/antislop.toml"
    ))
    .unwrap()
    .replace(
        "binary = \"codex\"",
        &format!("binary = {:?}", dir.join("codex").to_string_lossy()),
    )
    .replace(
        "binary = \"agy\"",
        &format!("binary = {:?}", dir.join("agy").to_string_lossy()),
    );
    std::fs::write(dir.join("antislop.toml"), text).unwrap();
    dir.canonicalize().unwrap()
}

struct Daemon {
    child: std::process::Child,
    conn: ProjectConn,
}

impl Daemon {
    fn start(project: &Path) -> Self {
        common::runtime_environment(project);
        let state = antislop::lean::paths::project_state(project).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        std::fs::set_permissions(&state, std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .unwrap();
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
            .current_dir(project)
            .arg("__core")
            .arg(project)
            .arg(project.join("antislop.toml"))
            .arg(&state)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let client = Client::at(&state);
        let deadline = Instant::now() + Duration::from_secs(120);
        while client
            .call(&Request::Control(
                antislop::lean::daemon::ControlRequest::Identity,
            ))
            .is_err()
        {
            if let Some(status) = child.try_wait().unwrap() {
                panic!("fixture daemon exited during startup: {status}");
            }
            assert!(Instant::now() < deadline, "daemon did not start");
            std::thread::sleep(Duration::from_millis(25));
        }
        let client = Client::connect(
            project,
            &project.join("antislop.toml"),
            &state,
            Path::new(env!("CARGO_BIN_EXE_antislop")),
        )
        .unwrap();
        Self {
            child,
            conn: ProjectConn::new(project.to_path_buf(), client),
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.conn.client.call(&Request::Control(
            antislop::lean::daemon::ControlRequest::Shutdown,
        ));
        std::thread::sleep(Duration::from_millis(100));
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn pump(app: &mut App, rx: &UpdateReceiver, until: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(120);
    while !until(app) {
        assert!(Instant::now() < deadline, "the screen never got there");
        if let Ok(update) = rx.recv_timeout(Duration::from_millis(50)) {
            app.apply_update(update);
        }
    }
}

fn conversations(conn: &ProjectConn) -> Vec<Value> {
    conn.client
        .call(&Request::Control(
            antislop::lean::daemon::ControlRequest::Conversations,
        ))
        .unwrap()
        .as_array()
        .cloned()
        .unwrap_or_default()
}

#[test]
fn switching_projects_keeps_each_ones_conversation_draft_and_daemon() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let (path_a, path_b) = (project(&root, "alpha"), project(&root, "beta"));
    let (a, b) = (Daemon::start(&path_a), Daemon::start(&path_b));

    let (tx, rx) = update_channel(256);
    let mut app = App::test_app_with_client_and_updates(a.conn.client.clone(), tx);
    app.project = "alpha".into();
    let first = app.register_project(a.conn.clone());
    app.switch_project(first, None);
    let conversation_a = app.conversation.clone();
    assert!(!conversation_a.is_empty());

    // Words typed but not sent belong to the conversation they were typed in.
    app.input = antislop::lean::tui::InputBuffer::with_text("draft in alpha");
    let second = app.register_project(b.conn.clone());
    assert_eq!((first, second), (0, 1));
    assert_eq!(app.register_project(b.conn.clone()), 1, "registered once");
    app.switch_project(second, None);
    assert_eq!(app.project, "beta");
    assert_ne!(app.conversation, conversation_a);
    assert_eq!(app.input.text, "", "beta starts with an empty draft");

    // Messages go to the daemon of the project on screen.
    app.input = antislop::lean::tui::InputBuffer::with_text("hello beta");
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    pump(&mut app, &rx, |app| {
        app.view
            .transcript
            .iter()
            .any(|entry| entry.text.contains("beta answers"))
            || app
                .view
                .cards
                .iter()
                .any(|card| format!("{card:?}").contains("beta answers"))
    });
    assert_eq!(conversations(&a.conn).len(), 1, "alpha was not asked");
    let beta_conversations = conversations(&b.conn);
    assert_eq!(beta_conversations.len(), 1);
    assert_eq!(
        beta_conversations[0]["project"],
        path_b.to_string_lossy().as_ref()
    );

    // Back in alpha the draft is where it was left, and the daemons stay apart.
    app.switch_project(first, None);
    assert_eq!(app.project, "alpha");
    assert_eq!(app.conversation, conversation_a);
    assert_eq!(app.input.text, "draft in alpha");

    // The header tells of the other project; the conversation picker lists it.
    pump(&mut app, &rx, |app| {
        app.projects[1]
            .conversations
            .iter()
            .any(|c| c["status"] == "idle")
    });
    let header: String = app
        .header_line(120)
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect();
    assert!(header.contains("alpha (1 more"), "{header}");
    app.open_conversation_picker();
    let picker = app.picker.as_ref().unwrap();
    assert!(
        picker
            .conversations
            .iter()
            .any(|c| c.project == path_b.to_string_lossy().as_ref())
    );

    // Choosing beta's conversation there switches the project too.
    let beta_id = beta_conversations[0]["id"].as_str().unwrap().to_owned();
    app.picker = None;
    app.switch_project(second, Some(beta_id.clone()));
    assert_eq!(
        (app.project.as_str(), app.conversation.as_str()),
        ("beta", beta_id.as_str())
    );
}

#[test]
fn opening_a_project_uses_global_state_without_consent_or_local_artifacts() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let path_a = project(&root, "alpha");
    let a = Daemon::start(&path_a);
    let next = project(&root, "next");
    let (tx, rx) = update_channel(256);
    let mut app = App::test_app_with_client_and_updates(a.conn.client.clone(), tx);
    app.register_project(a.conn.clone());
    app.open_project(&next.to_string_lossy());
    pump(&mut app, &rx, |app| app.project == "next");
    let selected = app.projects[app.active_project]
        .client
        .state_dir()
        .to_path_buf();
    let stopped = Client::stop(&selected, true);
    assert!(stopped.unwrap());
    assert_eq!(app.projects.len(), 2);
    assert_eq!(
        selected,
        antislop::lean::paths::project_state(&next).unwrap()
    );
    assert!(selected.join("core.db").exists());
    assert!(!selected.starts_with(&next));
    assert!(!next.join(".antislop").exists());
    app.open_project(&path_a.to_string_lossy());
    assert_eq!(app.projects.len(), 2);
}

#[test]
fn configuration_precedence_and_errors_without_creating_state() {
    // Resolve in an isolated child so concurrent tests never mutate process env.
    if let Some(project) = std::env::var_os("ANTISLOP_TEST_RESOLVE_PROJECT") {
        let selected = antislop::lean::cli::resolve_config(Path::new(&project), None).unwrap();
        println!("SELECTED={}", selected.display());
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    let global = root.path().join("global");
    std::fs::create_dir(&project).unwrap();
    std::fs::create_dir(&global).unwrap();
    let local_file = project.join("antislop.toml");
    let global_file = global.join("antislop.toml");
    let valid = include_str!("../examples/antislop.toml");
    let runtime = root.path().join("runtime");
    let state = runtime
        .join("projects")
        .join(antislop::lean::paths::project_identity(&project).unwrap());
    std::fs::create_dir_all(&state).unwrap();
    let shadow = state.join("antislop.toml");
    std::fs::write(&shadow, "invalid stale runtime configuration").unwrap();
    let run = |args: &[&str]| {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
            .current_dir(&project)
            .env("ANTISLOP_CONFIG_DIR", &global)
            .env("ANTISLOP_STATE_ROOT", &runtime)
            .args(args)
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(!project.join(".antislop").exists());
        String::from_utf8(output.stderr).unwrap()
    };
    let assert_selected = |expected: &Path| {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "configuration_precedence_and_errors_without_creating_state",
                "--nocapture",
            ])
            .env("ANTISLOP_TEST_RESOLVE_PROJECT", &project)
            .env("ANTISLOP_CONFIG_DIR", &global)
            .env("ANTISLOP_STATE_ROOT", &runtime)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(String::from_utf8(output.stdout).unwrap().contains(&format!(
            "SELECTED={}",
            expected.canonicalize().unwrap().display()
        )));
    };
    // A stale runtime file is never a candidate, even without local/user files.
    let error = run(&["chat", "hello"]);
    assert!(error.contains("no configuration at"), "{error}");
    assert!(error.contains(local_file.to_str().unwrap()), "{error}");
    assert!(error.contains(global_file.to_str().unwrap()), "{error}");
    assert!(!global_file.exists());
    // Only project, only global, then both: assert the actual canonical selection.
    std::fs::write(&local_file, valid).unwrap();
    assert_selected(&local_file);
    std::fs::remove_file(&local_file).unwrap();
    std::fs::write(&global_file, valid).unwrap();
    assert_selected(&global_file);
    std::fs::write(&local_file, valid).unwrap();
    assert_selected(&local_file);
    std::fs::write(&global_file, "invalid TOML").unwrap();
    // A broken project file must not fall back to the valid global file.
    std::fs::write(&global_file, valid).unwrap();
    std::fs::write(&local_file, "invalid TOML").unwrap();
    assert_selected(&local_file);
    assert!(run(&["chat", "hello"]).contains("invalid TOML configuration"));
    // An explicit file wins even over the broken project file.
    std::fs::write(project.join("custom.toml"), valid).unwrap();
    assert_eq!(
        antislop::lean::cli::resolve_config(&project, Some(Path::new("custom.toml"))).unwrap(),
        project.join("custom.toml").canonicalize().unwrap()
    );
    let error = run(&["chat", "--config", "absent.toml", "hello"]);
    assert!(
        error.contains(project.join("absent.toml").to_str().unwrap()),
        "{error}"
    );
    assert!(error.contains("No such file"), "{error}");
    let link = project.join("linked.toml");
    std::os::unix::fs::symlink(project.join("custom.toml"), &link).unwrap();
    assert_eq!(
        antislop::lean::cli::resolve_config(&project, Some(&link)).unwrap(),
        project.join("custom.toml").canonicalize().unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(&shadow).unwrap(),
        "invalid stale runtime configuration"
    );
    assert!(!state.join("core.db").exists());
}

#[test]
fn worktree_cleanup_keeps_history_and_uses_the_remaining_checkout_config() {
    let root = tempfile::tempdir().unwrap();
    let primary = project(root.path(), "primary");
    common::runtime_environment(&primary);
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(&primary)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "--quiet", "-b", "main"]);
    git(&[
        "-c",
        "user.name=Fixture",
        "-c",
        "user.email=fixture@example.test",
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "--quiet",
        "--allow-empty",
        "-m",
        "fixture",
    ]);
    let checkout = root.path().join("checkout");
    git(&[
        "worktree",
        "add",
        "--quiet",
        "--detach",
        checkout.to_str().unwrap(),
    ]);
    std::fs::copy(
        primary.join("antislop.toml"),
        checkout.join("antislop.toml"),
    )
    .unwrap();
    let daemon = Daemon::start(&checkout);
    let state = daemon.conn.client.state_dir().to_path_buf();
    let conversation = daemon
        .conn
        .client
        .call(&Request::Control(
            antislop::lean::daemon::ControlRequest::Start,
        ))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let before = daemon
        .conn
        .client
        .call(&Request::Control(
            antislop::lean::daemon::ControlRequest::Identity,
        ))
        .unwrap();
    git(&["worktree", "remove", "--force", checkout.to_str().unwrap()]);
    assert_eq!(
        state,
        antislop::lean::paths::project_state(&primary).unwrap()
    );
    let client = Client::connect(
        &primary,
        &primary.join("antislop.toml"),
        &state,
        Path::new(env!("CARGO_BIN_EXE_antislop")),
    )
    .unwrap();
    let attached = client
        .call(&Request::Control(
            antislop::lean::daemon::ControlRequest::Identity,
        ))
        .unwrap();
    assert_ne!(
        attached["pid"], before["pid"],
        "the idle daemon adopts the remaining checkout source"
    );
    assert_eq!(
        attached["config"],
        primary.join("antislop.toml").to_string_lossy().as_ref()
    );
    let read_history = |client: &Client| {
        client
            .call(&Request::History(
                antislop::lean::daemon::HistoryRequest::Events {
                    conversation: conversation.clone(),
                    after: 0,
                    wait_ms: 0,
                },
            ))
            .unwrap()
    };
    assert!(
        !read_history(&client)["events"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    drop(daemon);
    let restarted = Client::connect(
        &primary,
        &primary.join("antislop.toml"),
        &state,
        Path::new(env!("CARGO_BIN_EXE_antislop")),
    )
    .unwrap();
    let history = read_history(&restarted);
    Client::stop(&state, true).unwrap();
    assert!(!history["events"].as_array().unwrap().is_empty());
    assert!(state.join("core.db").exists());
    assert!(!state.join("antislop.toml").exists());
    assert!(!primary.join(".antislop").exists());
}

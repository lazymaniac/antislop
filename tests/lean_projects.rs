//! Two projects open in one screen: each has its own daemon, state and
//! configuration, and the screen switches between them without mixing them up.

mod common;

use antislop::lean::daemon::{Client, Request};
use antislop::lean::tui::workspace::{self, ProjectConn};
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
        common::trust(project);
        let state = project.join(".antislop");
        std::fs::create_dir_all(&state).unwrap();
        std::fs::set_permissions(&state, std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .unwrap();
        let child = std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
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
            assert!(Instant::now() < deadline, "daemon did not start");
            std::thread::sleep(Duration::from_millis(25));
        }
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
            .any(|c| c.title.starts_with("beta ·"))
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
fn an_untrusted_directory_is_asked_about_before_anything_is_created_there() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let path_a = project(&root, "alpha");
    let a = Daemon::start(&path_a);
    let stranger = project(&root, "stranger");

    let (tx, rx) = update_channel(256);
    let mut app = App::test_app_with_client_and_updates(a.conn.client.clone(), tx);
    app.register_project(a.conn.clone());
    app.open_project(&stranger.to_string_lossy());
    pump(&mut app, &rx, |app| app.project_trust.is_some());
    assert_eq!(app.project_trust.as_deref(), Some(stranger.as_path()));
    assert!(!stranger.join(".antislop").exists(), "nothing created yet");

    // Declining closes the question and changes nothing.
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.project_trust.is_none());
    assert_eq!(app.projects.len(), 1);
    assert!(!stranger.join(".antislop").exists());

    // The same checks the screen makes, without starting a daemon.
    let registry = antislop::lean::trust::Registry::from_env().unwrap();
    assert!(matches!(
        workspace::connect(&stranger, &registry),
        Err(workspace::ConnectError::NotTrusted(_))
    ));
    // A project that is open is switched to, not opened twice.
    app.open_project(&path_a.to_string_lossy());
    assert_eq!(app.projects.len(), 1);
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
    let run = |args: &[&str]| {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
            .current_dir(&project)
            .env("ANTISLOP_CONFIG_DIR", &global)
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
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(String::from_utf8(output.stdout).unwrap().contains(&format!(
            "SELECTED={}",
            expected.canonicalize().unwrap().display()
        )));
    };
    // Neither file: report both candidates, without creating either file.
    let error = run(&["chat", "hello"]);
    assert!(error.contains("no configuration at"), "{error}");
    assert!(error.contains(local_file.to_str().unwrap()), "{error}");
    assert!(error.contains(global_file.to_str().unwrap()), "{error}");
    assert!(!global_file.exists());
    // Only project, only global, then both: assert the actual canonical selection.
    std::fs::write(&local_file, valid).unwrap();
    assert_selected(&local_file);
    assert!(run(&["chat", "hello"]).contains("not trusted"));
    std::fs::remove_file(&local_file).unwrap();
    std::fs::write(&global_file, valid).unwrap();
    assert_selected(&global_file);
    assert!(run(&["chat", "hello"]).contains("not trusted"));
    std::fs::write(&local_file, valid).unwrap();
    assert_selected(&local_file);
    assert!(run(&["chat", "hello"]).contains("not trusted"));
    std::fs::write(&global_file, "invalid TOML").unwrap();
    assert!(run(&["chat", "hello"]).contains("not trusted"));
    // A broken project file must not fall back to the valid global file.
    std::fs::write(&global_file, valid).unwrap();
    std::fs::write(&local_file, "invalid TOML").unwrap();
    assert_selected(&local_file);
    assert!(run(&["chat", "hello"]).contains("not trusted"));
    assert!(run(&["chat", "--trust", "hello"]).contains("invalid TOML configuration"));
    antislop::lean::trust::Registry::at(&global)
        .untrust(&project)
        .unwrap();
    // An explicit file wins even over the broken project file.
    std::fs::write(project.join("custom.toml"), valid).unwrap();
    assert!(run(&["chat", "--config", "custom.toml", "hello"]).contains("not trusted"));
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
}

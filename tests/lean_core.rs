use architect::{
    config::Config,
    lean::{Core, store::TurnStatus},
};
use std::time::{Duration, Instant};

/// A codex-shaped CLI: records its arguments, answers with the prompt's last
/// line, sleeps on "slow" and fails on "fail".
fn fake_codex(dir: &std::path::Path) -> std::path::PathBuf {
    let script = dir.join("codex");
    let log = dir.join("calls.log");
    std::fs::write(
        &script,
        format!(
            r#"#!/bin/sh
for last; do :; done
printf '%s\n---\n' "$*" >> '{log}'
answer=$(printf '%s' "$last" | tail -n 1)
case "$answer" in *slow*) sleep 1 ;; esac
case "$answer" in *fail*) echo 'login required' >&2; exit 1 ;; esac
echo '{{"type":"thread.started","thread_id":"T1"}}'
echo '{{"type":"item.started","item":{{"type":"command_execution","command":"ls"}}}}'
printf '{{"type":"item.completed","item":{{"type":"agent_message","text":"answer: %s"}}}}\n' "$answer"
echo '{{"type":"turn.completed","usage":{{"input_tokens":1,"output_tokens":1}}}}'
"#,
            log = log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    script
}

fn config(codex: &std::path::Path) -> Config {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/examples/architect.toml"
    ))
    .unwrap()
    .replace(
        "binary = \"codex\"",
        &format!("binary = {:?}", codex.to_string_lossy()),
    );
    Config::from_toml(&text).unwrap()
}

fn wait_idle(core: &Core, conversation: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !core.is_idle(conversation) {
        assert!(Instant::now() < deadline, "orchestrator never went idle");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn messages_are_accepted_at_once_and_answered_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let core = Core::open(&dir.path().join("state"), config(&fake_codex(dir.path()))).unwrap();
    let conversation = core.start(&project).unwrap();

    let started = Instant::now();
    core.send(&conversation.id, "slow: przeanalizuj architekturę projektu")
        .unwrap();
    // Send the second message while the first turn is running.
    while core
        .store
        .last_turn(&conversation.id, "orchestrator")
        .unwrap()
        .is_none()
    {
        std::thread::sleep(Duration::from_millis(5));
    }
    core.send(&conversation.id, "druga wiadomość").unwrap();
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "sending must not wait for the agent"
    );
    wait_idle(&core, &conversation.id);

    let replies: Vec<_> = core
        .store
        .messages_after(&conversation.id, 0)
        .unwrap()
        .into_iter()
        .filter(|message| message.author == "orchestrator")
        .map(|message| message.text)
        .collect();
    assert_eq!(
        replies,
        [
            "answer: slow: przeanalizuj architekturę projektu",
            "answer: druga wiadomość"
        ]
    );
    let calls = std::fs::read_to_string(dir.path().join("calls.log")).unwrap();
    let calls: Vec<_> = calls.split("\n---\n").filter(|c| !c.is_empty()).collect();
    assert_eq!(calls.len(), 2);
    assert!(calls[0].contains("-s read-only"), "{}", calls[0]);
    assert!(calls[0].contains("Nothing from it is pasted here"));
    assert!(
        calls[1].contains("resume"),
        "follow-up resumes: {}",
        calls[1]
    );
    assert!(!calls[1].contains("Nothing from it is pasted here"));
    assert!(
        !calls[1].contains("przeanalizuj"),
        "answered text is not re-sent"
    );
}

#[test]
fn a_failed_turn_blocks_nothing_and_its_question_is_asked_again() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let core = Core::open(&dir.path().join("state"), config(&fake_codex(dir.path()))).unwrap();
    let conversation = core.start(&project).unwrap();

    core.send(&conversation.id, "fail please").unwrap();
    wait_idle(&core, &conversation.id);
    let turn = core
        .store
        .last_turn(&conversation.id, "orchestrator")
        .unwrap()
        .unwrap();
    assert_eq!(turn.status, TurnStatus::Failed);
    assert!(turn.error.unwrap().contains("login required"));

    core.send(&conversation.id, "now it works").unwrap();
    wait_idle(&core, &conversation.id);
    let turn = core
        .store
        .last_turn(&conversation.id, "orchestrator")
        .unwrap()
        .unwrap();
    assert_eq!(turn.status, TurnStatus::Done);
    let calls = std::fs::read_to_string(dir.path().join("calls.log")).unwrap();
    let last = calls
        .split("\n---\n")
        .filter(|c| !c.is_empty())
        .last()
        .unwrap();
    assert!(last.contains("fail please") && last.contains("now it works"));
}

fn chat(dir: &std::path::Path, project: &std::path::Path, message: &str) -> String {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
        .args(["chat", "--project"])
        .arg(project)
        .arg("--config")
        .arg(dir.join("architect.toml"))
        .arg("--state")
        .arg(dir.join("state"))
        .arg(message)
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn the_cli_talks_to_one_daemon_that_outlives_it() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path().canonicalize().unwrap();
    let project = dir.join("project");
    std::fs::create_dir(&project).unwrap();
    let codex = fake_codex(&dir);
    std::fs::write(
        dir.join("architect.toml"),
        std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/examples/architect.toml"
        ))
        .unwrap()
        .replace(
            "binary = \"codex\"",
            &format!("binary = {:?}", codex.to_string_lossy()),
        ),
    )
    .unwrap();

    let first = chat(&dir, &project, "pierwsza");
    assert!(first.contains("answer: pierwsza"), "{first}");
    let pid = |state: &std::path::Path| {
        architect::lean::daemon::Client::at(state)
            .call(&architect::lean::daemon::Request::Identity)
            .unwrap()["pid"]
            .clone()
    };
    let daemon = pid(&dir.join("state"));
    // The second frontend continues the same conversation on the same daemon.
    let second = chat(&dir, &project, "druga");
    assert!(
        second.contains("answer: druga") && !second.contains("pierwsza"),
        "{second}"
    );
    assert_eq!(pid(&dir.join("state")), daemon);
    let calls = std::fs::read_to_string(dir.join("calls.log")).unwrap();
    assert!(
        calls.contains("exec resume") && calls.contains("T1 druga"),
        "{calls}"
    );

    architect::lean::daemon::Client::at(&dir.join("state"))
        .call(&architect::lean::daemon::Request::Shutdown)
        .unwrap();
}

#[test]
fn a_question_cut_off_by_a_restart_is_answered_after_it() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let state = dir.path().join("state");
    let codex = fake_codex(dir.path());
    let core = Core::open(&state, config(&codex)).unwrap();
    let conversation = core.start(&project).unwrap();
    // As if the daemon died while the orchestrator worked on this message.
    let message = core
        .store
        .add_message(&conversation.id, "user", "co dalej?")
        .unwrap();
    core.store
        .start_turn(&conversation.id, "orchestrator", "codex", message.seq)
        .unwrap();
    drop(core);

    let core = Core::open(&state, config(&codex)).unwrap();
    wait_idle(&core, &conversation.id);
    let last = core
        .store
        .messages_after(&conversation.id, 0)
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(
        (last.author.as_str(), last.text.as_str()),
        ("orchestrator", "answer: co dalej?")
    );
}

#[test]
fn a_changed_configuration_waits_for_running_work_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path().canonicalize().unwrap();
    let project = dir.join("project");
    std::fs::create_dir(&project).unwrap();
    let codex = fake_codex(&dir);
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/examples/architect.toml"
    ))
    .unwrap()
    .replace(
        "binary = \"codex\"",
        &format!("binary = {:?}", codex.to_string_lossy()),
    );
    std::fs::write(dir.join("architect.toml"), &text).unwrap();
    let state = dir.join("state");
    let identity = || {
        architect::lean::daemon::Client::at(&state)
            .call(&architect::lean::daemon::Request::Identity)
            .ok()
    };
    let run = |message: &str| {
        std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
            .args(["chat", "--project"])
            .arg(&project)
            .arg("--config")
            .arg(dir.join("architect.toml"))
            .arg("--state")
            .arg(&state)
            .arg(message)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap()
    };

    let first = run("slow: pierwsza");
    let deadline = Instant::now() + Duration::from_secs(10);
    let daemon = loop {
        if let Some(identity) = identity().filter(|identity| identity["busy"] == true) {
            break identity["pid"].clone();
        }
        assert!(Instant::now() < deadline, "the daemon never got busy");
        std::thread::sleep(Duration::from_millis(10));
    };
    // The user edits the configuration while the work runs.
    std::thread::sleep(Duration::from_millis(20));
    std::fs::write(dir.join("architect.toml"), format!("{text}\n# edited\n")).unwrap();
    let second = run("druga").wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(second.status.success(), "{stderr}");
    assert!(stderr.contains("architect.toml zmienił się"), "{stderr}");
    assert_eq!(
        identity().unwrap()["pid"],
        daemon,
        "the running work kept its daemon"
    );
    assert!(first.wait_with_output().unwrap().status.success());

    // Once the work is done, the next frontend gets the new configuration.
    let third = chat(&dir, &project, "trzecia");
    assert!(third.contains("answer: trzecia"), "{third}");
    assert_ne!(identity().unwrap()["pid"], daemon);
    architect::lean::daemon::Client::at(&state)
        .call(&architect::lean::daemon::Request::Shutdown)
        .unwrap();
}

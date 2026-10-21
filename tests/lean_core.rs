mod common;
use antislop::{
    config::Config,
    lean::{Core, store::TurnStatus},
};
use std::time::{Duration, Instant};

/// A codex-shaped CLI: records its arguments, answers with the prompt's last
/// line, sleeps on "slow" (longer on "sleepy") and fails on "fail".
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
case "$answer" in *sleepy*) sleep 4 ;; esac
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
        "/examples/antislop.toml"
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

/// Each new native session gets its own id; resuming preserves that id.
fn comment_codex(dir: &std::path::Path) -> std::path::PathBuf {
    let script = dir.join("comment-codex");
    std::fs::write(
        &script,
        format!(
            r#"#!/bin/sh
session="session-$$"
for arg; do
  case "$arg" in session-*) session="$arg" ;; esac
  last="$arg"
done
printf '%s\n---\n' "$*" >> '{dir}/comment-calls.log'
printf '{{"type":"thread.started","thread_id":"%s"}}\n' "$session"
case "$last" in
  *retry-once*)
    if [ ! -f '{dir}/failed-once' ]; then
      touch '{dir}/failed-once'
      echo 'thread-store conflict: thread already has an active writer' >&2
      exit 1
    fi ;;
esac
case "$last" in *slow-comment*) sleep 1 ;; esac
echo '{{"type":"item.completed","item":{{"type":"agent_message","text":"Feedback addressed."}}}}'
echo '{{"type":"turn.completed"}}'
"#,
            dir = dir.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    script
}

#[test]
fn comments_use_an_independent_session_and_arrivals_get_their_own_turns() {
    use antislop::lean::COMMENT_ORCHESTRATOR;
    let dir = tempfile::tempdir().unwrap();
    let core = Core::open(
        &dir.path().join("state"),
        config(&comment_codex(dir.path())),
    )
    .unwrap();
    let conversation = core.start(dir.path()).unwrap();
    core.send(&conversation.id, "Original request").unwrap();
    wait_idle(&core, &conversation.id);
    let main = core
        .store
        .last_turn(&conversation.id, "orchestrator")
        .unwrap()
        .unwrap();
    let first = core
        .comment(&conversation.id, "orchestrator", "slow-comment")
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while core
        .store
        .last_turn(&conversation.id, COMMENT_ORCHESTRATOR)
        .unwrap()
        .is_none()
    {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let second = core
        .comment(&conversation.id, "orchestrator", "second comment")
        .unwrap();
    let third = core
        .comment(&conversation.id, "orchestrator", "third comment")
        .unwrap();
    wait_idle(&core, &conversation.id);

    let sessions = core.store.sessions(&conversation.id).unwrap();
    let comments: Vec<_> = sessions
        .iter()
        .filter(|turn| turn.role == COMMENT_ORCHESTRATOR)
        .collect();
    assert_eq!(comments.len(), 3);
    assert!(comments.iter().all(|turn| turn.status == "done"));
    assert!(
        comments
            .iter()
            .all(|turn| turn.session_id != main.session_id)
    );
    assert!(
        comments
            .iter()
            .all(|turn| turn.session_id == comments[0].session_id)
    );
    let events = core.store.events_since(&conversation.id, 0, 500).unwrap();
    let ids: Vec<_> = events
        .iter()
        .filter(|event| {
            event.kind == "turn_started" && event.payload["role"] == COMMENT_ORCHESTRATOR
        })
        .map(|event| event.payload["comments"].clone())
        .collect();
    assert_eq!(
        ids,
        [
            serde_json::json!([first.id]),
            serde_json::json!([second.id]),
            serde_json::json!([third.id])
        ]
    );
    core.send(&conversation.id, "Continue the request").unwrap();
    wait_idle(&core, &conversation.id);
    let resumed_main = core
        .store
        .last_turn(&conversation.id, "orchestrator")
        .unwrap()
        .unwrap();
    assert_eq!(resumed_main.session_id, main.session_id);
    let calls = std::fs::read_to_string(dir.path().join("comment-calls.log")).unwrap();
    assert!(calls.contains("The comment agent handled feedback"));
}

#[test]
fn failed_comments_keep_visible_cause_and_reopening_retries_them() {
    use antislop::lean::{COMMENT_ORCHESTRATOR, tui::view::View};
    let dir = tempfile::tempdir().unwrap();
    let core = Core::open(
        &dir.path().join("state"),
        config(&comment_codex(dir.path())),
    )
    .unwrap();
    let conversation = core.start(dir.path()).unwrap();
    let comment = core
        .comment(&conversation.id, "plan", "retry-once")
        .unwrap();
    // A comment on the plan is a draft until the user sends the review.
    assert_eq!(comment.status, "draft");
    assert!(core.is_idle(&conversation.id));
    core.review_comments(&conversation.id, "plan").unwrap();
    wait_idle(&core, &conversation.id);
    let comment = core
        .store
        .comments(&conversation.id, "open")
        .unwrap()
        .remove(0);
    let failed = core
        .store
        .last_turn(&conversation.id, COMMENT_ORCHESTRATOR)
        .unwrap()
        .unwrap();
    assert_eq!(failed.status, TurnStatus::Failed);
    assert!(
        failed
            .error
            .as_deref()
            .unwrap()
            .contains("thread-store conflict")
    );
    let mut view = View::default();
    for event in core.store.events_since(&conversation.id, 0, 500).unwrap() {
        view.apply(&serde_json::to_value(event).unwrap());
    }
    assert_eq!(view.comment_state(comment.id), "failed");
    assert!(
        view.comment_error(comment.id)
            .unwrap()
            .contains("thread-store conflict")
    );
    assert!(view.comment_activity().is_none());
    let reopened = core
        .set_comment(&conversation.id, comment.id, "open")
        .unwrap();
    assert!(reopened.message > comment.message);
    wait_idle(&core, &conversation.id);
    for event in core
        .store
        .events_since(&conversation.id, view.after, 500)
        .unwrap()
    {
        view.apply(&serde_json::to_value(event).unwrap());
    }
    assert_eq!(view.comment_state(comment.id), "answered");
    assert!(view.comment_error(comment.id).is_none());
    assert_eq!(
        core.store.comments(&conversation.id, "open").unwrap().len(),
        1
    );
}

#[test]
fn plan_revision_resolves_only_comments_in_its_answered_snapshot() {
    use antislop::lean::store::{Comment, Store};
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("core.db")).unwrap();
    let conversation = store.create_conversation("project").unwrap();
    let add = |target: &str| {
        store
            .add_comment(
                &conversation.id,
                &Comment {
                    id: 0,
                    plan_id: Some("p1".into()),
                    target: target.into(),
                    label: target.into(),
                    text: "feedback".into(),
                    status: "open".into(),
                    message: 0,
                },
                |id| format!("[user comment #{id}] feedback"),
            )
            .unwrap()
    };
    let first = add("plan:p1:line:2");
    let second = add("plan:p1");
    let other = add("worker:t1");
    store
        .resolve_plan_comments(&conversation.id, first.message)
        .unwrap();
    assert_eq!(
        store.comments(&conversation.id, "resolved").unwrap()[0].id,
        first.id
    );
    assert_eq!(
        store
            .comments(&conversation.id, "open")
            .unwrap()
            .iter()
            .map(|comment| comment.id)
            .collect::<Vec<_>>(),
        [second.id, other.id]
    );
    // A retry of an older comment must also stay outside a stale snapshot.
    store
        .set_comment_status(&conversation.id, first.id, "open")
        .unwrap();
    store
        .resolve_plan_comments(&conversation.id, second.message)
        .unwrap();
    assert!(
        store
            .comments(&conversation.id, "open")
            .unwrap()
            .iter()
            .any(|comment| comment.id == first.id)
    );
}

#[test]
fn interrupted_comment_sessions_resume_after_daemon_restart() {
    use antislop::lean::{
        COMMENT_ORCHESTRATOR,
        store::{Comment, Store},
        tui::view::View,
    };
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    antislop::lean::daemon::private_directory(&state).unwrap();
    let store = Store::open(&state.join("core.db")).unwrap();
    let conversation = store
        .create_conversation(dir.path().to_str().unwrap())
        .unwrap();
    let comment = store
        .add_comment(
            &conversation.id,
            &Comment {
                id: 0,
                plan_id: None,
                target: "plan".into(),
                label: "the plan".into(),
                text: "feedback".into(),
                status: "open".into(),
                message: 0,
            },
            |id| format!("[user comment #{id}] feedback"),
        )
        .unwrap();
    let turn = store
        .start_turn(
            &conversation.id,
            COMMENT_ORCHESTRATOR,
            "codex",
            comment.message,
            None,
        )
        .unwrap();
    store.append_event(&conversation.id, Some(&turn.id), None, "turn_started", &serde_json::json!({"role":COMMENT_ORCHESTRATOR, "provider":"codex", "comments":[comment.id]})).unwrap();
    drop(store);
    let core = Core::open(&state, config(&comment_codex(dir.path()))).unwrap();
    wait_idle(&core, &conversation.id);
    let turns = core.store.sessions(&conversation.id).unwrap();
    assert!(
        turns
            .iter()
            .any(|session| session.turn_id == turn.id && session.status == "interrupted")
    );
    assert!(
        turns
            .iter()
            .any(|session| session.role == COMMENT_ORCHESTRATOR && session.status == "done")
    );
    let mut view = View::default();
    for event in core.store.events_since(&conversation.id, 0, 500).unwrap() {
        view.apply(&serde_json::to_value(event).unwrap());
    }
    assert_eq!(view.comment_state(comment.id), "answered");
    assert!(view.comment_activity().is_none());
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
    common::trust(project);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
        .current_dir(project)
        .arg("chat")
        .arg("--config")
        .arg(dir.join("antislop.toml"))
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
        dir.join("antislop.toml"),
        std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/examples/antislop.toml"
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
        antislop::lean::daemon::Client::at(state)
            .call(&antislop::lean::daemon::Request::Identity)
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
        calls.contains("exec resume") && calls.contains("T1 [user]\ndruga"),
        "{calls}"
    );

    let client = antislop::lean::daemon::Client::at(&dir.join("state"));
    let conversation = client
        .call(&antislop::lean::daemon::Request::Conversations)
        .unwrap()[0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    client
        .call(&antislop::lean::daemon::Request::Send {
            conversation: conversation.clone(),
            text: "sleepy".into(),
        })
        .unwrap();
    let stop = |force: bool| {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_antislop"));
        command.current_dir(&dir).args(["stop", "--state", "state"]);
        if force {
            command.arg("--force");
        }
        command.output().unwrap()
    };
    let busy = stop(false);
    assert!(!busy.status.success(), "{busy:?}");
    assert!(
        String::from_utf8_lossy(&busy.stderr).contains("the daemon is executing work"),
        "{busy:?}"
    );
    assert_eq!(pid(&dir.join("state")), daemon);
    let follower = antislop::lean::daemon::Client::connect(
        &project,
        &dir.join("antislop.toml"),
        &dir.join("state"),
    )
    .unwrap();
    let forced = stop(true);
    assert!(forced.status.success(), "{forced:?}");
    assert!(
        client
            .call(&antislop::lean::daemon::Request::Identity)
            .is_err()
    );
    // An existing frontend must not automatically restart an explicitly stopped daemon.
    assert!(
        follower
            .call(&antislop::lean::daemon::Request::Events {
                conversation,
                after: 0,
                wait_ms: 0,
            })
            .is_err()
    );
    assert!(
        client
            .call(&antislop::lean::daemon::Request::Identity)
            .is_err()
    );
    let store = antislop::lean::store::Store::open(&dir.join("state/core.db")).unwrap();
    assert!(store.running_processes().unwrap().is_empty());
    drop(store);
    // The saved conversation remains usable after a forced shutdown.
    assert!(chat(&dir, &project, "after force").contains("answer: after force"));
    // Stopping works even after configuration has been removed.
    std::fs::remove_file(dir.join("antislop.toml")).unwrap();
    let stopped = stop(false);
    assert!(stopped.status.success(), "{stopped:?}");
    assert!(String::from_utf8_lossy(&stopped.stdout).contains("daemon stopped"));
    assert!(
        client
            .call(&antislop::lean::daemon::Request::Identity)
            .is_err()
    );
    let again = stop(false);
    assert!(again.status.success(), "{again:?}");
    assert!(String::from_utf8_lossy(&again.stdout).contains("daemon is not running"));
}

#[test]
fn stopping_without_a_daemon_creates_no_state() {
    let dir = tempfile::tempdir().unwrap();
    for args in [vec!["stop"], vec!["stop", "--state", "custom"]] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
            .current_dir(dir.path())
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stdout).contains("daemon is not running"));
    }
    for args in [
        vec!["stop", "--state"],
        vec!["stop", "--new"],
        vec!["stop", "oops"],
    ] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
            .current_dir(dir.path())
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success(), "{output:?}");
    }
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
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
        .start_turn(&conversation.id, "orchestrator", "codex", message.seq, None)
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
    reconnect_to_busy_daemon(false, true);
}

#[test]
fn a_rebuilt_frontend_reconnects_to_running_work() {
    reconnect_to_busy_daemon(true, false);
}

#[test]
fn a_rebuilt_frontend_with_changed_configuration_reconnects_to_running_work() {
    reconnect_to_busy_daemon(true, true);
}

fn reconnect_to_busy_daemon(rebuilt: bool, edit_config: bool) {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path().canonicalize().unwrap();
    let project = dir.join("project");
    std::fs::create_dir(&project).unwrap();
    let codex = fake_codex(&dir);
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/examples/antislop.toml"
    ))
    .unwrap()
    .replace(
        "binary = \"codex\"",
        &format!("binary = {:?}", codex.to_string_lossy()),
    );
    std::fs::write(dir.join("antislop.toml"), &text).unwrap();
    let state = dir.join("state");
    let identity = || {
        antislop::lean::daemon::Client::at(&state)
            .call(&antislop::lean::daemon::Request::Identity)
            .ok()
    };
    let current_build = std::path::Path::new(env!("CARGO_BIN_EXE_antislop"));
    let old_build = dir.join("antislop-old");
    if rebuilt {
        // A different executable path has a different build identity, just
        // like a rebuilt or reinstalled frontend.
        std::fs::copy(current_build, &old_build).unwrap();
    }
    let run = |binary: &std::path::Path, message: &str| {
        std::process::Command::new(binary)
            .current_dir(&project)
            .arg("chat")
            .arg("--config")
            .arg(dir.join("antislop.toml"))
            .arg("--state")
            .arg(&state)
            .arg(message)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap()
    };

    common::trust(&project);
    let mut first = run(
        if rebuilt { &old_build } else { current_build },
        "sleepy: pierwsza",
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    let daemon = loop {
        if let Some(identity) = identity().filter(|identity| identity["busy"] == true) {
            break identity["pid"].clone();
        }
        if first.try_wait().unwrap().is_some() {
            panic!(
                "the first frontend exited: {:?}",
                first.wait_with_output().unwrap()
            );
        }
        assert!(Instant::now() < deadline, "the daemon never got busy");
        std::thread::sleep(Duration::from_millis(10));
    };
    // Other projects and configuration paths must not silently use this daemon.
    let other_project = dir.join("other-project");
    std::fs::create_dir(&other_project).unwrap();
    let other_config = dir.join("other.toml");
    std::fs::write(&other_config, &text).unwrap();
    for (project, config) in [
        (other_project.as_path(), dir.join("antislop.toml")),
        (project.as_path(), other_config),
    ] {
        let error = antislop::lean::daemon::Client::connect(project, &config, &state)
            .err()
            .expect("a busy daemon must reject another project or configuration path");
        assert!(error.to_string().contains("different --state directory"));
    }
    if edit_config {
        // The user edits the configuration while the work runs.
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(dir.join("antislop.toml"), format!("{text}\n# edited\n")).unwrap();
    }
    let second = run(current_build, "druga").wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(second.status.success(), "{stderr}");
    assert!(
        stderr.contains(if rebuilt {
            "connected to the running daemon"
        } else {
            "antislop.toml changed"
        }),
        "{stderr}"
    );
    let stdout = String::from_utf8_lossy(&second.stdout);
    assert!(stdout.contains("answer: druga"), "{stdout}");
    assert_eq!(
        identity().unwrap()["pid"],
        daemon,
        "the running work kept its daemon"
    );
    assert!(first.wait_with_output().unwrap().status.success());

    // Once the work is done, the next frontend gets the new build/configuration.
    let third = chat(&dir, &project, "trzecia");
    assert!(third.contains("answer: trzecia"), "{third}");
    assert_ne!(identity().unwrap()["pid"], daemon);
    antislop::lean::daemon::Client::at(&state)
        .call(&antislop::lean::daemon::Request::Shutdown)
        .unwrap();
}

#[test]
fn a_daemon_whose_state_is_deleted_leaves() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path().canonicalize().unwrap();
    let project = dir.join("project");
    std::fs::create_dir(&project).unwrap();
    let codex = fake_codex(&dir);
    std::fs::write(
        dir.join("antislop.toml"),
        std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/examples/antislop.toml"
        ))
        .unwrap()
        .replace(
            "binary = \"codex\"",
            &format!("binary = {:?}", codex.to_string_lossy()),
        ),
    )
    .unwrap();
    chat(&dir, &project, "pierwsza");
    let pid = antislop::lean::daemon::Client::at(&dir.join("state"))
        .call(&antislop::lean::daemon::Request::Identity)
        .unwrap()["pid"]
        .as_i64()
        .unwrap() as libc::pid_t;

    std::fs::remove_dir_all(dir.join("state")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    // SAFETY: signal 0 only asks whether the process exists.
    while unsafe { libc::kill(pid, 0) } == 0 {
        assert!(Instant::now() < deadline, "the daemon lingered");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn a_dead_daemons_agent_is_ended_before_its_turn_resumes() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let state = dir.path().join("state");
    let codex = fake_codex(dir.path());
    let core = Core::open(&state, config(&codex)).unwrap();
    let conversation = core.start(&project).unwrap();
    // As if the daemon died with this CLI still working: it was its own
    // process group, so it outlived the daemon.
    let turn = core
        .store
        .start_turn(&conversation.id, "worker:t1", "codex", 0, None)
        .unwrap();
    let orphan_script = dir.path().join("orphan").join("codex");
    std::fs::create_dir(orphan_script.parent().unwrap()).unwrap();
    std::fs::write(&orphan_script, "#!/bin/sh\nsleep 30\n").unwrap();
    std::fs::set_permissions(
        &orphan_script,
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .unwrap();
    let mut orphan = std::os::unix::process::CommandExt::process_group(
        &mut std::process::Command::new(&orphan_script),
        0,
    )
    .spawn()
    .unwrap();
    core.store.set_turn_pid(&turn.id, orphan.id()).unwrap();
    drop(core);

    let _core = Core::open(&state, config(&codex)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while orphan.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "the orphaned CLI still runs");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_frontend_outlives_a_daemon_killed_mid_turn() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path().canonicalize().unwrap();
    let project = dir.join("project");
    std::fs::create_dir(&project).unwrap();
    let codex = fake_codex(&dir);
    std::fs::write(
        dir.join("antislop.toml"),
        std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/examples/antislop.toml"
        ))
        .unwrap()
        .replace(
            "binary = \"codex\"",
            &format!("binary = {:?}", codex.to_string_lossy()),
        ),
    )
    .unwrap();
    common::trust(&project);
    let frontend = std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
        .current_dir(&project)
        .arg("chat")
        .arg("--config")
        .arg(dir.join("antislop.toml"))
        .arg("--state")
        .arg(dir.join("state"))
        .arg("sleepy pytanie")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !std::fs::read_to_string(dir.join("calls.log"))
        .unwrap_or_default()
        .contains("sleepy pytanie")
    {
        assert!(Instant::now() < deadline, "the orchestrator never started");
        std::thread::sleep(Duration::from_millis(20));
    }
    let daemon = antislop::lean::daemon::Client::at(&dir.join("state"))
        .call(&antislop::lean::daemon::Request::Identity)
        .unwrap()["pid"]
        .as_i64()
        .unwrap() as libc::pid_t;
    // SAFETY: kills the test's own daemon, as a crash would.
    unsafe { libc::kill(daemon, libc::SIGKILL) };

    let output = frontend.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{output:?}");
    assert!(stdout.contains("answer: sleepy pytanie"), "{stdout}");
    let calls = std::fs::read_to_string(dir.join("calls.log")).unwrap();
    assert_eq!(calls.matches("sleepy pytanie").count(), 2, "{calls}");
}

/// The runtime's new card-feedback shape — bounded effect text plus the
/// explicit `effect_cut` flag, and a coverage preview with bounded unknown
/// ids and an omitted count — written through the task row and announced as
/// a `task` event, then read back: nothing the runtime stored changes on
/// the way through SQLite, and the whole verdict keeps every id intact.
#[test]
fn new_card_feedback_payloads_survive_storage_and_events() {
    use antislop::lean::{store::Store, tasks};
    use serde_json::json;

    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("core.db")).unwrap();
    let conversation = store.create_conversation("proj").unwrap();
    let specs = [
        json!({"id": "t1", "title": "x", "goal": "g", "writes": true,
        "kind": "implementation", "verifier": {"focus": "f"}}),
    ];
    let mut task = store.add_plan(&conversation.id, &specs).unwrap().remove(0);

    let obligations = vec![("R1".to_owned(), "risk".to_owned())];
    let mut coverage = serde_json::Map::new();
    coverage.insert("R1".into(), json!({"status": "ok", "note": "checked"}));
    for id in ["W1", "W2", "W3"] {
        coverage.insert(id.into(), json!({"status": "ok", "note": "n"}));
    }
    let enormous = format!("X{}", "y".repeat(500));
    coverage.insert(enormous.clone(), json!({"status": "ok", "note": "n"}));
    let verdict = tasks::Verdict {
        approved: true,
        findings: vec![],
        coverage,
    };
    let full = serde_json::to_value(&verdict).unwrap();

    // A diff stat longer than the effect budget: the stored text is cut on
    // whole clusters and the flag records that the runtime cut it.
    let files: Vec<String> = (0..40).map(|i| format!("src/m{i}.rs")).collect();
    let stat = format!("{} files changed, 500 insertions(+)", "z".repeat(260));
    let (effect, effect_cut) = tasks::effect(&stat, &files);
    assert!(effect.len() <= tasks::EFFECT_LIMIT, "{}", effect.len());
    assert!(effect_cut);
    let (card_effect, card_cut) = tasks::effect_keys(&Some((effect.clone(), effect_cut)));

    task.round = 2;
    task.feedback = Some(json!({
        "round": 2,
        "effect": card_effect,
        "effect_cut": card_cut,
        "coverage": tasks::coverage(&verdict, &obligations),
    }));
    task.verdict = Some(full);
    store.update_task(&task).unwrap();
    store
        .append_event(
            &conversation.id,
            None,
            Some(&task.plan_id),
            "task",
            &json!({"id": "t1", "task": task.id, "status": task.status,
                    "round": task.round, "reason": null,
                    "feedback": task.feedback}),
        )
        .unwrap();

    let feedback = task.feedback.clone().unwrap();
    let stored = store.task_by_id(&task.id).unwrap().expect("task stored");
    assert_eq!(stored.feedback, task.feedback);
    let event = store
        .events_since(&conversation.id, 0, 10)
        .unwrap()
        .into_iter()
        .rev()
        .find(|event| event.kind == "task")
        .expect("task event");
    assert_eq!(event.payload["feedback"], feedback);

    assert_eq!(feedback["effect"], effect);
    assert_eq!(feedback["effect_cut"], json!(true));
    assert!(feedback["effect"].as_str().unwrap().ends_with('…'));
    // Unknown ids: all four shown (fewer than the limit), each within the
    // preview's byte budget, the enormous one kept whole in the verdict.
    let shown = feedback["coverage"]["unknown"].as_array().unwrap();
    assert_eq!(shown.len(), 4);
    assert_eq!(feedback["coverage"]["unknown_more"], 0);
    assert!(
        shown
            .iter()
            .all(|id| id.as_str().unwrap().len() <= tasks::UNKNOWN_ID_BYTES)
    );
    assert!(
        shown.iter().any(|id| id.as_str().unwrap().ends_with('…')),
        "the oversized id is cut, not dropped"
    );
    assert_eq!(feedback["coverage"]["total"], 1);
    assert_eq!(feedback["coverage"]["ok"], 1);
    let stored_verdict = stored.verdict.expect("verdict stored");
    assert!(
        stored_verdict["coverage"]
            .as_object()
            .unwrap()
            .contains_key(&enormous),
        "the full verdict still holds the whole id"
    );

    // Older feedback stays readable: a row without the new keys round-trips
    // unchanged and the round contract never reads a cut from the text.
    let mut old = task.clone();
    old.round = 3;
    old.feedback = Some(json!({"round": 3, "effect": "done…", "coverage": {"total": 0}}));
    store.update_task(&old).unwrap();
    let restored = store.task_by_id(&task.id).unwrap().expect("task");
    assert_eq!(restored.feedback, old.feedback);
    assert_eq!(
        restored.feedback.as_ref().unwrap()["effect_cut"],
        serde_json::Value::Null
    );
    assert!(!tasks::feedback_is_current(&restored.feedback.unwrap(), 2));
}

#[test]
fn missing_configuration_reports_project_and_global_candidates() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    let global = root.path().join("user-config");
    std::fs::create_dir(&project).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
        .current_dir(&project)
        .env("ANTISLOP_CONFIG_DIR", &global)
        .args(["chat", "hello"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("no configuration at"), "{error}");
    for candidate in [project.join("antislop.toml"), global.join("antislop.toml")] {
        assert!(error.contains(candidate.to_str().unwrap()), "{error}");
    }
    assert!(!global.exists());
    assert!(!project.join(".antislop").exists());
}

#[test]
fn configuration_snapshots_survive_live_replacement() {
    let dir = tempfile::tempdir().unwrap();
    let codex = fake_codex(dir.path());
    let initial = config(&codex);
    let core = Core::open(&dir.path().join(".antislop"), initial.clone()).unwrap();
    let in_flight = core.config_snapshot();
    let mut updated = initial.clone();
    updated.policy.max_parallel = 1;
    updated.orchestrator.profile = "review".into();
    core.replace_config(updated.clone());
    assert_eq!(*in_flight, initial);
    assert_eq!(*core.config_snapshot(), updated);
}

#[test]
fn next_turn_uses_live_profile_while_running_turn_keeps_its_profile() {
    let dir = tempfile::tempdir().unwrap();
    let codex = fake_codex(dir.path());
    let initial = config(&codex);
    let core = Core::open(&dir.path().join(".antislop"), initial.clone()).unwrap();
    let conversation = core.start(dir.path()).unwrap();
    core.send(&conversation.id, "sleepy").unwrap();
    let log = dir.path().join("calls.log");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !log.exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    let running = std::fs::read_to_string(&log).unwrap();
    let mut updated = initial;
    updated.profiles.get_mut("orchestrator").unwrap().model = Some("live-model".into());
    updated.profiles.get_mut("orchestrator").unwrap().effort = antislop::config::Effort::Low;
    updated.policy.max_parallel = 1;
    assert!(!core.is_idle(&conversation.id));
    core.replace_config(updated);
    wait_idle(&core, &conversation.id);
    let completed = std::fs::read_to_string(&log).unwrap();
    assert_eq!(
        completed, running,
        "configuration update restarted the active turn"
    );
    assert!(!completed.contains("live-model"));
    core.send(&conversation.id, "next request").unwrap();
    wait_idle(&core, &conversation.id);
    let calls = std::fs::read_to_string(log).unwrap();
    assert!(calls.contains("live-model"));
    assert!(calls.contains("model_reasoning_effort=\"low\""));
    assert!(calls.starts_with(&running));
}

#[test]
fn process_failure_does_not_persist_credentials_from_cli_files() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let script = fake_codex(dir.path());
    let source = std::fs::read_to_string(&script).unwrap().replace(
        "login required",
        "login required https://user:file-only-secret@host/encoded-token",
    );
    std::fs::write(&script, source).unwrap();
    let core = Core::open(&dir.path().join("state"), config(&script)).unwrap();
    let conversation = core.start(&project).unwrap();
    core.send(&conversation.id, "fail please").unwrap();
    wait_idle(&core, &conversation.id);
    let turn = core
        .store
        .last_turn(&conversation.id, "orchestrator")
        .unwrap()
        .unwrap();
    assert_eq!(turn.status, TurnStatus::Failed);
    let stored = turn.error.unwrap();
    assert!(stored.contains("codex ended without a result (exit status: 1): login required"));
    assert!(stored.contains("[REDACTED]"));
    assert!(!stored.contains("file-only-secret"));
    let events =
        serde_json::to_string(&core.store.events_since(&conversation.id, 0, 500).unwrap()).unwrap();
    assert!(!events.contains("file-only-secret"));
    assert!(!events.contains("encoded-token"));
    assert!(
        core.store
            .events_since(&conversation.id, 0, 500)
            .unwrap()
            .iter()
            .any(|event| event.kind == "agent"
                && event.payload["type"] == "process_exit"
                && event.payload["code"] == 1)
    );
}

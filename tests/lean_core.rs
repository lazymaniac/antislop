mod common;
use architect::{
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
    use architect::lean::COMMENT_ORCHESTRATOR;
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
fn failed_comments_keep_the_raw_error_and_reopening_retries_them() {
    use architect::lean::{COMMENT_ORCHESTRATOR, tui::view::View};
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
            .contains("already has an active writer")
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
    use architect::lean::store::{Comment, Store};
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
    use architect::lean::{
        COMMENT_ORCHESTRATOR,
        store::{Comment, Store},
        tui::view::View,
    };
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    architect::lean::daemon::private_directory(&state).unwrap();
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
        calls.contains("exec resume") && calls.contains("T1 [user]\ndruga"),
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
        (other_project.as_path(), dir.join("architect.toml")),
        (project.as_path(), other_config),
    ] {
        let error = architect::lean::daemon::Client::connect(project, &config, &state)
            .err()
            .expect("a busy daemon must reject another project or configuration path");
        assert!(error.to_string().contains("different --state directory"));
    }
    if edit_config {
        // The user edits the configuration while the work runs.
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(dir.join("architect.toml"), format!("{text}\n# edited\n")).unwrap();
    }
    let second = run(current_build, "druga").wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(second.status.success(), "{stderr}");
    assert!(
        stderr.contains(if rebuilt {
            "connected to the running daemon"
        } else {
            "architect.toml changed"
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
    architect::lean::daemon::Client::at(&state)
        .call(&architect::lean::daemon::Request::Shutdown)
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
    chat(&dir, &project, "pierwsza");
    let pid = architect::lean::daemon::Client::at(&dir.join("state"))
        .call(&architect::lean::daemon::Request::Identity)
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
    common::trust(&project);
    let frontend = std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
        .current_dir(&project)
        .arg("chat")
        .arg("--config")
        .arg(dir.join("architect.toml"))
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
    let daemon = architect::lean::daemon::Client::at(&dir.join("state"))
        .call(&architect::lean::daemon::Request::Identity)
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

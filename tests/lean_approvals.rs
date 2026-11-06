mod common;
use antislop::{config::Config, lean::Core};
use std::time::{Duration, Instant};

/// A codex-shaped CLI that answers with the prompt's last line.
fn fake_codex(dir: &std::path::Path) -> std::path::PathBuf {
    let script = dir.join("codex");
    std::fs::write(
        &script,
        r#"#!/bin/sh
for last; do :; done
answer=$(printf '%s' "$last" | tail -n 1)
echo '{"type":"thread.started","thread_id":"T1"}'
printf '{"type":"item.completed","item":{"type":"agent_message","text":"answer: %s"}}\n' "$answer"
echo '{"type":"turn.completed"}'
"#,
    )
    .unwrap();
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    common::interactive(&script);
    script.canonicalize().unwrap()
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
    let deadline = Instant::now() + Duration::from_secs(120);
    while !core.is_idle(conversation) {
        assert!(Instant::now() < deadline, "orchestrator never went idle");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The screen Codex showed for a command that needs approval.
fn codex_dialog() -> String {
    include_str!("fixtures/approvals/codex/command.txt")
        .split_once('\n')
        .unwrap()
        .1
        .to_owned()
}

fn events_of(core: &Core, conversation: &str, kind: &str) -> Vec<serde_json::Value> {
    core.store
        .events_since(conversation, 0, 1000)
        .unwrap()
        .into_iter()
        .filter(|event| event.kind == kind)
        .map(|event| event.payload)
        .collect()
}

#[test]
fn an_agent_waits_for_the_users_answer_and_gets_the_chosen_key() {
    let dir = common::tempdir();
    let codex = fake_codex(dir.path());
    let mut dialog = codex.as_os_str().to_owned();
    dialog.push(".dialog.turn");
    std::fs::write(&dialog, codex_dialog()).unwrap();
    let mut keys = codex.as_os_str().to_owned();
    keys.push(".keys");
    let core = Core::open(&dir.path().join("state"), config(&codex)).unwrap();
    let conversation = core.start(dir.path()).unwrap();
    assert_eq!(core.approval_mode(&conversation.id).unwrap(), "supervised");
    core.send(&conversation.id, "Original request").unwrap();

    let deadline = Instant::now() + Duration::from_secs(60);
    let request = loop {
        if let Some(asked) = events_of(&core, &conversation.id, "approval_requested").first() {
            break asked.clone();
        }
        assert!(
            Instant::now() < deadline,
            "the dialog never became a question"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(request["role"], "orchestrator");
    assert_eq!(request["provider"], "codex");
    assert_eq!(request["decider"], "user");
    assert!(request["summary"].as_str().unwrap().contains("touch"));
    assert_eq!(request["options"][0]["decision"], "allow");
    // Nothing is pressed while the user has not answered.
    std::thread::sleep(Duration::from_millis(500));
    assert!(!std::path::Path::new(&keys).exists());
    assert!(!core.is_idle(&conversation.id));

    core.answer_approval(&conversation.id, request["request"].as_str().unwrap(), "1")
        .unwrap();
    wait_idle(&core, &conversation.id);
    // Codex's own shortcut for "Yes, proceed".
    assert_eq!(std::fs::read(&keys).unwrap(), b"y");
    let resolved = events_of(&core, &conversation.id, "approval_resolved");
    assert_eq!(resolved.len(), 1, "{resolved:?}");
    assert_eq!(resolved[0]["state"], "answered");
    assert_eq!(resolved[0]["by"], "user");
}

#[test]
fn the_mode_of_a_conversation_is_kept_and_announced() {
    let dir = common::tempdir();
    let core = Core::open(&dir.path().join("state"), config(&fake_codex(dir.path()))).unwrap();
    let conversation = core.start(dir.path()).unwrap();
    core.set_approval_mode(&conversation.id, "autonomous")
        .unwrap();
    assert_eq!(core.approval_mode(&conversation.id).unwrap(), "autonomous");
    assert!(core.set_approval_mode(&conversation.id, "yolo").is_err());
    let changed = events_of(&core, &conversation.id, "approval_mode_changed");
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0]["mode"], "autonomous");
    // Another conversation keeps the default.
    let other = core.start(dir.path()).unwrap();
    assert_eq!(core.approval_mode(&other.id).unwrap(), "supervised");
}

use architect::{
    domain::{Run, RunSpec},
    providers::{FailureKind, Provider, TurnRequest, codex::CodexProvider},
};
use std::{
    os::unix::fs::PermissionsExt,
    sync::{Arc, atomic::AtomicBool},
};

const MAX: usize = 1_048_576;
const SECRET: &str = "SENTINEL-PRIVATE-PROMPT";

fn fixture(root: &std::path::Path) -> (Run, CodexProvider, std::path::PathBuf) {
    let counter = root.join("launches");
    let binary = root.join("mock-codex");
    let code = format!(
        "#!/usr/bin/env python3\nimport sys,json,pathlib\nwith pathlib.Path({counter:?}).open('a') as f: f.write('launch\\n')\nif '--version' in sys.argv: print('codex-cli 0.162.1');sys.exit(0)\nprompt=sys.stdin.read()\nassert len(prompt)<={MAX}\nprint(json.dumps(dict(type='thread.started',thread_id='00000000-0000-4000-8000-000000000001')))\nprint(json.dumps(dict(type='item.completed',item=dict(id='answer',type='agent_message',text='{{}}'))))\nprint(json.dumps(dict(type='turn.completed')))\n"
    );
    std::fs::write(&binary, code).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.to_string_lossy().into();
    let run = Run::new(architect::id(), spec).unwrap();
    let provider = CodexProvider {
        binary: Some(binary.to_string_lossy().into()),
        ..Default::default()
    };
    (run, provider, counter)
}

#[test]
fn exact_ascii_and_multibyte_scalar_boundaries_reach_mock_cli() {
    for character in ['x', 'é', '🦀'] {
        let root = tempfile::tempdir().unwrap();
        let (run, provider, counter) = fixture(root.path());
        let prompt: String = std::iter::repeat_n(character, MAX).collect();
        assert_eq!(prompt.chars().count(), MAX);
        if character != 'x' {
            assert!(prompt.len() > MAX);
        }
        let receipt = provider
            .turn_receipt(
                &TurnRequest {
                    turn_id: "boundary",
                    run: &run,
                    prompt: &prompt,
                    schema: &serde_json::json!({}),
                    resume: None,
                },
                Arc::new(AtomicBool::new(false)),
                &mut |_| Ok(()),
            )
            .unwrap();
        assert!(receipt.completed);
        assert_eq!(
            std::fs::read_to_string(counter).unwrap(),
            "launch\nlaunch\n"
        );
    }
}

#[test]
fn oversized_requests_create_no_cli_process_delivery_or_ephemeral_files() {
    // A subprocess isolates TMPDIR without changing this test runner's global
    // environment. All schema/controlled-cwd writes would be visible there.
    let root = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "oversized_private_temp_fixture", "--nocapture"])
        .env("ANTISLOP_PREFLIGHT_FIXTURE", root.path())
        .env("TMPDIR", root.path())
        .env("TMP", root.path())
        .env("TEMP", root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn oversized_private_temp_fixture() {
    let Some(root) = std::env::var_os("ANTISLOP_PREFLIGHT_FIXTURE") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let (run, provider, counter) = fixture(&root);
    let mut before: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    before.sort();
    for prompt in [
        "x".repeat(MAX + 1),
        "🦀".repeat(MAX + 1),
        format!("{SECRET}{}", "é".repeat(MAX)),
    ] {
        let mut events = Vec::new();
        let error = provider
            .turn_receipt(
                &TurnRequest {
                    turn_id: "oversized",
                    run: &run,
                    prompt: &prompt,
                    schema: &serde_json::json!({}),
                    resume: None,
                },
                Arc::new(AtomicBool::new(false)),
                &mut |event| {
                    events.push(event.clone());
                    Ok(())
                },
            )
            .unwrap_err();
        let architect::Error::Provider(failure) = error else {
            panic!("expected typed provider failure");
        };
        assert_eq!(failure.kind, FailureKind::Protocol);
        assert!(failure.message.contains("input_too_large"));
        assert!(
            failure
                .message
                .contains(&format!("input_characters={}", prompt.chars().count()))
        );
        assert!(failure.message.contains("max_characters=1048576"));
        assert!(failure.stopped && !failure.partial_effects && failure.session_id.is_none());
        assert!(!failure.message.contains(SECRET));
        assert!(events.is_empty());
        assert!(!counter.exists());
        let mut after: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        after.sort();
        assert_eq!(after, before);
    }
}

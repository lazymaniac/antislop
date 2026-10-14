//! Completed model reports must be repairable without relaxing acceptance gates.
use architect::{
    config::{Config, ProviderKind, WorkspaceMode},
    domain::{RunSpec, Status},
    runtime::{Command, Runtime},
    sessions::TurnState,
    storage::Store,
};
use std::{
    os::unix::fs::PermissionsExt,
    sync::Arc,
    time::{Duration, Instant},
};

fn rejected_completed_output(raw_output: &str, mutate_pinned_input: bool) {
    let root = tempfile::tempdir().unwrap();
    let binary = root.path().join("codex-fixture");
    let script = format!(
        "#!/usr/bin/env python3\nimport sys,json,pathlib\nif '--version' in sys.argv: print('codex-cli 0.162.1');sys.exit(0)\nsys.stdin.read()\n{}\nprint(json.dumps(dict(type='thread.started',thread_id='00000000-0000-4000-8000-000000000001')),flush=True)\nprint(json.dumps(dict(type='item.completed',item=dict(type='agent_message',text={}))),flush=True)\nprint(json.dumps(dict(type='turn.completed')),flush=True)\n",
        if mutate_pinned_input {
            format!(
                "pathlib.Path({}).write_text('external mutation')",
                serde_json::to_string(root.path().join("pinned.txt").to_str().unwrap()).unwrap()
            )
        } else {
            String::new()
        },
        serde_json::to_string(raw_output).unwrap()
    );
    std::fs::write(&binary, script).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut config = Config::from_toml(include_str!("../examples/architect.toml")).unwrap();
    config.workspace.mode = WorkspaceMode::InPlace;
    config
        .providers
        .get_mut(&ProviderKind::Codex)
        .unwrap()
        .binary = binary;
    for profile in config.profiles.values_mut() {
        profile.provider = ProviderKind::Codex;
    }
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().to_string_lossy().into();
    spec.provider = "codex".into();
    spec.config = Some(config.pin().unwrap());
    std::fs::write(root.path().join("pinned.txt"), "unchanged").unwrap();
    spec.input_files = vec!["pinned.txt".into()];
    let runtime =
        Arc::new(Runtime::new(Store::open(&root.path().join("state.db")).unwrap()).unwrap());
    let run = runtime
        .dispatch(&architect::id(), Command::Create { spec })
        .unwrap();
    runtime
        .dispatch(
            &architect::id(),
            Command::ApproveStart {
                run_id: run.id.clone(),
                scope_hash: run.scope_hash("start"),
            },
        )
        .unwrap();
    runtime
        .dispatch(
            &architect::id(),
            Command::Advance {
                run_id: run.id.clone(),
            },
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let finished = loop {
        let current = runtime.store.lock().unwrap().get(&run.id).unwrap();
        if current.status != Status::Running {
            break current;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(finished.session_ledger.turns.len(), 1);
    let turn = finished.session_ledger.turns.values().next().unwrap();
    assert_eq!(turn.state, TurnState::Completed);
    assert_eq!(turn.output.as_deref(), Some(raw_output));
    if mutate_pinned_input {
        assert_eq!(finished.status, Status::Blocked);
        assert!(finished.reconciliation_required);
        assert!(
            runtime
                .dispatch(
                    &architect::id(),
                    Command::ApproveRepair {
                        run_id: run.id,
                        scope_hash: finished.scope_hash("repair"),
                    }
                )
                .is_err()
        );
        return;
    }
    assert_eq!(
        finished.status,
        Status::ChangesRequested,
        "{:?}",
        finished.diagnostic
    );
    assert!(!finished.reconciliation_required);
    assert!(finished.pending_edits.is_empty());
    assert!(finished.reviews.is_empty());
    assert!(finished.provenance.is_empty());
    assert_eq!(
        std::fs::read_to_string(root.path().join("pinned.txt")).unwrap(),
        "unchanged"
    );
    assert!(
        runtime
            .dispatch(
                &architect::id(),
                Command::AcceptResult {
                    run_id: run.id.clone(),
                    scope_hash: finished.scope_hash("accept"),
                }
            )
            .is_err()
    );
    let repaired = runtime
        .dispatch(
            &architect::id(),
            Command::ApproveRepair {
                run_id: run.id,
                scope_hash: finished.scope_hash("repair"),
            },
        )
        .unwrap();
    assert_eq!(repaired.status, Status::Ready);
    assert_eq!(repaired.session_ledger, finished.session_ledger);
}

#[test]
fn completed_semantically_invalid_output_allows_explicit_repair_without_acceptance() {
    rejected_completed_output(
        r#"{"artifact":"rejected report","edits":[],"context":null,"solution":null,"quality":null}"#,
        false,
    );
}

#[test]
fn completed_structurally_invalid_json_allows_explicit_repair_without_acceptance() {
    rejected_completed_output("{}", false);
}

#[test]
fn completed_invalid_output_with_changed_inputs_still_requires_reconciliation() {
    rejected_completed_output(
        r#"{"artifact":"rejected report","edits":[],"context":null,"solution":null,"quality":null}"#,
        true,
    );
}

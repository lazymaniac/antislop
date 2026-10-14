use architect::{
    domain::*,
    id,
    runtime::{Command, Runtime, intake},
    storage::Store,
};

#[test]
fn intake_preserves_prompt_and_cannot_execute_a_provisional_contract() {
    let root = tempfile::tempdir().unwrap();
    let config = intake::bootstrap("codex".into()).unwrap();
    assert!(!config.config.policy.local_write);
    assert!(!config.config.policy.local_checks);
    let runtime = std::sync::Arc::new(
        Runtime::new(Store::open(&root.path().join("state.db")).unwrap()).unwrap(),
    );
    let text = "Zbuduj raport\nNajpierw dopytaj o dane i wynik";
    let run = runtime
        .dispatch(
            &id(),
            Command::BeginIntake {
                objective: text.into(),
                workspace: root.path().to_string_lossy().into(),
                config,
            },
        )
        .unwrap();
    assert!(run.intake);
    assert_eq!(run.spec.objective, text);
    assert_eq!(run.spec.requirements[0].original_text, text);
    assert!(run.spec.editable_files.is_empty());
    assert!(run.spec.check_command.is_empty());
    assert!(
        runtime
            .dispatch(
                &id(),
                Command::Advance {
                    run_id: run.id.clone()
                }
            )
            .unwrap_err()
            .to_string()
            .contains("intake is conversational")
    );
    assert!(
        runtime
            .dispatch(
                &id(),
                Command::ApproveStart {
                    run_id: run.id.clone(),
                    scope_hash: run.scope_hash("start")
                }
            )
            .is_err()
    );
    assert!(
        runtime
            .dispatch(
                &id(),
                Command::Check {
                    run_id: run.id.clone(),
                    argv: vec!["echo".into(), "must not run".into()],
                    scope_hash: "unknown".into()
                }
            )
            .is_err()
    );
    let response = architect::orchestration::planning::Response {
        agent_messages: vec![],
        message: "Clarifying question".into(),
        comment_reactions: vec![],
        intake_proposal_json: None,
        team_plan_json: Some("{}".into()),
        risk_owners: vec![],
    };
    assert!(
        architect::orchestration::planning::validate_response(&run, &response)
            .unwrap_err()
            .to_string()
            .contains("interview")
    );
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = run.spec.workspace.clone();
    let revised = runtime
        .dispatch(
            &id(),
            Command::Revise {
                run_id: run.id.clone(),
                expected_revision: run.spec_revision,
                spec,
            },
        )
        .unwrap();
    assert!(!revised.intake);
    assert_eq!(revised.spec_revision, 2);
}

#[test]
fn frontend_starts_one_daemon_and_reuses_it() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state");
    let mut daemon = architect::tui::connect_or_start_daemon(
        &state,
        std::path::Path::new(env!("CARGO_BIN_EXE_antislop")),
    )
    .unwrap()
    .expect("first connection should start the daemon");
    struct Cleanup<'a>(&'a mut std::process::Child);
    impl Drop for Cleanup<'_> {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let _cleanup = Cleanup(&mut daemon);
    assert!(
        architect::tui::connect_or_start_daemon(
            &state,
            std::path::Path::new(env!("CARGO_BIN_EXE_antislop"))
        )
        .unwrap()
        .is_none()
    );
    use architect::tui::backend::{Backend, IpcBackend};
    assert!(IpcBackend::new(&state).list_runs().unwrap().is_empty());
}

#[test]
fn conversation_does_not_require_execution_routes_or_a_managed_worktree() {
    use architect::config::{WorkKind, WorkspaceMode};
    let root = tempfile::tempdir().unwrap();
    let mut config = intake::bootstrap("codex".into()).unwrap().config;
    config.workspace.mode = WorkspaceMode::Worktree;
    config
        .routing
        .retain(|kind, _| *kind == WorkKind::Orchestration);
    let runtime = std::sync::Arc::new(
        Runtime::new(Store::open(&root.path().join("state.db")).unwrap()).unwrap(),
    );
    let run = runtime
        .dispatch(
            &id(),
            Command::BeginIntake {
                objective: "Dopytaj o zadanie".into(),
                workspace: root.path().to_string_lossy().into(),
                config: config.pin().unwrap(),
            },
        )
        .unwrap();
    assert!(run.intake);
    assert_eq!(
        run.spec.config.as_ref().unwrap().config.workspace.mode,
        WorkspaceMode::Worktree
    );
    assert!(
        run.spec.validate().is_err(),
        "the provisional spec must not pass as an executable worktree contract"
    );
}

#[test]
fn explicit_bootstrap_preserves_read_only_intake_for_each_supported_cli() {
    use architect::config::*;
    for kind in [
        ProviderKind::Codex,
        ProviderKind::Claude,
        ProviderKind::Gemini,
        ProviderKind::Agy,
    ] {
        let binary = std::path::PathBuf::from("/explicit/conversation-cli");
        let pinned = intake::bootstrap_for(kind, binary.clone()).unwrap();
        let config = &pinned.config;
        assert_eq!(config.providers.len(), 1);
        assert_eq!(config.providers[&kind].binary, binary);
        assert!(
            !config.policy.local_write
                && !config.policy.local_checks
                && !config.policy.local_commits
        );
        assert_eq!(config.policy.mode, Mode::Supervised);
        assert_eq!(
            config.providers[&kind].isolation,
            if kind == ProviderKind::Codex {
                IsolationProfile::ReadOnly
            } else {
                IsolationProfile::ToolFree
            }
        );
        let orchestrator = config
            .resolve(WorkKind::Orchestration, RoleKind::Orchestrator)
            .unwrap();
        let reviewer = config
            .resolve(WorkKind::Orchestration, RoleKind::Verifier)
            .unwrap();
        assert_eq!(orchestrator.provider, kind);
        assert_eq!(reviewer.provider, kind);
        assert_ne!(
            config.orchestrator.profile,
            config.routing[&WorkKind::Orchestration].verifier
        );
        assert!(orchestrator.model.is_none() && reviewer.model.is_none());
        let spec = intake::spec("Ask about my task", "/workspace", &pinned).unwrap();
        assert!(spec.editable_files.is_empty() && spec.check_command.is_empty());
        assert_eq!(spec.requirements[0].source_ref, "user:chat");
    }
}

#[test]
fn ui_startup_rejects_ambiguous_or_existing_run_provider_changes_before_connecting() {
    use architect::{config::ProviderKind, tui::options::Options};
    let parse =
        |args: &[&str]| Options::parse(&args.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>());
    let defaults = parse(&[]).unwrap();
    assert_eq!(defaults.state, std::path::PathBuf::from(".antislop"));
    assert!(defaults.run_id.is_none() && defaults.orchestrator.is_none());
    let selected = parse(&["--orchestrator", "claude", "/state"]).unwrap();
    assert_eq!(selected.state, std::path::PathBuf::from("/state"));
    assert_eq!(selected.orchestrator, Some(ProviderKind::Claude));
    assert_eq!(
        parse(&["/state", "R1"]).unwrap().run_id.as_deref(),
        Some("R1")
    );
    for invalid in [
        vec!["--orchestrator"],
        vec!["--orchestrator", "unknown"],
        vec!["--orchestrator", "CLAUDE"],
        vec!["--orchestrator", "claude", "--orchestrator", "codex"],
        vec!["/state", "R1", "--orchestrator", "claude"],
        vec!["/state", "R1", "unexpected"],
        vec!["--unrecognized"],
        vec![""],
    ] {
        assert!(parse(&invalid).is_err(), "accepted {invalid:?}");
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
            .arg("ui")
            .args(&invalid)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(
            !String::from_utf8_lossy(&output.stderr).contains("TUI requires a terminal"),
            "must reject arguments before entering the terminal"
        );
    }
}

#[test]
fn explicit_bootstrap_cannot_override_a_project_configuration() {
    use architect::{config::ProviderKind, tui::backend::IpcBackend};
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("architect.toml"),
        "invalid project config must not be ignored",
    )
    .unwrap();
    let state = root.path().join("no-daemon");
    let error = IpcBackend::new(&state)
        .with_bootstrap_provider(ProviderKind::Claude)
        .begin_conversation_in("Interview first", root.path())
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("applies only without architect.toml"),
        "{error}"
    );
    assert!(
        !state.exists(),
        "configuration conflict must not create a run or daemon"
    );
}

#[test]
fn an_invalid_existing_project_config_never_falls_back_to_the_bootstrap() {
    use architect::tui::backend::IpcBackend;
    for broken_link in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("architect.toml");
        if broken_link {
            std::os::unix::fs::symlink(root.path().join("absent-config"), &config).unwrap();
        } else {
            std::fs::create_dir(&config).unwrap();
        }
        let state = root.path().join("no-daemon");
        let error = IpcBackend::new(&state)
            .begin_conversation_in("Interview first", root.path())
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("architect.toml is not a readable regular file"),
            "{error}"
        );
        assert!(!state.exists());
    }
}

#[test]
fn ui_restarts_only_an_owned_refused_socket_and_never_replaces_an_unexpected_path() {
    use std::os::unix::net::UnixListener;
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state");
    architect::ipc::private_directory(&state).unwrap();
    let socket = state.join("architect.sock");
    let stale = UnixListener::bind(&socket).unwrap();
    std::fs::set_permissions(&socket, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
    drop(stale);
    let mut child = architect::tui::connect_or_start_daemon(
        &state,
        std::path::Path::new(env!("CARGO_BIN_EXE_antislop")),
    )
    .unwrap()
    .unwrap();
    struct Cleanup<'a>(&'a mut std::process::Child);
    impl Drop for Cleanup<'_> {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let _cleanup = Cleanup(&mut child);
    use architect::tui::backend::{Backend, IpcBackend};
    assert!(IpcBackend::new(&state).list_runs().unwrap().is_empty());
    let other = root.path().join("unexpected");
    architect::ipc::private_directory(&other).unwrap();
    std::fs::write(other.join("architect.sock"), "user file, not a socket").unwrap();
    let error = architect::tui::connect_or_start_daemon(
        &other,
        std::path::Path::new(env!("CARGO_BIN_EXE_antislop")),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("unsafe UI socket path"));
    assert_eq!(
        std::fs::read_to_string(other.join("architect.sock")).unwrap(),
        "user file, not a socket"
    );
    assert!(!other.join("state.db").exists());
}

#[test]
fn ui_does_not_restart_a_listening_daemon_on_protocol_or_application_rejection() {
    use std::{
        io::{BufRead, Write},
        os::unix::net::UnixListener,
    };
    for wrong_protocol in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state");
        architect::ipc::private_directory(&state).unwrap();
        let socket = state.join("architect.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::os::unix::fs::PermissionsExt::from_mode(0o600))
            .unwrap();
        let responder = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            std::io::BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let request: architect::ipc::Request = serde_json::from_str(&line).unwrap();
            assert!(request.idempotency_key.is_none());
            let response = architect::ipc::Response {
                version: if wrong_protocol {
                    architect::ipc::PROTOCOL_VERSION + 1
                } else {
                    architect::ipc::PROTOCOL_VERSION
                },
                request_id: request.request_id,
                data: None,
                error: Some("controlled daemon rejection".into()),
            };
            writeln!(stream, "{}", serde_json::to_string(&response).unwrap()).unwrap();
        });
        let error = architect::tui::connect_or_start_daemon(
            &state,
            std::path::Path::new(env!("CARGO_BIN_EXE_antislop")),
        )
        .unwrap_err()
        .to_string();
        responder.join().unwrap();
        assert!(
            error.contains(if wrong_protocol {
                "mismatched IPC response"
            } else {
                "daemon rejected UI connection"
            }),
            "{error}"
        );
        assert!(!state.join("state.db").exists());
        assert!(
            !state.join("daemon.lock").exists(),
            "must not start another daemon for a rejected live response"
        );
    }
}

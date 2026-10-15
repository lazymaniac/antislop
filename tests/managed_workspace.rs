use architect::{
    config::WorkspaceMode,
    domain::*,
    id,
    runtime::{Command, Runtime},
    storage::Store,
    workspace::managed,
};
use std::{
    collections::BTreeSet,
    path::Path,
    process::Command as Process,
    sync::Arc,
    time::{Duration, Instant},
};
fn git(root: &Path, args: &[&str]) -> Vec<u8> {
    let o = Process::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@localhost",
            "-c",
            "commit.gpgSign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    o.stdout
}
struct Fixture {
    root: tempfile::TempDir,
    runtime: Arc<Runtime>,
    run: Run,
}
impl Fixture {
    fn new(with_git: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let state = root.path().join("state");
        std::fs::create_dir(&source).unwrap();
        std::fs::create_dir(&state).unwrap();
        std::fs::write(source.join("input.txt"), "base").unwrap();
        if with_git {
            git(&source, &["init", "-b", "main"]);
            git(&source, &["add", "input.txt"]);
            git(&source, &["commit", "-m", "base"]);
        }
        let mut spec: RunSpec =
            serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
        spec.workspace = source.to_str().unwrap().into();
        spec.input_files = vec!["input.txt".into()];
        spec.editable_files.clear();
        spec.check_command.clear();
        let mut config = architect::runtime::intake::bootstrap("codex".into())
            .unwrap()
            .config;
        config.workspace.mode = WorkspaceMode::Worktree;
        spec.config = Some(config.pin().unwrap());
        let runtime =
            Arc::new(Runtime::new(Store::open(&state.join("state.db")).unwrap()).unwrap());
        let run = runtime.dispatch(&id(), Command::Create { spec }).unwrap();
        Self { root, runtime, run }
    }
    fn state(&self) -> std::path::PathBuf {
        self.root.path().join("state")
    }
    fn wait(&self) -> Run {
        let end = Instant::now() + Duration::from_secs(20);
        loop {
            let r = self
                .runtime
                .store
                .lock()
                .unwrap()
                .get(&self.run.id)
                .unwrap();
            if r.status != Status::Running
                && architect::recovery::WorkspaceLease::acquire(&r.spec.workspace).is_ok()
            {
                return r;
            }
            assert!(Instant::now() < end, "{:?}", r.diagnostic);
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    fn prepare(&self, selected: BTreeSet<String>) -> Run {
        let p = managed::plan(&self.run, &self.state(), &selected).unwrap();
        self.runtime
            .dispatch(
                &id(),
                Command::PrepareWorkspace {
                    run_id: self.run.id.clone(),
                    selected,
                    scope_hash: p.scope_hash,
                },
            )
            .unwrap();
        self.wait()
    }
    fn inspect(&self) -> Run {
        self.runtime
            .dispatch(
                &id(),
                Command::InspectWorkspace {
                    run_id: self.run.id.clone(),
                },
            )
            .unwrap();
        self.wait()
    }
}

#[test]
fn background_workspace_inspection_has_no_candidate_effect_and_its_proposal_is_explicitly_approved()
{
    let f = Fixture::new(false);
    let inspected = f.inspect();
    assert_eq!(
        inspected.workspace_inspection.as_ref().unwrap().state,
        architect::runtime::workspace::InspectionState::Completed,
        "{:?}",
        inspected.diagnostic
    );
    assert!(!f.state().join("workspaces").exists());
    assert!(inspected.approvals.is_empty());
    let plan = managed::pending(&inspected, &f.state()).unwrap();
    assert_eq!(plan.selectors, ["input.txt".into()].into());
    assert_eq!(plan.selected, ["input.txt".into()].into());
    f.runtime
        .dispatch(
            &id(),
            Command::PrepareWorkspace {
                run_id: f.run.id.clone(),
                selected: plan.selectors.clone(),
                scope_hash: plan.scope_hash.clone(),
            },
        )
        .unwrap();
    let prepared = f.wait();
    assert_eq!(
        prepared.managed_workspace.as_ref().unwrap().state,
        managed::State::Captured,
        "{:?}",
        prepared.diagnostic
    );
    let scope = managed::activation_scope(&prepared).unwrap();
    f.runtime
        .dispatch(
            &id(),
            Command::ActivateWorkspace {
                run_id: f.run.id.clone(),
                scope_hash: scope,
            },
        )
        .unwrap();
    let active = f.wait();
    assert_eq!(
        active.managed_workspace.as_ref().unwrap().state,
        managed::State::Active,
        "{:?}",
        active.diagnostic
    );
    assert_eq!(active.input_snapshot["input.txt"], "base");
}

#[test]
fn source_drift_after_inspection_does_not_copy_a_new_version_under_the_old_approval() {
    let f = Fixture::new(false);
    let inspected = f.inspect();
    let plan = inspected.workspace_plan.unwrap();
    std::fs::write(
        Path::new(&f.run.spec.workspace).join("input.txt"),
        "user newer version",
    )
    .unwrap();
    f.runtime
        .dispatch(
            &id(),
            Command::PrepareWorkspace {
                run_id: f.run.id.clone(),
                selected: plan.selectors,
                scope_hash: plan.scope_hash,
            },
        )
        .unwrap();
    let refused = f.wait();
    assert_eq!(
        refused.managed_workspace.as_ref().unwrap().state,
        managed::State::Unknown
    );
    assert!(!Path::new(&refused.managed_workspace.as_ref().unwrap().root).exists());
    assert_eq!(
        std::fs::read(Path::new(&f.run.spec.workspace).join("input.txt")).unwrap(),
        b"user newer version"
    );
}

#[test]
fn corrupted_private_candidate_is_not_activated_and_task_sources_remain_the_original_version() {
    let f = Fixture::new(true);
    let prepared = f.prepare(BTreeSet::new());
    let scope = managed::activation_scope(&prepared).unwrap();
    let candidate = &prepared
        .managed_workspace
        .as_ref()
        .unwrap()
        .receipt
        .as_ref()
        .unwrap()
        .project_root;
    std::fs::write(
        Path::new(candidate).join("input.txt"),
        "corrupted private input",
    )
    .unwrap();
    f.runtime
        .dispatch(
            &id(),
            Command::ActivateWorkspace {
                run_id: f.run.id.clone(),
                scope_hash: scope,
            },
        )
        .unwrap();
    let refused = f.wait();
    assert_eq!(
        refused.managed_workspace.as_ref().unwrap().state,
        managed::State::Captured
    );
    assert_eq!(
        refused.workspace_activation_attempt.as_ref().unwrap().state,
        architect::runtime::workspace::InspectionState::Failed
    );
    assert_eq!(refused.spec.workspace, f.run.spec.workspace);
    assert_eq!(refused.input_snapshot, f.run.input_snapshot);
    assert_eq!(refused.spec_revision, f.run.spec_revision);
    assert!(!refused.reconciliation_required);
}

#[test]
fn activating_a_file_scoped_task_baselines_the_complete_candidate_without_false_added_files() {
    let f = Fixture::new(true);
    let source = Path::new(&f.run.spec.workspace);
    std::fs::write(source.join("other.txt"), "unchanged repository file\n").unwrap();
    git(source, &["add", "other.txt"]);
    git(source, &["commit", "-m", "other project file"]);
    let prepared = f.prepare(BTreeSet::new());
    f.runtime
        .dispatch(
            &id(),
            Command::ActivateWorkspace {
                run_id: f.run.id.clone(),
                scope_hash: managed::activation_scope(&prepared).unwrap(),
            },
        )
        .unwrap();
    let active = f.wait();
    let project = Path::new(&active.spec.workspace);
    assert_eq!(active.spec.input_files, vec!["input.txt"]);
    assert_eq!(active.diff_baseline()["input.txt"], "base");
    assert!(
        architect::workspace::compute_diff(project, active.diff_baseline())
            .unwrap()
            .files
            .is_empty()
    );
    std::fs::write(project.join("other.txt"), "actual new candidate content\n").unwrap();
    let diff = architect::workspace::compute_diff(project, active.diff_baseline()).unwrap();
    assert_eq!(diff.files.len(), 1);
    assert_eq!(diff.files[0].path, "other.txt");
    assert_eq!(
        diff.files[0].status,
        architect::workspace::DiffFileStatus::Modified
    );
}

#[test]
fn interrupted_read_only_workspace_inspection_and_activation_restart_without_replaying_or_claiming_project_effects()
 {
    let f = Fixture::new(true);
    let mut inspecting = f.run.clone();
    inspecting.status = Status::Running;
    inspecting.workspace_inspection = Some(architect::runtime::workspace::Inspection {
        state: architect::runtime::workspace::InspectionState::Running,
        previous_status: Status::AwaitingStart,
        diagnostic: None,
    });
    f.runtime
        .store
        .lock()
        .unwrap()
        .save(&inspecting, "fixture_interrupted_inspection", None)
        .unwrap();
    let reopened = Runtime::new(Store::open(&f.state().join("state.db")).unwrap()).unwrap();
    let recovered = reopened.store.lock().unwrap().get(&f.run.id).unwrap();
    assert_eq!(recovered.status, Status::AwaitingStart);
    assert!(!recovered.reconciliation_required);
    assert_eq!(
        recovered.workspace_inspection.as_ref().unwrap().state,
        architect::runtime::workspace::InspectionState::Interrupted
    );
    assert!(!f.state().join("workspaces").exists());
    let mut captured = f.prepare(BTreeSet::new());
    captured.status = Status::Running;
    captured.workspace_activation_attempt = Some(architect::runtime::workspace::Inspection {
        state: architect::runtime::workspace::InspectionState::Running,
        previous_status: Status::AwaitingStart,
        diagnostic: None,
    });
    f.runtime
        .store
        .lock()
        .unwrap()
        .save(&captured, "fixture_interrupted_activation", None)
        .unwrap();
    let reopened = Runtime::new(Store::open(&f.state().join("state.db")).unwrap()).unwrap();
    let recovered = reopened.store.lock().unwrap().get(&f.run.id).unwrap();
    assert_eq!(recovered.status, Status::AwaitingStart);
    assert!(!recovered.reconciliation_required);
    assert_eq!(
        recovered.managed_workspace.as_ref().unwrap().state,
        managed::State::Captured
    );
    assert_eq!(recovered.spec.workspace, f.run.spec.workspace);
    assert_eq!(
        recovered
            .workspace_activation_attempt
            .as_ref()
            .unwrap()
            .state,
        architect::runtime::workspace::InspectionState::Interrupted
    );
}
#[test]
fn private_git_workspace_preserves_source_and_activation_creates_new_scoped_revision() {
    let f = Fixture::new(true);
    let source = Path::new(&f.run.spec.workspace);
    let index = std::fs::read(source.join(".git/index")).unwrap();
    let head = git(source, &["rev-parse", "HEAD"]);
    let prepared = f.prepare(BTreeSet::new());
    let w = prepared.managed_workspace.as_ref().unwrap();
    assert_eq!(
        w.state,
        managed::State::Captured,
        "{:?}",
        prepared.diagnostic
    );
    let receipt = w.receipt.as_ref().unwrap();
    assert_eq!(
        std::fs::read_to_string(Path::new(&receipt.project_root).join("input.txt")).unwrap(),
        "base"
    );
    assert_eq!(std::fs::read(source.join(".git/index")).unwrap(), index);
    assert_eq!(git(source, &["rev-parse", "HEAD"]), head);
    assert!(!source.join(".git/worktrees").exists());
    let policy = f.run.spec.config.as_ref().unwrap().config.policy.clone();
    let scope = managed::activation_scope(&prepared).unwrap();
    let _started = f
        .runtime
        .dispatch(
            &id(),
            Command::ActivateWorkspace {
                run_id: prepared.id.clone(),
                scope_hash: scope,
            },
        )
        .unwrap();
    let activated = f.wait();
    assert_eq!(activated.spec_revision, prepared.spec_revision + 1);
    assert_ne!(activated.spec.workspace, f.run.spec.workspace);
    assert_eq!(
        activated.managed_workspace.as_ref().unwrap().state,
        managed::State::Active
    );
    assert_eq!(
        activated.spec.config.as_ref().unwrap().config.policy,
        policy
    );
    assert!(activated.artifacts.is_empty() && activated.quality_gates.is_empty());
    assert_eq!(activated.input_snapshot, f.run.input_snapshot);
}

#[test]
fn whole_directory_inputs_move_to_git_and_nongit_candidates_with_bytes_links_and_empty_scopes_intact()
 {
    for with_git in [true, false] {
        let mut f = Fixture::new(with_git);
        let source = std::path::PathBuf::from(&f.run.spec.workspace);
        std::fs::create_dir(source.join("empty")).unwrap();
        std::fs::write(source.join("input.txt"), "accepted local input\n").unwrap();
        std::fs::write(source.join("binary.bin"), [0, 255, 42]).unwrap();
        std::os::unix::fs::symlink("input.txt", source.join("link")).unwrap();
        let mut spec = f.run.spec.clone();
        spec.input_files = vec![".".into(), "empty".into(), "input.txt".into()];
        f.run = f
            .runtime
            .dispatch(
                &id(),
                Command::Revise {
                    run_id: f.run.id.clone(),
                    spec,
                    expected_revision: f.run.spec_revision,
                },
            )
            .unwrap();
        let baseline = f.run.diff_baseline().clone();
        let prepared = f.prepare([".".into()].into());
        assert_eq!(
            prepared.managed_workspace.as_ref().unwrap().state,
            managed::State::Captured,
            "{:?}",
            prepared.diagnostic
        );
        let scope = managed::activation_scope(&prepared).unwrap();
        let _started = f
            .runtime
            .dispatch(
                &id(),
                Command::ActivateWorkspace {
                    run_id: prepared.id.clone(),
                    scope_hash: scope,
                },
            )
            .unwrap();
        let active = f.wait();
        let candidate = Path::new(&active.spec.workspace);
        assert_eq!(
            std::fs::read(candidate.join("binary.bin")).unwrap(),
            [0, 255, 42]
        );
        assert_eq!(
            std::fs::read_link(candidate.join("link")).unwrap(),
            std::path::PathBuf::from("input.txt")
        );
        assert!(candidate.join("empty").is_dir());
        assert_eq!(active.diff_baseline(), &baseline);
        assert_eq!(active.spec.input_files, vec![".", "empty", "input.txt"]);
        assert_eq!(
            active.input_snapshot,
            architect::workspace::read_inputs(candidate, &active.spec.input_files).unwrap()
        );
        assert!(
            architect::workspace::compute_diff(candidate, active.diff_baseline())
                .unwrap()
                .files
                .is_empty()
        );
        assert_eq!(
            std::fs::read(source.join("input.txt")).unwrap(),
            b"accepted local input\n"
        );
        assert_eq!(source.join(".git").exists(), with_git);
    }
}

#[test]
fn directory_candidate_requires_task_owned_dirty_changes_and_refuses_changed_sources() {
    let mut f = Fixture::new(true);
    let source = std::path::PathBuf::from(&f.run.spec.workspace);
    std::fs::write(source.join("input.txt"), "accepted dirty input").unwrap();
    let mut spec = f.run.spec.clone();
    spec.input_files = vec![".".into()];
    f.run = f
        .runtime
        .dispatch(
            &id(),
            Command::Revise {
                run_id: f.run.id.clone(),
                spec,
                expected_revision: f.run.spec_revision,
            },
        )
        .unwrap();
    assert!(managed::plan(&f.run, &f.state(), &BTreeSet::new()).is_err());
    let plan = managed::plan(&f.run, &f.state(), &[".".into()].into()).unwrap();
    assert_eq!(plan.selected, ["input.txt".into()].into());
    std::fs::write(source.join("input.txt"), "concurrent user change").unwrap();
    assert!(
        f.runtime
            .dispatch(
                &id(),
                Command::PrepareWorkspace {
                    run_id: f.run.id.clone(),
                    selected: [".".into()].into(),
                    scope_hash: plan.scope_hash
                }
            )
            .is_err()
    );
    assert!(!f.state().join("workspaces").exists());
}

#[test]
fn subproject_and_absolute_directory_inputs_are_rebound_inside_the_private_candidate() {
    let mut f = Fixture::new(false);
    git(f.root.path(), &["init", "-b", "main"]);
    std::fs::write(f.root.path().join("sibling.txt"), "outside task scope").unwrap();
    git(f.root.path(), &["add", "source/input.txt", "sibling.txt"]);
    git(f.root.path(), &["commit", "-m", "parent repository"]);
    let mut spec = f.run.spec.clone();
    spec.input_files = vec![spec.workspace.clone()];
    f.run = f
        .runtime
        .dispatch(
            &id(),
            Command::Revise {
                run_id: f.run.id.clone(),
                spec,
                expected_revision: f.run.spec_revision,
            },
        )
        .unwrap();
    let prepared = f.prepare([".".into()].into());
    assert_eq!(
        prepared.managed_workspace.as_ref().unwrap().state,
        managed::State::Captured,
        "{:?}",
        prepared.diagnostic
    );
    let _started = f
        .runtime
        .dispatch(
            &id(),
            Command::ActivateWorkspace {
                run_id: prepared.id.clone(),
                scope_hash: managed::activation_scope(&prepared).unwrap(),
            },
        )
        .unwrap();
    let active = f.wait();
    assert_eq!(active.spec.input_files, vec!["."]);
    assert_eq!(active.input_snapshot.len(), 1);
    assert!(Path::new(&active.spec.workspace).ends_with("candidate/source"));
    assert!(!active.input_snapshot["."].contains("outside task scope"));
    assert!(
        architect::workspace::compute_diff(
            Path::new(&active.spec.workspace),
            active.diff_baseline()
        )
        .unwrap()
        .files
        .is_empty()
    );
}
#[test]
fn nongit_selected_inputs_and_local_changes_are_copied_without_initializing_user_source() {
    let f = Fixture::new(false);
    assert!(managed::plan(&f.run, &f.state(), &BTreeSet::new()).is_err());
    let prepared = f.prepare(BTreeSet::from(["input.txt".into()]));
    assert_eq!(
        prepared.managed_workspace.as_ref().unwrap().state,
        managed::State::Captured,
        "{:?}",
        prepared.diagnostic
    );
    assert!(!Path::new(&f.run.spec.workspace).join(".git").exists());
}
#[test]
fn failed_intent_commit_starts_no_git_and_failed_result_checkpoint_restores_without_replay() {
    let f = Fixture::new(true);
    let db = rusqlite::Connection::open(f.state().join("state.db")).unwrap();
    db.execute_batch("CREATE TRIGGER deny_workspace_intent BEFORE INSERT ON events WHEN NEW.kind='workspace_prepare_intent' BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    let p = managed::plan(&f.run, &f.state(), &BTreeSet::new()).unwrap();
    let c = Command::PrepareWorkspace {
        run_id: f.run.id.clone(),
        selected: BTreeSet::new(),
        scope_hash: p.scope_hash,
    };
    let key = id();
    assert!(f.runtime.dispatch(&key, c.clone()).is_err());
    assert!(!f.state().join("workspaces").exists());
    db.execute_batch("DROP TRIGGER deny_workspace_intent; CREATE TRIGGER deny_workspace_finish BEFORE INSERT ON events WHEN NEW.kind='workspace_prepare_finished' BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    f.runtime.dispatch(&key, c).unwrap();
    let unknown = f.wait();
    assert_eq!(
        unknown.managed_workspace.as_ref().unwrap().state,
        managed::State::Unknown
    );
    let before = managed::observed_result(unknown.managed_workspace.as_ref().unwrap()).unwrap();
    let scope = managed::recovery_scope(&unknown).unwrap();
    db.execute_batch("DROP TRIGGER deny_workspace_finish")
        .unwrap();
    let reopened =
        Arc::new(Runtime::new(Store::open(&f.state().join("state.db")).unwrap()).unwrap());
    let restored = reopened
        .dispatch(
            &id(),
            Command::RestoreWorkspaceResult {
                run_id: unknown.id.clone(),
                scope_hash: scope,
            },
        )
        .unwrap();
    let after = restored
        .managed_workspace
        .as_ref()
        .unwrap()
        .receipt
        .as_ref()
        .unwrap();
    assert_eq!(after.commit, before.commit);
    assert_eq!(after.candidate, before.candidate);
    assert!(!restored.reconciliation_required);
    assert_eq!(
        std::fs::read_dir(f.state().join("workspaces"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn daemon_and_chat_guide_inspection_preparation_activation_and_queue_planning_without_starting_agents()
 {
    use architect::{
        ipc,
        tui::{
            app::{App, Effect, View},
            backend::{Action, Backend, IpcBackend},
        },
    };
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    use std::os::unix::fs::PermissionsExt;
    struct Daemon(std::process::Child);
    impl Drop for Daemon {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let f = Fixture::new(false);
    std::fs::set_permissions(f.state(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut conversation = f.run.clone();
    conversation.chat_origin = true; // Fixture represents an already accepted chat contract.
    f.runtime
        .store
        .lock()
        .unwrap()
        .save(&conversation, "fixture_accepted_chat_contract", None)
        .unwrap();
    let mut daemon = Daemon(
        Process::new(env!("CARGO_BIN_EXE_antislop"))
            .arg("daemon")
            .arg(f.state())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if ipc::call(
            &f.state(),
            &ipc::Request {
                version: ipc::PROTOCOL_VERSION,
                request_id: id(),
                idempotency_key: None,
                operation: ipc::Operation::List,
            },
        )
        .is_ok()
        {
            break;
        }
        if let Some(status) = daemon.0.try_wait().unwrap() {
            use std::io::Read;
            let mut diagnostic = String::new();
            daemon
                .0
                .stderr
                .take()
                .unwrap()
                .read_to_string(&mut diagnostic)
                .unwrap();
            panic!("fixture daemon startup failed {status}: {diagnostic}");
        }
        assert!(
            Instant::now() < deadline,
            "fixture daemon startup observation expired"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut backend = IpcBackend::new(&f.state());
    let mut app = App::default();
    app.update(backend.snapshot(&f.run.id).unwrap());
    assert!(matches!(
        app.next_approval(),
        Some((Action::InspectWorkspace, _))
    ));
    for expected in [0, 1, 2] {
        app.handle(Event::Key(KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::NONE,
        )));
        assert!(
            matches!(&app.view,View::Confirm {action,..} if match expected {
                0=>matches!(action,Action::InspectWorkspace),
                1=>matches!(action,Action::PrepareWorkspace {..}),
                _=>matches!(action,Action::ActivateWorkspace {..}),
            })
        );
        if expected == 1 {
            assert!(
                matches!(&app.view,View::Confirm {details,..} if details.contains("input.txt") && details.contains("included_local_changes"))
            );
        }
        let effects = app.handle(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        let Effect::Action(action) = effects.into_iter().next().unwrap() else {
            panic!("missing workspace control intent")
        };
        backend.act(&f.run.id, action).unwrap();
        let run = f.wait();
        assert!(run.session_ledger.sessions.is_empty() && run.artifacts.is_empty());
        if expected == 0 {
            assert!(!f.state().join("workspaces").exists());
        }
        app.update(backend.snapshot(&f.run.id).unwrap());
    }
    let active = f.wait();
    assert_eq!(
        active.managed_workspace.as_ref().unwrap().state,
        managed::State::Active,
        "{:?}",
        active.diagnostic
    );
    assert!(!app.snapshot.workspace_required);
    assert!(matches!(
        app.next_approval(),
        Some((Action::Orchestrate { .. }, _))
    ));
    let messages: Vec<_> = active
        .communication_ledger
        .messages
        .values()
        .filter(|m| m.recipient_agent_id == "orchestrator")
        .collect();
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0].sender,
        architect::communication::Sender::Runtime
    );
    assert_eq!(
        messages[0].state,
        architect::communication::DeliveryState::QueuedNextTurn
    );
    assert!(active.session_ledger.sessions.is_empty());
}

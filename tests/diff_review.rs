use architect::{
    config::{Config, ProviderKind, WorkspaceMode},
    domain::{CheckStatus, Run, RunSpec, Status},
    id,
    runtime::{Command, Runtime, diff_review::State},
    storage::Store,
};
use std::{
    os::unix::fs::PermissionsExt,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

struct Fixture {
    root: tempfile::TempDir,
    runtime: Arc<Runtime>,
    run: Run,
}
impl Fixture {
    fn new(mode: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let workspace = root.path().join("project");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::write(workspace.join("main.rs"), "fn main() {}\n").unwrap();
        let binary = root.path().join("review-cli");
        let script = r#"#!/usr/bin/env python3
import json,sys,pathlib,time
base=pathlib.Path(__BASE__)
mode=__MODE__
if '--version' in sys.argv: print('2.1.296 (Claude Code)'); sys.exit(0)
if '--help' in sys.argv: print('--safe-mode --strict-mcp-config --tools --permission-prompts --session-id --resume --json-schema --disable-slash-commands'); sys.exit(0)
prompt=sys.stdin.read()
context=json.JSONDecoder().raw_decode(prompt.split('Immutable diff review context: ',1)[1])[0]
native=sys.argv[sys.argv.index('--session-id')+1]
with (base/'calls.jsonl').open('a') as f: f.write(json.dumps(dict(native=native,argv=sys.argv,context=context))+'\n')
print(json.dumps(dict(type='system',subtype='init',session_id=native,tools=['StructuredOutput'])),flush=True)
(base/'started').write_text('observed')
if mode == 'wait':
    deadline=time.monotonic()+30
    while not (base/'release').exists() and time.monotonic()<deadline: time.sleep(.02)
out=dict(diff_hash=context['diff_hash'],input_hash=context['input_hash'],spec_revision=context['spec_revision'],status='fail' if mode=='reject' else 'pass',method='Actual fixture inspection of supplied source and diff',rationale='Transport fixture; no real-model quality claim',findings=['Concrete fixture defect at main.rs:1'] if mode=='reject' else [],agent_messages=[])
if mode=='wrong_hash': out['diff_hash']='forged'
if mode=='contradictory': out['findings']=['Unresolved defect despite pass']
if mode=='malformed': out={'status':'pass'}
print(json.dumps(dict(type='result',subtype='success',is_error=False,session_id=native,structured_output=out)),flush=True)
"#
            .replace("__BASE__", &serde_json::to_string(root.path().to_str().unwrap()).unwrap())
            .replace("__MODE__", &serde_json::to_string(mode).unwrap());
        std::fs::write(&binary, script).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut config = Config::from_toml(include_str!("../examples/architect.toml")).unwrap();
        config.workspace.mode = WorkspaceMode::InPlace;
        config
            .providers
            .get_mut(&ProviderKind::Claude)
            .unwrap()
            .binary = binary;
        for profile in config.profiles.values_mut() {
            profile.provider = ProviderKind::Claude;
        }
        let mut spec: RunSpec =
            serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
        spec.workspace = workspace.to_string_lossy().into();
        spec.input_files = vec!["main.rs".into()];
        spec.editable_files.clear();
        spec.check_command.clear();
        spec.config = Some(config.pin().unwrap());
        let runtime =
            Arc::new(Runtime::new(Store::open(&root.path().join("state.db")).unwrap()).unwrap());
        let run = runtime.dispatch(&id(), Command::Create { spec }).unwrap();
        std::fs::write(
            workspace.join("main.rs"),
            "fn main() { println!(\"candidate\"); }\n",
        )
        .unwrap();
        Self { root, runtime, run }
    }
    fn current(&self) -> Run {
        self.runtime
            .store
            .lock()
            .unwrap()
            .get(&self.run.id)
            .unwrap()
    }
    fn start(&self) -> Run {
        let current = self.current();
        self.runtime
            .dispatch(
                &id(),
                Command::ReviewDiff {
                    run_id: current.id.clone(),
                    scope_hash: current.diff_review_scope().unwrap(),
                },
            )
            .unwrap()
    }
    fn wait(&self) -> Run {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let run = self.current();
            if run
                .diff_reviews
                .last()
                .is_some_and(|r| r.state != State::Running)
                && architect::recovery::WorkspaceLease::acquire(&run.spec.workspace).is_ok()
            {
                return run;
            }
            assert!(Instant::now() < deadline, "{:?}", run.diagnostic);
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    fn started(&self) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !self.root.path().join("started").exists() {
            assert!(Instant::now() < deadline, "{:?}", self.current().diagnostic);
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

#[test]
fn observed_diff_review_preserves_task_inputs_stage_attempt_and_exposes_native_findings() {
    for mode in ["pass", "reject"] {
        let f = Fixture::new(mode);
        let started = f.start();
        assert!(started.reviews.is_empty());
        assert!(started.diff_reviews[0].response.is_none());
        assert_eq!(started.attempt, f.run.attempt);
        let run = f.wait();
        let record = &run.diff_reviews[0];
        assert_eq!(record.state, State::Completed, "{:?}", run.diagnostic);
        assert_eq!(
            record.response.as_ref().unwrap().status,
            if mode == "pass" {
                CheckStatus::Pass
            } else {
                CheckStatus::Fail
            }
        );
        let session = &run.session_ledger.sessions[record.session_id.as_ref().unwrap()];
        assert!(session.native_observed);
        assert_eq!(session.scope.input_hash, record.input_hash);
        assert_eq!(session.scope.role, architect::config::RoleKind::Verifier);
        assert_eq!(run.input_snapshot, f.run.input_snapshot);
        assert_eq!(run.diff_baseline(), f.run.diff_baseline());
        assert_eq!(run.stage, f.run.stage);
        assert_eq!(run.status, f.run.status);
        assert_eq!(run.attempt, f.run.attempt);
        assert!(run.reviews.is_empty() && run.quality_gates.is_empty());
        let snapshot = architect::tui::model::Snapshot::from_document(
            &serde_json::to_value(&run).unwrap(),
            &serde_json::json!({}),
        );
        assert!(snapshot.run_details["Diff"].contains("Independent candidate reviews"));
        assert!(snapshot.agents.iter().any(|a| a.role == "Diff reviewer"));
        let elements = architect::comments::elements(&run);
        let id = format!("diff-review:{}", record.id);
        assert_eq!(
            elements
                .iter()
                .filter(
                    |e| e.reference.kind == architect::comments::ElementKind::Agent
                        && e.reference.id == id
                )
                .count(),
            1
        );
    }
}

#[test]
fn forged_incomplete_and_contradictory_review_responses_never_become_passes() {
    for mode in ["wrong_hash", "contradictory", "malformed"] {
        let f = Fixture::new(mode);
        f.start();
        let run = f.wait();
        assert_eq!(run.diff_reviews[0].state, State::Failed);
        assert!(run.diff_reviews[0].response.is_none());
        assert_eq!(run.status, f.run.status);
        assert!(!run.reconciliation_required);
        assert!(run.reviews.is_empty() && run.quality_gates.is_empty());
        assert_eq!(run.input_snapshot, f.run.input_snapshot);
        let session =
            &run.session_ledger.sessions[run.diff_reviews[0].session_id.as_ref().unwrap()];
        assert!(
            session
                .current_turn_id
                .as_ref()
                .is_some_and(|id| run.session_ledger.turns[id].output.is_some())
        );
    }
}

#[test]
fn candidate_and_comment_drift_keep_a_stale_versioned_response_without_adopting_sources() {
    for comments in [false, true] {
        let f = Fixture::new("wait");
        f.start();
        f.started();
        if comments {
            f.runtime
                .dispatch(
                    &id(),
                    Command::AddDiffComment {
                        run_id: f.run.id.clone(),
                        path: "main.rs".into(),
                        line_number: Some(1),
                        text: "New concern while review is in flight".into(),
                        author: "local_user".into(),
                    },
                )
                .unwrap();
        } else {
            std::fs::write(
                Path::new(&f.run.spec.workspace).join("main.rs"),
                "user changed candidate\n",
            )
            .unwrap();
        }
        std::fs::write(f.root.path().join("release"), "").unwrap();
        let run = f.wait();
        assert_eq!(run.diff_reviews[0].state, State::Stale);
        assert!(run.diff_reviews[0].response.is_some());
        assert_eq!(run.input_snapshot, f.run.input_snapshot);
        assert!(!run.reconciliation_required);
        assert_eq!(run.status, Status::AwaitingStart);
    }
}

#[test]
fn failed_review_intent_commit_starts_no_provider_and_busy_or_stale_scopes_are_refused() {
    let f = Fixture::new("pass");
    let db = rusqlite::Connection::open(f.root.path().join("state.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_review BEFORE INSERT ON events WHEN NEW.kind='diff_review_intent' BEGIN SELECT RAISE(ABORT,'injected intent failure'); END;").unwrap();
    let command = Command::ReviewDiff {
        run_id: f.run.id.clone(),
        scope_hash: f.run.diff_review_scope().unwrap(),
    };
    assert!(f.runtime.dispatch(&id(), command).is_err());
    assert!(!f.root.path().join("calls.jsonl").exists());
    assert!(f.current().diff_reviews.is_empty());
    assert!(
        f.runtime
            .dispatch(
                &id(),
                Command::ReviewDiff {
                    run_id: f.run.id.clone(),
                    scope_hash: "stale".into()
                }
            )
            .is_err()
    );
    db.execute_batch("DROP TRIGGER refuse_review;").unwrap();
    f.start();
    assert!(
        f.runtime
            .dispatch(
                &id(),
                Command::ReviewDiff {
                    run_id: f.run.id.clone(),
                    scope_hash: f.run.diff_review_scope().unwrap()
                }
            )
            .is_err()
    );
    assert_eq!(f.wait().diff_reviews[0].state, State::Completed);
}

#[test]
fn cancelled_diff_review_retains_observed_turn_and_restart_does_not_replay() {
    let f = Fixture::new("wait");
    f.start();
    f.started();
    f.runtime
        .dispatch(
            &id(),
            Command::Cancel {
                run_id: f.run.id.clone(),
            },
        )
        .unwrap();
    let run = f.wait();
    assert_eq!(run.status, Status::Cancelled);
    assert_eq!(run.diff_reviews[0].state, State::Interrupted);
    assert!(run.diff_reviews[0].response.is_none());
    let calls = std::fs::read(f.root.path().join("calls.jsonl")).unwrap();
    let restored = Runtime::new(Store::open(&f.root.path().join("state.db")).unwrap()).unwrap();
    assert_eq!(
        restored
            .store
            .lock()
            .unwrap()
            .get(&run.id)
            .unwrap()
            .diff_reviews[0]
            .state,
        State::Interrupted
    );
    assert_eq!(
        std::fs::read(f.root.path().join("calls.jsonl")).unwrap(),
        calls
    );
}

#[test]
fn failed_outcome_commit_retains_completed_output_without_a_review_pass_and_restart_never_replays()
{
    let f = Fixture::new("pass");
    let db = rusqlite::Connection::open(f.root.path().join("state.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_outcome BEFORE INSERT ON events WHEN NEW.kind='diff_review_finished' BEGIN SELECT RAISE(ABORT,'injected outcome failure'); END;").unwrap();
    f.start();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let run = f.current();
        if !run.session_ledger.turns.is_empty()
            && run
                .session_ledger
                .turns
                .values()
                .all(|t| t.state == architect::sessions::TurnState::Completed)
            && architect::recovery::WorkspaceLease::acquire(&run.spec.workspace).is_ok()
        {
            break;
        }
        assert!(Instant::now() < deadline, "{:?}", run.diagnostic);
        std::thread::sleep(Duration::from_millis(20));
    }
    let run = f.current();
    assert_eq!(run.diff_reviews[0].state, State::Running);
    assert!(run.diff_reviews[0].response.is_none() && run.reviews.is_empty());
    db.execute_batch("DROP TRIGGER refuse_outcome;").unwrap();
    let calls = std::fs::read(f.root.path().join("calls.jsonl")).unwrap();
    let restored = Runtime::new(Store::open(&f.root.path().join("state.db")).unwrap()).unwrap();
    let run = restored.store.lock().unwrap().get(&run.id).unwrap();
    assert_eq!(run.diff_reviews[0].state, State::Interrupted);
    assert!(run.diff_reviews[0].response.is_none());
    assert_eq!(run.input_snapshot, f.run.input_snapshot);
    assert_eq!(
        std::fs::read(f.root.path().join("calls.jsonl")).unwrap(),
        calls
    );
}

#[test]
fn diff_comment_notifies_orchestrator_once_with_exact_comment_reference_without_starting_a_turn() {
    let f = Fixture::new("pass");
    let command = Command::AddDiffComment {
        run_id: f.run.id.clone(),
        path: "main.rs".into(),
        line_number: Some(1),
        text: "Check this regression".into(),
        author: "local_user".into(),
    };
    let key = id();
    let run = f.runtime.dispatch(&key, command.clone()).unwrap();
    assert_eq!(run.communication_ledger.messages.len(), 1);
    let comment = &run.diff_comments[0];
    let (id, message) = run.communication_ledger.messages.iter().next().unwrap();
    assert_eq!(message.recipient_agent_id, "orchestrator");
    assert!(run.message_bodies[id].contains(&comment.id));
    assert!(run.message_bodies[id].contains("main.rs:1"));
    assert!(!f.root.path().join("calls.jsonl").exists());
    assert_eq!(
        f.runtime
            .dispatch(&key, command)
            .unwrap()
            .communication_ledger
            .messages
            .len(),
        1
    );
}

#[test]
fn daemon_tui_and_cli_run_the_scoped_native_reviewer_and_show_its_actual_rejection() {
    use architect::{
        ipc,
        tui::{
            app::{App, Effect, View},
            backend::{Backend, IpcBackend},
        },
    };
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    struct Daemon(std::process::Child);
    impl Drop for Daemon {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let f = Fixture::new("reject");
    let state = f.root.path();
    let mut daemon = Daemon(
        std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
            .arg("daemon")
            .arg(state)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if ipc::call(
            state,
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
            panic!("fixture daemon stopped during startup ({status}): {diagnostic}");
        }
        assert!(
            Instant::now() < deadline,
            "fixture daemon startup observation expired"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut backend = IpcBackend::new(state);
    let mut app = App::default();
    app.update(backend.snapshot(&f.run.id).unwrap());
    app.view = View::Run { tab: 4 };
    app.handle(Event::Key(KeyEvent::new(
        KeyCode::Char('r'),
        KeyModifiers::NONE,
    )));
    assert!(
        matches!(&app.view,View::Confirm{details,..} if details.contains("Independent diff reviewer profile") && details.contains("main.rs"))
    );
    let effects = app.handle(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    let Effect::Action(action) = effects.into_iter().next().unwrap() else {
        panic!("missing review intent")
    };
    backend.act(&f.run.id, action).unwrap();
    let run = f.wait();
    assert_eq!(run.diff_reviews[0].state, State::Completed);
    assert_eq!(
        run.diff_reviews[0].response.as_ref().unwrap().status,
        CheckStatus::Fail
    );
    let snapshot = backend.snapshot(&run.id).unwrap();
    assert!(snapshot.run_details["Diff"].contains("Concrete fixture defect at main.rs:1"));
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
        .arg("diff")
        .arg(state)
        .arg(&run.id)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.contains("Independent candidate review")
            && text.contains("Concrete fixture defect at main.rs:1")
    );
    assert!(run.reviews.is_empty() && run.quality_gates.is_empty());
}

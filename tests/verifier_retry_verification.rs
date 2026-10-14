//! Independent verification-only recovery tests. All persisted edits are disposable fixtures.
use architect::{
    config::RoleKind,
    domain::*,
    hash, id,
    providers::{Provider, fake::FakeProvider},
    runtime::{Command, Runtime},
    sessions::OutputRef,
    sessions::TurnState,
    storage::Store,
};
use std::{
    process::Command as Process,
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};

fn read(runtime: &Arc<Runtime>, run: &Run) -> Run {
    runtime.store.lock().unwrap().get(&run.id).unwrap()
}
fn save(runtime: &Arc<Runtime>, run: &Run) {
    runtime
        .store
        .lock()
        .unwrap()
        .save(run, "independent_failed_verifier_fixture", None)
        .unwrap();
}
fn wait(runtime: &Arc<Runtime>, run: &Run) -> Run {
    let until = Instant::now() + Duration::from_secs(8);
    loop {
        let current = read(runtime, run);
        if current.status != Status::Running {
            return current;
        }
        assert!(Instant::now() < until, "verification worker timeout");
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn fixture(checker: bool) -> (tempfile::TempDir, Arc<Runtime>, Run) {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("pinned.txt"), "original").unwrap();
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().canonicalize().unwrap().to_string_lossy().into();
    spec.input_files = vec!["pinned.txt".into()];
    if checker {
        spec.check_command = vec!["python3".into(), "-c".into(), "from pathlib import Path; p=Path('checks'); p.write_text((p.read_text() if p.exists() else '')+'x')".into()];
    }
    let runtime =
        Arc::new(Runtime::new(Store::open(&root.path().join("state.db")).unwrap()).unwrap());
    let mut run = runtime.dispatch(&id(), Command::Create { spec }).unwrap();
    for stage in [Stage::Discovery, Stage::Planning, Stage::Execution] {
        assert_eq!(run.stage, stage);
        runtime
            .dispatch(
                &id(),
                Command::ApproveStart {
                    run_id: run.id.clone(),
                    scope_hash: run.scope_hash("start"),
                },
            )
            .unwrap();
        runtime
            .dispatch(
                &id(),
                Command::Advance {
                    run_id: run.id.clone(),
                },
            )
            .unwrap();
        run = wait(&runtime, &run);
        if stage != Stage::Execution {
            assert_eq!(run.status, Status::AwaitingResult, "{:?}", run.diagnostic);
            run = runtime
                .dispatch(
                    &id(),
                    Command::AcceptResult {
                        run_id: run.id.clone(),
                        scope_hash: run.scope_hash("accept"),
                    },
                )
                .unwrap();
        }
    }
    assert!(
        matches!(
            run.status,
            Status::AwaitingResult | Status::ChangesRequested
        ),
        "{:?}",
        run.diagnostic
    );
    // Model a transport failure after the genuine completed executor output was committed.
    // The production paths that create artifacts, provenance and ledger records ran above.
    let active = run.active_session.clone().unwrap();
    let session = run.session_ledger.sessions.get_mut(&active).unwrap();
    assert_eq!(session.scope.role, RoleKind::Verifier);
    session.state = TurnState::Failed;
    let turn = run
        .session_ledger
        .turns
        .get_mut(session.current_turn_id.as_ref().unwrap())
        .unwrap();
    turn.state = TurnState::Failed;
    turn.provider_completed = false;
    turn.output = None;
    turn.output_ref = None;
    let removed = run.reviews.pop().unwrap();
    run.provenance_reviews
        .remove(&hash(serde_json::to_vec(&removed).unwrap()));
    run.status = Status::Blocked;
    run.reconciliation_required = false;
    run.diagnostic =
        Some("Reconciled interrupted main verifier; executor output remains durable".into());
    save(&runtime, &run);
    (root, runtime, run)
}
fn retry(run: &Run) -> Command {
    let artifact = run
        .artifacts
        .iter()
        .rev()
        .find(|a| {
            a.stage == run.stage && a.attempt == run.attempt && a.spec_revision == run.spec_revision
        })
        .unwrap();
    Command::RetryVerification {
        run_id: run.id.clone(),
        artifact_hash: artifact.hash.clone(),
        scope_hash: run.scope_hash(&format!("verify:{}", artifact.hash)),
    }
}
#[test]
fn retry_is_verifier_only_same_attempt_and_idempotent() {
    let (root, runtime, before) = fixture(false);
    let key = id();
    let command = retry(&before);
    let started = runtime.dispatch(&key, command.clone()).unwrap();
    assert_eq!(started.status, Status::Running);
    runtime.dispatch(&key, command.clone()).unwrap();
    let after = wait(&runtime, &before);
    runtime.dispatch(&key, command).unwrap();
    assert_eq!(
        after.status,
        Status::AwaitingResult,
        "{:?}",
        after.diagnostic
    );
    assert!(!after.reconciliation_required);
    assert_eq!(
        (after.stage, after.attempt, after.spec_revision),
        (before.stage, before.attempt, before.spec_revision)
    );
    assert_eq!(
        serde_json::to_value(&after.artifacts).unwrap(),
        serde_json::to_value(&before.artifacts).unwrap()
    );
    let new_sessions: Vec<_> = after
        .session_ledger
        .sessions
        .iter()
        .filter(|(id, _)| !before.session_ledger.sessions.contains_key(*id))
        .collect();
    assert_eq!(new_sessions.len(), 1);
    assert_eq!(new_sessions[0].1.scope.role, RoleKind::Verifier);
    assert_eq!(new_sessions[0].1.scope.stage, before.stage);
    assert_eq!(new_sessions[0].1.state, TurnState::Completed);
    for (id, old) in &before.session_ledger.sessions {
        assert_eq!(&after.session_ledger.sessions[id], old);
    }
    assert_eq!(
        after.session_ledger.turns.len(),
        before.session_ledger.turns.len() + 1
    );
    assert_eq!(after.reviews.len(), before.reviews.len() + 1);
    assert_eq!(after.approvals.len(), before.approvals.len() + 1);
    assert!(
        after
            .approvals
            .last()
            .unwrap()
            .action
            .starts_with("verify:")
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("pinned.txt")).unwrap(),
        "original"
    );
    assert_eq!(
        after
            .evidence
            .iter()
            .filter(|e| e.source.starts_with("runner:"))
            .count(),
        0
    );
}
#[test]
fn retry_reuses_real_runner_without_rerunning_it_and_keeps_quality_rejection() {
    let (root, runtime, before) = fixture(true);
    let prior_runner = before
        .evidence
        .iter()
        .filter(|e| e.source.starts_with("runner:"))
        .count();
    let prior_effects = std::fs::read_to_string(root.path().join("checks")).unwrap();
    assert!(!prior_effects.is_empty());
    runtime.dispatch(&id(), retry(&before)).unwrap();
    let after = wait(&runtime, &before);
    assert_eq!(
        after.status,
        Status::ChangesRequested,
        "a synthetic review must not validate real runner evidence"
    );
    assert!(!after.reconciliation_required);
    assert_eq!(
        std::fs::read_to_string(root.path().join("checks")).unwrap(),
        prior_effects
    );
    assert_eq!(
        after
            .evidence
            .iter()
            .filter(|e| e.source.starts_with("runner:"))
            .count(),
        prior_runner
    );
    assert_eq!(after.artifacts.len(), before.artifacts.len());
    assert_eq!(
        (after.stage, after.attempt, after.spec_revision),
        (before.stage, before.attempt, before.spec_revision)
    );
}
#[test]
fn stale_scope_and_artifact_fail_without_approval_or_sessions() {
    let (_root, runtime, before) = fixture(false);
    for change in ["scope", "artifact"] {
        let mut command = retry(&before);
        if let Command::RetryVerification {
            scope_hash,
            artifact_hash,
            ..
        } = &mut command
        {
            if change == "scope" {
                *scope_hash = "stale".into();
            } else {
                *artifact_hash = hash("wrong artifact");
            }
        }
        assert!(runtime.dispatch(&id(), command).is_err());
        assert_eq!(
            serde_json::to_value(read(&runtime, &before)).unwrap(),
            serde_json::to_value(&before).unwrap()
        );
    }
}
#[test]
fn status_uncertainty_completed_verifier_and_wrong_role_cannot_retry() {
    let (_root, runtime, before) = fixture(false);
    for change in ["ready", "paused", "uncertain", "completed", "executor"] {
        let mut run = before.clone();
        match change {
            "ready" => run.status = Status::Ready,
            "paused" => run.status = Status::Paused,
            "uncertain" => run.reconciliation_required = true,
            "completed" => {
                run.session_ledger
                    .sessions
                    .get_mut(run.active_session.as_ref().unwrap())
                    .unwrap()
                    .state = TurnState::Completed
            }
            "executor" => {
                run.session_ledger
                    .sessions
                    .get_mut(run.active_session.as_ref().unwrap())
                    .unwrap()
                    .scope
                    .role = RoleKind::Executor
            }
            _ => unreachable!(),
        }
        save(&runtime, &run);
        assert!(
            runtime.dispatch(&id(), retry(&run)).is_err(),
            "accepted {change}"
        );
        assert_eq!(
            serde_json::to_value(read(&runtime, &run)).unwrap(),
            serde_json::to_value(&run).unwrap()
        );
    }
}
#[test]
fn changed_inputs_and_uncommitted_executor_output_cannot_retry() {
    let (root, runtime, before) = fixture(false);
    std::fs::write(root.path().join("pinned.txt"), "external edit").unwrap();
    assert!(runtime.dispatch(&id(), retry(&before)).is_err());
    assert_eq!(
        read(&runtime, &before).approvals.len(),
        before.approvals.len()
    );
    std::fs::write(root.path().join("pinned.txt"), "original").unwrap();
    let mut run = before.clone();
    let artifact = run.artifacts.last().unwrap();
    let author = &run.session_ledger.sessions[&artifact.session_id];
    run.session_ledger
        .turns
        .get_mut(author.current_turn_id.as_ref().unwrap())
        .unwrap()
        .output_ref = None;
    save(&runtime, &run);
    assert!(runtime.dispatch(&id(), retry(&run)).is_err());
    assert_eq!(read(&runtime, &run).session_ledger, run.session_ledger);
}
#[test]
fn cli_requires_exact_scoped_authorization_before_ipc() {
    let root = tempfile::tempdir().unwrap();
    for args in [
        vec!["verify", "missing-state", "run"],
        vec!["verify", "missing-state", "run", "--wrong"],
        vec![
            "verify",
            "missing-state",
            "run",
            "--approve-scoped",
            "extra",
        ],
    ] {
        let output = Process::new(env!("CARGO_BIN_EXE_antislop"))
            .current_dir(root.path())
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("verify requires"));
    }
    assert!(!root.path().join("missing-state").exists());
}

fn completed_invalid_fixture() -> (tempfile::TempDir, Arc<Runtime>, Run) {
    let (root, runtime, mut run) = fixture(false);
    let artifact = run.artifacts.last().unwrap();
    let evidence = run
        .evidence
        .iter()
        .find(|e| e.source == "runtime:artifact-read" && e.artifact_hash == artifact.hash)
        .unwrap();
    let mut review = FakeProvider
        .verify(
            &run,
            artifact,
            evidence,
            Arc::new(AtomicBool::new(false)),
            &mut |_| Ok(()),
        )
        .unwrap();
    let quality = review.quality.as_mut().unwrap();
    quality.context = None;
    quality.planning = None;
    quality.software = None;
    quality.research = None;
    quality.analysis = None;
    quality.editorial = None;
    quality.delivery = None;
    review.verifier_session = run.active_session.clone().unwrap();
    run.reviews.push(review);
    run.status = Status::ChangesRequested;
    run.diagnostic =
        Some("Completed provider response omitted mandatory typed quality report".into());
    bind_completed_review(&mut run);
    assert!(
        architect::verification::validate_review(
            &run,
            run.artifacts.last().unwrap(),
            run.reviews.last().unwrap()
        )
        .is_err()
    );
    save(&runtime, &run);
    (root, runtime, run)
}
fn bind_completed_review(run: &mut Run) {
    let session = run
        .session_ledger
        .sessions
        .get_mut(run.active_session.as_ref().unwrap())
        .unwrap();
    session.state = TurnState::Completed;
    let mut raw = run.reviews.last().unwrap().clone();
    raw.verifier_session = session.native_session_id.clone().unwrap();
    let output = serde_json::to_string(&raw).unwrap();
    let turn = run
        .session_ledger
        .turns
        .get_mut(session.current_turn_id.as_ref().unwrap())
        .unwrap();
    turn.state = TurnState::Completed;
    turn.provider_completed = true;
    turn.failure = None;
    turn.output_ref = Some(OutputRef {
        document: format!("turn:{}", turn.id),
        hash: hash(&output),
    });
    turn.output = Some(output);
}
#[test]
fn completed_quality_rejection_retries_only_verifier_without_reset_or_history_rewrite() {
    let (root, runtime, before) = completed_invalid_fixture();
    let invalid_review = serde_json::to_value(before.reviews.last().unwrap()).unwrap();
    let key = id();
    let command = retry(&before);
    runtime.dispatch(&key, command.clone()).unwrap();
    let after = wait(&runtime, &before);
    runtime.dispatch(&key, command).unwrap();
    assert_eq!(
        after.status,
        Status::AwaitingResult,
        "{:?}",
        after.diagnostic
    );
    assert_eq!(
        (after.stage, after.spec_revision, after.attempt),
        (before.stage, before.spec_revision, before.attempt)
    );
    assert_eq!(
        serde_json::to_value(&after.artifacts).unwrap(),
        serde_json::to_value(&before.artifacts).unwrap()
    );
    assert_eq!(after.reviews.len(), before.reviews.len() + 1);
    assert_eq!(
        serde_json::to_value(&after.reviews[before.reviews.len() - 1]).unwrap(),
        invalid_review
    );
    assert_eq!(
        after.session_ledger.sessions.len(),
        before.session_ledger.sessions.len() + 1
    );
    assert_eq!(
        after.session_ledger.turns.len(),
        before.session_ledger.turns.len() + 1
    );
    for (id, session) in &before.session_ledger.sessions {
        assert_eq!(&after.session_ledger.sessions[id], session);
    }
    let fresh = after
        .session_ledger
        .sessions
        .values()
        .find(|s| !before.session_ledger.sessions.contains_key(&s.id))
        .unwrap();
    assert_eq!(fresh.scope.role, RoleKind::Verifier);
    assert_eq!(fresh.scope.stage, before.stage);
    assert_eq!(fresh.state, TurnState::Completed);
    assert!(!after.reconciliation_required);
    assert_eq!(
        after
            .approvals
            .iter()
            .filter(|a| a.action == "repair")
            .count(),
        before
            .approvals
            .iter()
            .filter(|a| a.action == "repair")
            .count()
    );
    assert_eq!(
        after
            .evidence
            .iter()
            .filter(|e| e.source.starts_with("runner:"))
            .count(),
        before
            .evidence
            .iter()
            .filter(|e| e.source.starts_with("runner:"))
            .count()
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("pinned.txt")).unwrap(),
        "original"
    );
    architect::verification::validate_review(
        &after,
        after.artifacts.last().unwrap(),
        after.reviews.last().unwrap(),
    )
    .unwrap();
}
#[test]
fn completed_retry_rejects_substantive_defects_and_unauthenticated_reports_without_effects() {
    let (_root, runtime, before) = completed_invalid_fixture();
    for change in [
        "finding",
        "failed-check",
        "blocked-check",
        "mandatory-na",
        "raw-mismatch",
        "raw-hash",
        "not-completed",
        "foreign-scope",
        "foreign-provider",
        "wrong-artifact",
        "uncertain",
        "valid-report",
    ] {
        let mut run = before.clone();
        match change {
            "finding" => run
                .reviews
                .last_mut()
                .unwrap()
                .findings
                .push("Actual unresolved behavior defect".into()),
            "failed-check" => run.reviews.last_mut().unwrap().checks[0].status = CheckStatus::Fail,
            "blocked-check" => {
                run.reviews.last_mut().unwrap().checks[0].status = CheckStatus::Blocked
            }
            "mandatory-na" => {
                let check = run
                    .reviews
                    .last_mut()
                    .unwrap()
                    .checks
                    .iter_mut()
                    .find(|c| c.check_id.starts_with("REQ:"))
                    .unwrap();
                check.status = CheckStatus::NotApplicable;
            }
            "wrong-artifact" => {
                run.reviews.last_mut().unwrap().artifact_hash = hash("another artifact")
            }
            "valid-report" => {
                run.reviews.last_mut().unwrap().quality =
                    Some(architect::quality::synthetic_artifacts(&before));
            }
            _ => (),
        }
        bind_completed_review(&mut run);
        if change == "valid-report" {
            architect::verification::validate_review(
                &run,
                run.artifacts.last().unwrap(),
                run.reviews.last().unwrap(),
            )
            .unwrap();
        }
        let session = run
            .session_ledger
            .sessions
            .get_mut(run.active_session.as_ref().unwrap())
            .unwrap();
        let turn = run
            .session_ledger
            .turns
            .get_mut(session.current_turn_id.as_ref().unwrap())
            .unwrap();
        match change {
            "raw-mismatch" => {
                let mut raw: serde_json::Value =
                    serde_json::from_str(turn.output.as_ref().unwrap()).unwrap();
                raw["findings"] = serde_json::json!(["Different durable response"]);
                let output = raw.to_string();
                turn.output_ref.as_mut().unwrap().hash = hash(&output);
                turn.output = Some(output);
            }
            "raw-hash" => turn.output_ref.as_mut().unwrap().hash = hash("not committed output"),
            "not-completed" => turn.provider_completed = false,
            "foreign-scope" => session.scope.spec_revision += 1,
            "foreign-provider" => session.scope.provider = "codex".into(),
            "uncertain" => run.reconciliation_required = true,
            _ => (),
        }
        save(&runtime, &run);
        assert!(
            runtime.dispatch(&id(), retry(&run)).is_err(),
            "accepted {change}"
        );
        assert_eq!(
            serde_json::to_value(read(&runtime, &run)).unwrap(),
            serde_json::to_value(&run).unwrap(),
            "effects from rejected {change}"
        );
    }
}

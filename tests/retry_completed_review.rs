use architect::{
    domain::*,
    hash, id,
    runtime::{Command, Runtime},
    storage::Store,
    verification,
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

fn wait(runtime: &Arc<Runtime>, id: &str) -> Run {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let run = runtime.store.lock().unwrap().get(id).unwrap();
        if run.status != Status::Running {
            return run;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn fixture() -> (tempfile::TempDir, Arc<Runtime>, Run) {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("pinned.txt"), "original").unwrap();
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().canonicalize().unwrap().to_string_lossy().into();
    spec.input_files = vec!["pinned.txt".into()];
    let runtime =
        Arc::new(Runtime::new(Store::open(&root.path().join("state.db")).unwrap()).unwrap());
    let run = runtime.dispatch(&id(), Command::Create { spec }).unwrap();
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
    let run = wait(&runtime, &run.id);
    assert_eq!(run.status, Status::AwaitingResult, "{:?}", run.diagnostic);
    verification::validate_review(
        &run,
        run.artifacts.last().unwrap(),
        run.reviews.last().unwrap(),
    )
    .unwrap();
    (root, runtime, run)
}

fn bind_output(run: &mut Run) {
    let session = &run.session_ledger.sessions[run.active_session.as_ref().unwrap()];
    let mut raw = run.reviews.last().unwrap().clone();
    raw.verifier_session = session.native_session_id.clone().unwrap();
    let output = serde_json::to_string(&raw).unwrap();
    let turn = run
        .session_ledger
        .turns
        .get_mut(session.current_turn_id.as_ref().unwrap())
        .unwrap();
    turn.output_ref.as_mut().unwrap().hash = hash(&output);
    turn.output = Some(output);
}

fn malformed(mut run: Run) -> Run {
    run.status = Status::ChangesRequested;
    run.reviews
        .last_mut()
        .unwrap()
        .quality
        .as_mut()
        .unwrap()
        .context = None;
    bind_output(&mut run);
    let error = verification::validate_review(
        &run,
        run.artifacts.last().unwrap(),
        run.reviews.last().unwrap(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("ContextAudit"), "{error}");
    run.diagnostic = Some(error.to_string());
    run
}

fn command(run: &Run) -> Command {
    let artifact = run.artifacts.last().unwrap();
    Command::RetryVerification {
        run_id: run.id.clone(),
        artifact_hash: artifact.hash.clone(),
        scope_hash: run.scope_hash(&format!("verify:{}", artifact.hash)),
    }
}

fn save(runtime: &Arc<Runtime>, run: &Run) {
    runtime
        .store
        .lock()
        .unwrap()
        .save(run, "completed_review_fixture", None)
        .unwrap();
}

#[test]
fn missing_context_audit_retries_only_verifier_with_durable_history() {
    let (_root, runtime, run) = fixture();
    let before = malformed(run);
    save(&runtime, &before);
    runtime.dispatch(&id(), command(&before)).unwrap();
    let after = wait(&runtime, &before.id);
    assert_eq!(
        after.status,
        Status::AwaitingResult,
        "{:?}",
        after.diagnostic
    );
    assert_eq!(
        (after.stage, after.attempt, after.spec_revision),
        (before.stage, before.attempt, before.spec_revision)
    );
    assert_eq!(
        serde_json::to_value(&after.artifacts).unwrap(),
        serde_json::to_value(&before.artifacts).unwrap()
    );
    assert_eq!(after.reviews.len(), before.reviews.len() + 1);
    assert_eq!(
        after.session_ledger.sessions.len(),
        before.session_ledger.sessions.len() + 1
    );
    for (id, old) in &before.session_ledger.sessions {
        assert_eq!(&after.session_ledger.sessions[id], old);
    }
    for (id, old) in &before.session_ledger.turns {
        assert_eq!(&after.session_ledger.turns[id], old);
    }
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
    assert!(
        !after
            .evidence
            .iter()
            .any(|e| e.source.starts_with("runner:"))
    );
}

#[test]
fn valid_completed_and_substantive_rejection_cannot_retry() {
    let (_root, runtime, original) = fixture();
    for change in ["valid", "findings", "failed_check", "mandatory_na"] {
        let mut run = if change == "valid" {
            original.clone()
        } else {
            malformed(original.clone())
        };
        run.status = Status::ChangesRequested;
        match change {
            "findings" => run
                .reviews
                .last_mut()
                .unwrap()
                .findings
                .push("actual defect requires repair".into()),
            "failed_check" => run.reviews.last_mut().unwrap().checks[0].status = CheckStatus::Fail,
            "mandatory_na" => {
                run.reviews
                    .last_mut()
                    .unwrap()
                    .checks
                    .iter_mut()
                    .find(|c| c.check_id == "V01")
                    .unwrap()
                    .status = CheckStatus::NotApplicable
            }
            _ => (),
        }
        bind_output(&mut run);
        save(&runtime, &run);
        assert!(
            runtime.dispatch(&id(), command(&run)).is_err(),
            "accepted {change}"
        );
        assert_eq!(
            serde_json::to_value(runtime.store.lock().unwrap().get(&run.id).unwrap()).unwrap(),
            serde_json::to_value(&run).unwrap()
        );
    }
}

#[test]
fn malformed_report_requires_exact_durable_output_and_authorship() {
    let (_root, runtime, original) = fixture();
    for change in [
        "hash",
        "document",
        "raw",
        "stored",
        "author",
        "scope",
        "incomplete",
    ] {
        let mut run = malformed(original.clone());
        if change == "author" {
            run.reviews.last_mut().unwrap().executor_session = "other-author".into();
            bind_output(&mut run);
        }
        if change == "stored" {
            run.reviews.last_mut().unwrap().checks[0]
                .rationale
                .push_str("changed");
        }
        let session = run
            .session_ledger
            .sessions
            .get_mut(run.active_session.as_ref().unwrap())
            .unwrap();
        if change == "scope" {
            session.scope.input_hash = hash("other-inputs");
        }
        let turn = run
            .session_ledger
            .turns
            .get_mut(session.current_turn_id.as_ref().unwrap())
            .unwrap();
        match change {
            "hash" => turn.output_ref.as_mut().unwrap().hash = hash("other-output"),
            "document" => turn.output_ref.as_mut().unwrap().document = "turn:other".into(),
            "raw" => {
                turn.output = Some("{}".into());
                turn.output_ref.as_mut().unwrap().hash = hash("{}");
            }
            "incomplete" => turn.provider_completed = false,
            _ => (),
        }
        save(&runtime, &run);
        assert!(
            runtime.dispatch(&id(), command(&run)).is_err(),
            "accepted {change}"
        );
        assert_eq!(
            serde_json::to_value(runtime.store.lock().unwrap().get(&run.id).unwrap()).unwrap(),
            serde_json::to_value(&run).unwrap()
        );
    }
}

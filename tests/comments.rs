use architect::{
    comments::{self, ElementKind},
    domain::*,
    id,
    runtime::{Command, Runtime},
    storage::Store,
    team::TeamPlan,
};
use std::sync::Arc;

#[test]
fn every_inspectable_element_has_a_version_bound_durable_comment_target() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("source.txt"), "source").unwrap();
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().to_string_lossy().into();
    spec.input_files = vec!["source.txt".into()];
    spec.editable_files = spec.input_files.clone();
    spec.check_command = vec!["true".into()];
    let runtime =
        Arc::new(Runtime::new(Store::open(&root.path().join("state.db")).unwrap()).unwrap());
    let mut run = runtime.dispatch(&id(), Command::Create { spec }).unwrap();
    run.team = Some(TeamPlan::bootstrap(run.spec.task_family, &run.spec).unwrap());
    run.artifacts.push(Artifact {
        id: "artifact".into(),
        stage: Stage::Discovery,
        attempt: 1,
        spec_revision: 1,
        session_id: "fixture-session".into(),
        content: "report".into(),
        hash: architect::hash("report"),
    });
    run.evidence.push(Evidence {
        id: "evidence".into(),
        artifact_hash: architect::hash("report"),
        spec_revision: 1,
        source: "fixture".into(),
        output_hash: architect::hash("receipt"),
        output: "receipt".into(),
        success: true,
        input_hash: None,
    });
    run.reviews.push(Review {
        quality: None,
        artifact_hash: architect::hash("report"),
        spec_revision: 1,
        checklist_version: run.checklist_version.clone(),
        executor_session: "fixture-session".into(),
        verifier_session: "fixture-verifier".into(),
        checks: vec![CheckResult {
            check_id: "fixture-check".into(),
            status: CheckStatus::Blocked,
            method: "fixture".into(),
            evidence_ids: vec!["evidence".into()],
            rationale: "fixture".into(),
        }],
        findings: vec!["Fixture finding".into()],
    });
    run.quality_gates.insert(
        "fixture-gate".into(),
        architect::quality::GateAssessment {
            gate_id: "fixture-gate".into(),
            phase: architect::quality::GatePhase::Design,
            status: CheckStatus::Blocked,
            spec_revision: 1,
            source_hashes: run.source_hashes(),
            artifact_hash: architect::hash("report"),
            review_hash: "fixture".into(),
            check_id: "fixture-check".into(),
            evidence_ids: vec!["evidence".into()],
            report_hash: "fixture".into(),
        },
    );
    runtime
        .store
        .lock()
        .unwrap()
        .save(&run, "synthetic_annotation_fixture", None)
        .unwrap();
    let recipient = run.team.as_ref().unwrap().agents[0].id.clone();
    run = runtime
        .dispatch(
            &id(),
            Command::SendMessage {
                run_id: run.id.clone(),
                target_agent_id: recipient,
                text: "Fixture message".into(),
            },
        )
        .unwrap();
    let targets = comments::elements(&run);
    let kinds: std::collections::BTreeSet<_> = targets.iter().map(|e| e.reference.kind).collect();
    assert_eq!(
        kinds,
        [
            ElementKind::Agent,
            ElementKind::Task,
            ElementKind::Relation,
            ElementKind::Artifact,
            ElementKind::Finding,
            ElementKind::Message,
            ElementKind::Evidence,
            ElementKind::Check,
            ElementKind::Gate
        ]
        .into()
    );
    for kind in kinds {
        let target = targets
            .iter()
            .find(|e| e.reference.kind == kind)
            .unwrap()
            .reference
            .clone();
        let key = id();
        let command = Command::AddElementComment {
            run_id: run.id.clone(),
            target: target.clone(),
            text: format!("Comment on {kind:?}"),
        };
        run = runtime.dispatch(&key, command.clone()).unwrap();
        let count = run.element_comments.len();
        let repeated = runtime.dispatch(&key, command).unwrap();
        assert_eq!(
            repeated.element_comments.len(),
            count,
            "idempotent retry must not duplicate comments"
        );
        assert_eq!(run.element_comments.last().unwrap().target, target);
        assert_eq!(run.element_comments.last().unwrap().author, "local_user");
    }
    let snapshot = architect::tui::model::Snapshot::from_document(
        &serde_json::to_value(&run).unwrap(),
        &serde_json::json!({}),
    );
    assert_eq!(snapshot.element_comments.len(), 9);
    assert_eq!(
        snapshot.comment_targets.len(),
        comments::elements(&run).len()
    );
    let reopened = Runtime::new(Store::open(&root.path().join("state.db")).unwrap()).unwrap();
    let restored = reopened.store.lock().unwrap().get(&run.id).unwrap();
    assert_eq!(restored.element_comments.len(), 9);
    let old = comments::elements(&restored)
        .into_iter()
        .find(|e| e.reference.kind == ElementKind::Task)
        .unwrap()
        .reference;
    let mut changed = restored.clone();
    changed
        .team
        .as_mut()
        .unwrap()
        .tasks
        .iter_mut()
        .find(|t| t.id == old.id)
        .unwrap()
        .description
        .push_str(" updated");
    runtime
        .store
        .lock()
        .unwrap()
        .save(&changed, "synthetic_task_change", None)
        .unwrap();
    assert!(
        runtime
            .dispatch(
                &id(),
                Command::AddElementComment {
                    run_id: run.id.clone(),
                    target: old,
                    text: "Stale target".into()
                }
            )
            .is_err()
    );
    assert_eq!(
        runtime
            .store
            .lock()
            .unwrap()
            .get(&run.id)
            .unwrap()
            .element_comments
            .len(),
        9
    );
    // Exercise the actual TUI backend and private Unix socket, including reattachment.
    use architect::tui::backend::{Action, Backend, IpcBackend};
    struct Daemon(std::process::Child);
    impl Drop for Daemon {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut daemon = Daemon(
        std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
            .arg("daemon")
            .arg(root.path())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if let Some(status) = daemon.0.try_wait().unwrap() {
            use std::io::Read;
            let mut error = String::new();
            daemon
                .0
                .stderr
                .take()
                .unwrap()
                .read_to_string(&mut error)
                .unwrap();
            panic!("annotation fixture daemon exited {status}: {error}");
        }
        if architect::ipc::call(
            root.path(),
            &architect::ipc::Request {
                version: architect::ipc::PROTOCOL_VERSION,
                request_id: id(),
                idempotency_key: None,
                operation: architect::ipc::Operation::List,
            },
        )
        .is_ok_and(|reply| reply.error.is_none())
        {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let mut backend = IpcBackend::new(root.path());
    let snapshot = backend.snapshot(&run.id).unwrap();
    let target = snapshot
        .comment_targets
        .iter()
        .find(|e| e.reference.kind == ElementKind::Check)
        .unwrap()
        .reference
        .clone();
    backend
        .act(
            &run.id,
            Action::AddElementComment {
                target,
                text: "Comment through real TUI IPC".into(),
            },
        )
        .unwrap();
    let mut reattached = IpcBackend::new(root.path());
    let snapshot = reattached.snapshot(&run.id).unwrap();
    let comment = snapshot.element_comments.last().unwrap();
    assert_eq!(comment.text, "Comment through real TUI IPC");
    let comment_id = comment.id.clone();
    reattached
        .act(
            &run.id,
            Action::ResolveElementComment {
                comment_id: comment_id.clone(),
                resolved: true,
            },
        )
        .unwrap();
    assert!(
        reattached
            .snapshot(&run.id)
            .unwrap()
            .element_comments
            .last()
            .unwrap()
            .resolved
    );
    reattached
        .act(&run.id, Action::DeleteElementComment { comment_id })
        .unwrap();
    assert!(
        reattached
            .snapshot(&run.id)
            .unwrap()
            .element_comments
            .last()
            .unwrap()
            .deleted
    );
}

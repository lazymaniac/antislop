//! Independent recovery regressions on disposable local fixtures only.
use architect::{
    domain::*,
    hash, id,
    recovery::{self, EditState},
    runtime::{Command, Runtime},
    storage::Store,
    verification,
};
use std::{collections::BTreeMap, os::unix::fs::MetadataExt, path::Path, sync::Arc};
fn fixture(root: &Path, applied: usize) -> Run {
    std::fs::write(root.join("a"), "old a").unwrap();
    std::fs::write(root.join("b"), "old b").unwrap();
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.canonicalize().unwrap().to_string_lossy().into();
    spec.input_files = vec!["a".into(), "b".into()];
    spec.editable_files = spec.input_files.clone();
    spec.check_command = vec![
        "python3".into(),
        "-c".into(),
        "from pathlib import Path; Path('UNAUTHORIZED_REPLAY').write_text('ran')".into(),
    ];
    let mut run = Run::new(id(), spec).unwrap();
    run.stage = Stage::Execution;
    run.status = Status::Blocked;
    run.attempt = 3;
    run.reconciliation_required = true;
    run.diagnostic = Some("Independent verifier CLI exited before completed event".into());
    run.pending_edits = recovery::prepare(
        &run,
        &[
            FileEdit {
                operation: architect::domain::FileOperation::Write,
                path: "a".into(),
                content: "new a".into(),
            },
            FileEdit {
                operation: architect::domain::FileOperation::Write,
                path: "b".into(),
                content: "new b".into(),
            },
        ],
    )
    .unwrap();
    for (index, path) in ["a", "b"].iter().enumerate() {
        if index < applied {
            let text = format!("new {path}");
            std::fs::write(root.join(path), &text).unwrap();
            run.input_snapshot.insert(path.to_string(), text);
            run.pending_edits[index].state = EditState::Applied;
        } else {
            run.pending_edits[index].state = EditState::Unapplied;
        }
    }
    for (index, stage) in [Stage::Discovery, Stage::Planning, Stage::Execution]
        .into_iter()
        .enumerate()
    {
        let content = format!("Persisted historical artifact {index}");
        let artifact = Artifact {
            id: format!("artifact-{index}"),
            stage,
            attempt: index as u64 + 1,
            spec_revision: 1,
            session_id: format!("executor-{index}"),
            hash: hash(&content),
            content,
        };
        if stage != Stage::Execution {
            run.reviews.push(Review {
                quality: None,
                artifact_hash: artifact.hash.clone(),
                spec_revision: 1,
                checklist_version: run.checklist_version.clone(),
                executor_session: artifact.session_id.clone(),
                verifier_session: format!("verifier-{index}"),
                checks: vec![],
                findings: vec![],
            });
        }
        run.artifacts.push(artifact);
    }
    // Current source-bound context/model nodes represent checkpointed progress;
    // their preservation is verified independently from cached `valid` flags.
    for (name, kind) in [
        ("context", ProvenanceKind::ContextModel),
        ("solution", ProvenanceKind::SolutionModel),
    ] {
        let content = format!("Current fixture {name}");
        run.provenance.insert(
            name.into(),
            ProvenanceNode {
                kind,
                spec_revision: 1,
                content_hash: hash(&content),
                content,
                source_hashes: run.source_hashes(),
                dependencies: BTreeMap::new(),
                valid: true,
                revalidation: None,
                superseded_by: None,
                refresh: false,
            },
        );
    }
    let snapshot_output = serde_json::to_string(&run.input_snapshot).unwrap();
    let snapshot_hash = hash(&snapshot_output);
    let evidence_id = "current-input-evidence".to_string();
    run.evidence.push(Evidence {
        id: evidence_id.clone(),
        artifact_hash: run.artifacts.last().unwrap().hash.clone(),
        spec_revision: 1,
        source: "runtime:input-snapshot".into(),
        output_hash: snapshot_hash.clone(),
        output: snapshot_output,
        success: true,
        input_hash: Some(snapshot_hash),
    });
    let refresh_review = Review {
        quality: None,
        artifact_hash: run.artifacts.last().unwrap().hash.clone(),
        spec_revision: 1,
        checklist_version: run.checklist_version.clone(),
        executor_session: "refresh-executor".into(),
        verifier_session: "independent-refresh-verifier".into(),
        checks: ["context", "solution"]
            .into_iter()
            .map(|node| CheckResult {
                check_id: format!("REVALIDATE:{node}"),
                status: CheckStatus::Pass,
                method: "Fixture current input binding".into(),
                evidence_ids: vec![evidence_id.clone()],
                rationale: "Current source receipt backs this recovery fixture binding".into(),
            })
            .collect(),
        findings: vec![],
    };
    let review_hash = hash(serde_json::to_vec(&refresh_review).unwrap());
    for node in ["context", "solution"] {
        let binding = ProvenanceValidation {
            review_hash: review_hash.clone(),
            verifier_session: refresh_review.verifier_session.clone(),
            artifact_hash: refresh_review.artifact_hash.clone(),
            evidence_ids: vec![evidence_id.clone()],
            source_hashes: run.source_hashes(),
            dependencies: BTreeMap::new(),
        };
        let node = run.provenance.get_mut(node).unwrap();
        node.refresh = true;
        node.revalidation = Some(binding);
    }
    run.provenance_reviews.insert(review_hash, refresh_review);
    verification::refresh_provenance(&mut run);
    run
}
fn runtime(root: &Path, run: &Run) -> Arc<Runtime> {
    let mut store = Store::open(&root.join("state.db")).unwrap();
    store
        .save(run, "fixture_completed_writes_verifier_failed", None)
        .unwrap();
    Arc::new(Runtime::new(store).unwrap())
}
fn reconcile(runtime: &Arc<Runtime>, run: &Run) -> Run {
    runtime
        .dispatch(
            &id(),
            Command::Reconcile {
                run_id: run.id.clone(),
                scope_hash: run.scope_hash("reconcile"),
                input_hash: recovery::observe(run).unwrap().hash,
            },
        )
        .unwrap()
}
fn file_identity(root: &Path) -> Vec<(String, Vec<u8>, u64, std::time::SystemTime)> {
    ["a", "b"]
        .into_iter()
        .map(|name| {
            let p = root.join(name);
            let metadata = std::fs::metadata(&p).unwrap();
            (
                name.into(),
                std::fs::read(&p).unwrap(),
                metadata.ino(),
                metadata.modified().unwrap(),
            )
        })
        .collect()
}
fn assert_no_execution(runtime: &Arc<Runtime>, run: &Run, root: &Path) {
    assert!(!root.join("UNAUTHORIZED_REPLAY").exists());
    assert_eq!(run.attempt, 3);
    let events = runtime.store.lock().unwrap().events(0).unwrap();
    assert!(
        !events
            .iter()
            .any(|e| e.kind == "attempt_started" || e.kind == "turn_reserved")
    );
}
#[test]
fn fully_checkpointed_applied_writes_preserve_revision_stage_and_verified_progress() {
    let temp = tempfile::tempdir().unwrap();
    let before = fixture(temp.path(), 2);
    let files = file_identity(temp.path());
    let runtime = runtime(temp.path(), &before);
    let recovered = reconcile(&runtime, &before);
    assert_eq!(
        recovered.spec_revision, 1,
        "fully checkpointed writes must not invalidate original scoped delegation"
    );
    assert_eq!(recovered.stage, Stage::Execution);
    assert_eq!(recovered.status, Status::Blocked);
    assert_eq!(recovered.input_snapshot, before.input_snapshot);
    assert_eq!(
        serde_json::to_value(&recovered.artifacts).unwrap(),
        serde_json::to_value(&before.artifacts).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&recovered.reviews).unwrap(),
        serde_json::to_value(&before.reviews).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&recovered.provenance_reviews).unwrap(),
        serde_json::to_value(&before.provenance_reviews).unwrap()
    );
    for node in ["context", "solution"] {
        assert!(verification::provenance_node_current(&before, node));
        assert!(verification::provenance_node_current(&recovered, node));
        assert_eq!(
            recovered.provenance[node].binding_hash(),
            before.provenance[node].binding_hash()
        );
    }
    assert!(recovered.pending_edits.is_empty());
    assert!(!recovered.reconciliation_required);
    assert_eq!(file_identity(temp.path()), files);
    assert_no_execution(&runtime, &recovered, temp.path());
    assert!(
        runtime
            .dispatch(
                &id(),
                Command::Advance {
                    run_id: before.id.clone()
                }
            )
            .is_err(),
        "reconcile must not grant another attempt"
    );
    assert_eq!(recovered.approvals.len(), 1);
    assert_eq!(recovered.approvals[0].action, "reconcile");
    let receipt = recovered
        .evidence
        .iter()
        .find(|e| e.source == "runtime:edit-reconciliation")
        .unwrap();
    assert_eq!(receipt.output_hash, hash(&receipt.output));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&receipt.output).unwrap()["edits"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["state"] == "applied")
            .count(),
        2
    );
    let persisted = runtime.store.lock().unwrap().get(&before.id).unwrap();
    assert_eq!(persisted.spec_revision, 1);
    assert_eq!(persisted.status, Status::Blocked);
}
#[test]
fn unapplied_or_partial_receipts_reset_discovery_even_when_inputs_equal_checkpoint() {
    for applied in [0, 1] {
        let temp = tempfile::tempdir().unwrap();
        let before = fixture(temp.path(), applied);
        let files = file_identity(temp.path());
        let runtime = runtime(temp.path(), &before);
        let recovered = reconcile(&runtime, &before);
        assert_eq!(
            before.input_snapshot,
            architect::workspace::read_inputs(temp.path(), &before.spec.input_files).unwrap()
        );
        assert_eq!(
            recovered.spec_revision, 2,
            "unapplied receipt must invalidate progress even when snapshot matches actual inputs"
        );
        assert_eq!(recovered.stage, Stage::Discovery);
        assert_eq!(recovered.status, Status::AwaitingStart);
        assert!(recovered.pending_edits.is_empty());
        assert!(!recovered.reconciliation_required);
        assert_eq!(
            serde_json::to_value(&recovered.artifacts).unwrap(),
            serde_json::to_value(&before.artifacts).unwrap()
        );
        assert!(!verification::provenance_node_current(
            &recovered, "context"
        ));
        assert_eq!(file_identity(temp.path()), files);
        assert_no_execution(&runtime, &recovered, temp.path());
        assert!(
            runtime
                .dispatch(
                    &id(),
                    Command::Advance {
                        run_id: before.id.clone()
                    }
                )
                .is_err()
        );
    }
}
#[test]
fn external_conflict_still_resets_without_overwriting_user_changes() {
    let temp = tempfile::tempdir().unwrap();
    let before = fixture(temp.path(), 2);
    std::fs::write(temp.path().join("b"), "external user change").unwrap();
    let files = file_identity(temp.path());
    let runtime = runtime(temp.path(), &before);
    let recovered = reconcile(&runtime, &before);
    assert_eq!(recovered.spec_revision, 2);
    assert_eq!(recovered.stage, Stage::Discovery);
    assert_eq!(recovered.status, Status::AwaitingStart);
    assert_eq!(recovered.input_snapshot["b"], "external user change");
    assert_eq!(file_identity(temp.path()), files);
    assert_no_execution(&runtime, &recovered, temp.path());
    let receipt = recovered
        .evidence
        .iter()
        .find(|e| e.source == "runtime:edit-reconciliation")
        .unwrap();
    assert!(receipt.output.contains("conflict"));
}

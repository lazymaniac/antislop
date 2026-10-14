use architect::{
    domain::*,
    hash, id,
    providers::{Provider, fake::FakeProvider},
    runtime::{Command, Runtime},
    storage::Store,
    verification,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};

#[test]
fn runtime_refreshes_changed_claims_in_separate_sessions_and_persists_their_reviews() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("source"), "before").unwrap();
    let db = root.path().join("state.db");
    let mut run = Run::new(
        id(),
        RunSpec {
            task_family: architect::quality::TaskFamily::Software,
            config: None,
            objective: "Preserve the source contract".into(),
            workspace: root.path().to_string_lossy().into_owned(),
            provider: "fake".into(),
            model: None,
            requirements: vec![Requirement {
                id: "R1".into(),
                original_text: "Preserve the contract".into(),
                source_ref: "user".into(),
                criteria: BTreeMap::from([("contract".into(), "Contract remains valid".into())]),
            }],
            plan_items: vec![PlanItem {
                id: "P1".into(),
                dependencies: BTreeSet::new(),
                requirement_ids: BTreeSet::from(["R1".into()]),
                description: "Verify current source contract".into(),
            }],
            input_files: vec!["source".into()],
            editable_files: vec![],
            check_command: vec![],
        },
    )
    .unwrap();
    for stage in [Stage::Discovery, Stage::Planning] {
        run.stage = stage;
        run.attempt += 1;
        let (session_id, content) = FakeProvider
            .execute(&run, Arc::new(AtomicBool::new(false)), &mut |_| Ok(()))
            .unwrap();
        let output: ExecutionOutput = serde_json::from_str(&content).unwrap();
        let artifact = Artifact {
            id: id(),
            stage,
            attempt: run.attempt,
            spec_revision: run.spec_revision,
            session_id,
            hash: hash(&content),
            content,
        };
        run.artifacts.push(artifact.clone());
        verification::validate_output(&run, &output).unwrap();
        verification::register_artifact(&mut run, &artifact, &output).unwrap();
        let evidence = verification::observe(&artifact);
        run.evidence.push(evidence.clone());
        let output = serde_json::to_string(&run.input_snapshot).unwrap();
        run.evidence.push(Evidence {
            id: id(),
            artifact_hash: artifact.hash.clone(),
            spec_revision: run.spec_revision,
            input_hash: Some(hash(&output)),
            source: "runtime:input-snapshot".into(),
            output_hash: hash(&output),
            output,
            success: true,
        });
        let review = FakeProvider
            .verify(
                &run,
                &artifact,
                &evidence,
                Arc::new(AtomicBool::new(false)),
                &mut |_| Ok(()),
            )
            .unwrap();
        verification::validate_review(&run, &artifact, &review).unwrap();
        verification::record_review(&mut run, &artifact, &review).unwrap();
        run.reviews.push(review);
    }
    let old_facts: Vec<_> = run
        .provenance
        .iter()
        .filter(|(_, node)| node.kind == ProvenanceKind::ContextFact)
        .map(|(id, node)| (id.clone(), node.content.clone()))
        .collect();
    // Seed the persisted state after an authorized write, before its final review.
    std::fs::write(root.path().join("source"), "after").unwrap();
    run.input_snapshot.insert("source".into(), "after".into());
    verification::refresh_provenance(&mut run);
    run.stage = Stage::Execution;
    run.status = Status::AwaitingStart;
    let mut store = Store::open(&db).unwrap();
    store
        .save(&run, "seed_authorized_source_change", None)
        .unwrap();
    let runtime = Arc::new(Runtime::new(store).unwrap());
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
    let deadline = Instant::now() + Duration::from_secs(20);
    let completed = loop {
        let current = runtime.store.lock().unwrap().get(&run.id).unwrap();
        if current.status != Status::Running {
            break current;
        }
        assert!(Instant::now() < deadline, "runtime refresh did not finish");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(
        completed.status,
        Status::AwaitingResult,
        "{:?}",
        completed.diagnostic
    );
    for stage in [Stage::Discovery, Stage::Planning] {
        assert_eq!(
            completed
                .artifacts
                .iter()
                .filter(|artifact| artifact.stage == stage)
                .count(),
            2
        );
        let fresh = completed
            .artifacts
            .iter()
            .rev()
            .find(|artifact| artifact.stage == stage)
            .unwrap();
        let review = completed
            .reviews
            .iter()
            .find(|review| review.artifact_hash == fresh.hash)
            .unwrap();
        assert_ne!(review.verifier_session, fresh.session_id);
        assert_eq!(review.executor_session, fresh.session_id);
    }
    for (id, content) in old_facts {
        assert_eq!(completed.provenance[&id].content, content);
        assert!(!verification::provenance_node_current(&completed, &id));
    }
    assert!(!verification::needs_context_refresh(&completed));
    assert!(!verification::needs_solution_refresh(&completed));
    assert!(!completed.provenance_reviews.is_empty());
    let reopened = Store::open(&db).unwrap().get(&run.id).unwrap();
    assert_eq!(
        reopened.provenance_reviews.len(),
        completed.provenance_reviews.len()
    );
    assert!(!verification::needs_context_refresh(&reopened));
    assert!(!verification::needs_solution_refresh(&reopened));
}

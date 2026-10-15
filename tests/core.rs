use architect::{
    domain::*,
    hash, id,
    providers::{
        Provider,
        codex::{NormalizedEvent, normalize, read_frame},
        fake::FakeProvider,
    },
    runtime::{Command, Runtime},
    storage::Store,
    verification,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::BufReader,
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};

fn spec(root: &std::path::Path) -> RunSpec {
    RunSpec {
        team_template: None,
        task_family: architect::quality::TaskFamily::Software,
        config: None,
        objective: "Fix the documented behavior".into(),
        workspace: root.to_string_lossy().into(),
        provider: "fake".into(),
        model: None,
        requirements: vec![Requirement {
            id: "R1".into(),
            original_text: "Preserve zero; fix negative inputs".into(),
            source_ref: "user:1".into(),
            criteria: BTreeMap::from([
                ("zero".into(), "Zero remains zero".into()),
                ("negative".into(), "Negative inputs are handled".into()),
            ]),
        }],
        plan_items: vec![PlanItem {
            dependencies: BTreeSet::new(),
            id: "P1".into(),
            requirement_ids: BTreeSet::from(["R1".into()]),
            description: "Implement and check boundaries".into(),
        }],
        input_files: vec![],
        editable_files: vec![],
        check_command: vec![],
    }
}
fn runtime() -> (tempfile::TempDir, Arc<Runtime>) {
    let root = tempfile::tempdir().unwrap();
    let runtime =
        Arc::new(Runtime::new(Store::open(&root.path().join("state.db")).unwrap()).unwrap());
    (root, runtime)
}
fn create(runtime: &Arc<Runtime>, root: &std::path::Path) -> Run {
    runtime
        .dispatch(&id(), Command::Create { spec: spec(root) })
        .unwrap()
}
fn start(runtime: &Arc<Runtime>, run: &Run) -> Run {
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
    wait(runtime, &run.id)
}
fn wait(runtime: &Arc<Runtime>, run_id: &str) -> Run {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let run = runtime.store.lock().unwrap().get(run_id).unwrap();
        if run.status != Status::Running {
            return run;
        }
        assert!(
            Instant::now() < deadline,
            "worker did not finish: stage {:?}, attempt {}, sessions {:?}",
            run.stage,
            run.attempt,
            run.session_ledger
                .sessions
                .values()
                .map(|s| (&s.scope.agent_id, &s.state))
                .collect::<Vec<_>>()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn reviewed() -> (Run, Artifact, Review) {
    let root = tempfile::tempdir().unwrap();
    let mut run = Run::new(id(), spec(root.path())).unwrap();
    let artifact = Artifact {
        id: id(),
        stage: Stage::Discovery,
        attempt: 1,
        spec_revision: 1,
        session_id: "executor-session".into(),
        content: "artifact".into(),
        hash: hash("artifact"),
    };
    let evidence = verification::observe(&artifact);
    run.evidence.push(evidence.clone());
    let review = FakeProvider
        .verify(
            &run,
            &artifact,
            &evidence,
            Arc::new(AtomicBool::new(false)),
            &mut |_| Ok(()),
        )
        .unwrap();
    (run, artifact, review)
}
#[test]
fn supervised_full_cycle_requires_each_start_and_result_approval() {
    let (root, runtime) = runtime();
    let mut run = create(&runtime, root.path());
    for stage in [
        Stage::Discovery,
        Stage::Planning,
        Stage::Execution,
        Stage::Validation,
        Stage::Integration,
        Stage::Delivery,
    ] {
        assert_eq!(run.stage, stage);
        assert!(
            runtime
                .dispatch(
                    &id(),
                    Command::Advance {
                        run_id: run.id.clone()
                    }
                )
                .is_err()
        );
        run = start(&runtime, &run);
        assert_eq!(run.status, Status::AwaitingResult);
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
    assert_eq!(run.status, Status::Completed);
    assert_eq!(run.approvals.len(), 12);
    assert_eq!(run.artifacts.len(), 6);
    assert_eq!(run.reviews.len(), 6);
    assert!(
        run.reviews
            .iter()
            .all(|r| r.executor_session != r.verifier_session)
    );
}
#[test]
fn rejected_result_waits_for_explicit_repair() {
    let (root, runtime) = runtime();
    let mut input = spec(root.path());
    input.objective = "[reject] seeded defect".into();
    let run = runtime
        .dispatch(&id(), Command::Create { spec: input })
        .unwrap();
    let run = start(&runtime, &run);
    assert_eq!(run.status, Status::ChangesRequested);
    assert!(
        runtime
            .dispatch(
                &id(),
                Command::Advance {
                    run_id: run.id.clone()
                }
            )
            .is_err()
    );
    let ready = runtime
        .dispatch(
            &id(),
            Command::ApproveRepair {
                run_id: run.id.clone(),
                scope_hash: run.scope_hash("repair"),
            },
        )
        .unwrap();
    assert_eq!(ready.status, Status::Ready);
    assert_eq!(ready.artifacts.len(), 1);
}
#[test]
fn idempotency_survives_restart_and_rejects_different_payload() {
    let (root, runtime) = runtime();
    let command = Command::Create {
        spec: spec(root.path()),
    };
    let run = runtime.dispatch("same", command.clone()).unwrap();
    assert_eq!(
        runtime.dispatch("same", command.clone()).unwrap().id,
        run.id
    );
    assert_eq!(runtime.store.lock().unwrap().sequence().unwrap(), 1);
    drop(runtime);
    let runtime =
        Arc::new(Runtime::new(Store::open(&root.path().join("state.db")).unwrap()).unwrap());
    assert_eq!(runtime.dispatch("same", command).unwrap().id, run.id);
    assert!(
        runtime
            .dispatch("same", Command::Cancel { run_id: run.id })
            .is_err()
    );
}
#[test]
fn restart_blocks_unknown_attempt_and_preserves_approvals() {
    let (root, runtime) = runtime();
    let mut run = create(&runtime, root.path());
    run = runtime
        .dispatch(
            &id(),
            Command::ApproveStart {
                run_id: run.id.clone(),
                scope_hash: run.scope_hash("start"),
            },
        )
        .unwrap();
    run.status = Status::Running;
    run.active_session = Some("exact-provider-id".into());
    runtime
        .store
        .lock()
        .unwrap()
        .save(&run, "simulated_crash", None)
        .unwrap();
    drop(runtime);
    let runtime =
        Arc::new(Runtime::new(Store::open(&root.path().join("state.db")).unwrap()).unwrap());
    let recovered = runtime.store.lock().unwrap().get(&run.id).unwrap();
    assert_eq!(recovered.status, Status::Blocked);
    assert_eq!(recovered.approvals.len(), 1);
    assert_eq!(recovered.active_session, Some("exact-provider-id".into()));
    assert!(
        runtime
            .dispatch(&id(), Command::Advance { run_id: run.id })
            .is_err()
    );
}
#[test]
fn pause_resume_and_cancel_are_durable_and_legal() {
    let (root, runtime) = runtime();
    let run = create(&runtime, root.path());
    let paused = runtime
        .dispatch(
            &id(),
            Command::Pause {
                run_id: run.id.clone(),
            },
        )
        .unwrap();
    assert_eq!(paused.status, Status::Paused);
    let resumed = runtime
        .dispatch(
            &id(),
            Command::Resume {
                run_id: run.id.clone(),
            },
        )
        .unwrap();
    assert_eq!(resumed.status, Status::AwaitingStart);
    let cancelled = runtime
        .dispatch(
            &id(),
            Command::Cancel {
                run_id: run.id.clone(),
            },
        )
        .unwrap();
    assert_eq!(cancelled.status, Status::Cancelled);
    assert!(
        runtime
            .dispatch(&id(), Command::Resume { run_id: run.id })
            .is_err()
    );
}
#[test]
fn scope_revision_invalidates_old_approval_without_deleting_history() {
    let (root, runtime) = runtime();
    let run = create(&runtime, root.path());
    let old = run.scope_hash("start");
    let mut changed = run.spec.clone();
    changed.objective = "New objective".into();
    let revised = runtime
        .dispatch(
            &id(),
            Command::Revise {
                run_id: run.id.clone(),
                expected_revision: 1,
                spec: changed,
            },
        )
        .unwrap();
    assert_eq!(revised.spec_revision, 2);
    assert!(
        runtime
            .dispatch(
                &id(),
                Command::ApproveStart {
                    run_id: run.id,
                    scope_hash: old
                }
            )
            .is_err()
    );
}
#[test]
fn valid_complete_review_passes() {
    let (run, artifact, review) = reviewed();
    verification::validate_review(&run, &artifact, &review).unwrap();
}
#[test]
fn one_of_one_hundred_missing_requirements_blocks() {
    let (mut run, artifact, _) = reviewed();
    let template = run.spec.requirements[0].clone();
    run.spec.requirements = (0..100)
        .map(|i| Requirement {
            id: format!("R{i}"),
            ..template.clone()
        })
        .collect();
    let mut review = FakeProvider
        .verify(
            &run,
            &artifact,
            &run.evidence[0],
            Arc::new(AtomicBool::new(false)),
            &mut |_| Ok(()),
        )
        .unwrap();
    review.checks.retain(|c| c.check_id != "REQ:R50:negative");
    assert!(verification::validate_review(&run, &artifact, &review).is_err());
}
#[test]
fn missing_risk_blocks_even_if_all_other_checks_pass() {
    let (run, artifact, mut review) = reviewed();
    review.checks.retain(|c| c.check_id != "RISK:R191");
    assert!(verification::validate_review(&run, &artifact, &review).is_err());
}
#[test]
fn missing_plan_item_and_duplicate_check_block() {
    let (run, artifact, mut review) = reviewed();
    let original = review.clone();
    review.checks.retain(|c| c.check_id != "PLAN:P1");
    assert!(verification::validate_review(&run, &artifact, &review).is_err());
    review = original;
    review.checks.push(review.checks[0].clone());
    assert!(verification::validate_review(&run, &artifact, &review).is_err());
}
#[test]
fn stale_artifact_spec_checklist_and_same_session_block() {
    let (run, artifact, review) = reviewed();
    for variant in 0..4 {
        let mut review = review.clone();
        match variant {
            0 => review.artifact_hash = "wrong".into(),
            1 => review.spec_revision = 2,
            2 => review.checklist_version = "old".into(),
            _ => review.verifier_session = review.executor_session.clone(),
        }
        assert!(verification::validate_review(&run, &artifact, &review).is_err());
    }
}
#[test]
fn missing_failed_or_corrupted_evidence_and_findings_block() {
    let (run, artifact, review) = reviewed();
    for variant in 0..4 {
        let mut run = run.clone();
        let mut review = review.clone();
        match variant {
            0 => run.evidence.clear(),
            1 => run.evidence[0].success = false,
            2 => run.evidence[0].output = "tampered".into(),
            _ => review.findings.push("Unresolved defect".into()),
        }
        assert!(verification::validate_review(&run, &artifact, &review).is_err());
    }
}
#[test]
fn empty_na_rationale_or_na_requirement_blocks() {
    let (run, artifact, mut review) = reviewed();
    let risk = review
        .checks
        .iter_mut()
        .find(|c| c.check_id.starts_with("RISK:"))
        .unwrap();
    risk.status = CheckStatus::NotApplicable;
    risk.rationale.clear();
    assert!(verification::validate_review(&run, &artifact, &review).is_err());
    let (run, artifact, mut review) = reviewed();
    let criterion = review
        .checks
        .iter_mut()
        .find(|c| c.check_id.starts_with("REQ:"))
        .unwrap();
    criterion.status = CheckStatus::NotApplicable;
    assert!(verification::validate_review(&run, &artifact, &review).is_err());
}
#[test]
fn frames_handle_fragmentation_unicode_eof_unknown_and_bad_payloads() {
    struct Fragmented<'a>(&'a [u8]);
    impl std::io::Read for Fragmented<'_> {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            let n = out.len().min(1).min(self.0.len());
            out[..n].copy_from_slice(&self.0[..n]);
            self.0 = &self.0[n..];
            Ok(n)
        }
    }
    let bytes="{\"type\":\"future.event\",\"text\":\"żółw\"}\n{\"type\":\"thread.started\",\"thread_id\":\"exact\"}".as_bytes();
    let mut reader = BufReader::new(Fragmented(bytes));
    assert_eq!(
        normalize(&read_frame(&mut reader).unwrap().unwrap()).unwrap(),
        NormalizedEvent::Unknown
    );
    assert_eq!(
        normalize(&read_frame(&mut reader).unwrap().unwrap()).unwrap(),
        NormalizedEvent::Session("exact".into())
    );
    assert!(read_frame(&mut reader).unwrap().is_none());
    assert!(read_frame(&mut BufReader::new(&b"{\xff}\n"[..])).is_err());
    assert!(normalize(&serde_json::json!({"type":"thread.started"})).is_err());
}
#[test]
fn oversized_frame_and_event_cursor_fail_explicitly() {
    let oversized = vec![b'a'; architect::providers::codex::MAX_FRAME + 1];
    assert!(read_frame(&mut BufReader::new(&oversized[..])).is_err());
    let (_root, runtime) = runtime();
    assert!(runtime.store.lock().unwrap().events(u64::MAX).is_err());
}
#[test]
fn snapshot_event_cursor_is_consistent_and_unsupported_protocol_is_rejected() {
    use architect::ipc::{Operation, Request, handle};
    let (root, runtime) = runtime();
    let run = create(&runtime, root.path());
    let request = Request {
        version: architect::ipc::PROTOCOL_VERSION,
        request_id: id(),
        idempotency_key: None,
        operation: Operation::Snapshot { run_id: run.id },
    };
    let response = handle(&runtime, request);
    let data = response.data.unwrap();
    assert_eq!(data["sequence"], 1);
    let response = handle(
        &runtime,
        Request {
            version: architect::ipc::PROTOCOL_VERSION + 1,
            request_id: id(),
            idempotency_key: None,
            operation: Operation::List,
        },
    );
    assert!(response.error.is_some());
}
#[test]
fn changed_workspace_after_review_blocks_acceptance() {
    let (root, runtime) = runtime();
    std::fs::write(root.path().join("source"), "old").unwrap();
    let mut input = spec(root.path());
    input.input_files = vec!["source".into()];
    let run = runtime
        .dispatch(&id(), Command::Create { spec: input })
        .unwrap();
    let run = start(&runtime, &run);
    std::fs::write(root.path().join("source"), "user change").unwrap();
    assert!(
        runtime
            .dispatch(
                &id(),
                Command::AcceptResult {
                    run_id: run.id.clone(),
                    scope_hash: run.scope_hash("accept")
                }
            )
            .is_err()
    );
}
#[test]
fn workspace_rejects_traversal_symlinks_and_conflicting_writes() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("file"), "old").unwrap();
    assert!(architect::workspace::read_inputs(root.path(), &["../outside".into()]).is_err());
    std::os::unix::fs::symlink("file", root.path().join("link")).unwrap();
    assert!(architect::workspace::read_inputs(root.path(), &["link".into()]).is_err());
    assert!(architect::workspace::replace(root.path(), "file", &hash("other"), "new").is_err());
    assert_eq!(
        std::fs::read_to_string(root.path().join("file")).unwrap(),
        "old"
    );
}
#[test]
fn failed_command_is_not_evidence_of_success() {
    let root = tempfile::tempdir().unwrap();
    let (success, output) = architect::workspace::check(
        root.path(),
        &["false".into()],
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert!(!success);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&output).unwrap()["exit_code"],
        1
    );
}
#[test]
fn future_database_schema_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("state.db");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute_batch("PRAGMA user_version=999").unwrap();
    drop(connection);
    assert!(Store::open(&path).is_err());
}

#[test]
fn context_and_model_gaps_block_before_execution() {
    let root = tempfile::tempdir().unwrap();
    let mut run = Run::new(id(), spec(root.path())).unwrap();
    let (_, content) = FakeProvider
        .execute(&run, Arc::new(AtomicBool::new(false)), &mut |_| Ok(()))
        .unwrap();
    let mut output: ExecutionOutput = serde_json::from_str(&content).unwrap();
    verification::validate_output(&run, &output).unwrap();
    output.context.as_mut().unwrap().unknowns.push(Unknown {
        question: "What is the input contract?".into(),
        material: true,
        resolution: None,
        source_ref: None,
    });
    assert!(verification::validate_output(&run, &output).is_err());
    output.context.as_mut().unwrap().unknowns.clear();
    output.context.as_mut().unwrap().facts[0].source_hash = "wrong".into();
    assert!(verification::validate_output(&run, &output).is_err());
    run.stage = Stage::Planning;
    verification::validate_output(&run, &output).unwrap();
    output
        .solution
        .as_mut()
        .unwrap()
        .simulations
        .retain(|s| s.kind != "counterexample");
    assert!(verification::validate_output(&run, &output).is_err());
    output.solution.as_mut().unwrap().options[0]
        .requirement_ids
        .clear();
    assert!(verification::validate_output(&run, &output).is_err());
}
#[test]
fn privacy_checks_refuse_shared_state_and_symlink_database() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(architect::ipc::private_directory(root.path()).is_err());
    let target = root.path().join("target.db");
    std::fs::write(&target, "").unwrap();
    let link = root.path().join("linked.db");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(Store::open(&link).is_err());
}
#[test]
fn stale_revision_does_not_overwrite_another_controller() {
    let (root, runtime) = runtime();
    let run = create(&runtime, root.path());
    let mut changed = run.spec.clone();
    changed.objective = "first change".into();
    runtime
        .dispatch(
            &id(),
            Command::Revise {
                run_id: run.id.clone(),
                expected_revision: 1,
                spec: changed,
            },
        )
        .unwrap();
    assert!(
        runtime
            .dispatch(
                &id(),
                Command::Revise {
                    run_id: run.id.clone(),
                    expected_revision: 1,
                    spec: run.spec
                }
            )
            .is_err()
    );
}

#[test]
fn cyclic_or_unknown_plan_dependencies_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let mut input = spec(root.path());
    input.plan_items[0].dependencies.insert("P1".into());
    assert!(input.validate().is_err());
    input.plan_items[0].dependencies = BTreeSet::from(["unknown".into()]);
    assert!(input.validate().is_err());
    input.plan_items[0].dependencies.clear();
    let mut second = input.plan_items[0].clone();
    second.id = "P2".into();
    second.dependencies.insert("P1".into());
    input.plan_items.push(second);
    input.validate().unwrap();
    input.plan_items[0].dependencies.insert("P2".into());
    assert!(input.validate().is_err());
}

#[test]
fn discovery_can_reference_a_failed_baseline_but_current_failed_checks_block() {
    let (mut run, artifact, mut review) = reviewed();
    let mut baseline = run.evidence[0].clone();
    baseline.id = id();
    baseline.source = "runner:baseline:test".into();
    baseline.artifact_hash = "source-snapshot".into();
    baseline.success = false;
    for check in &mut review.checks {
        check.evidence_ids = vec![baseline.id.clone()];
    }
    run.evidence.push(baseline);
    verification::validate_review(&run, &artifact, &review).unwrap();
    let mut failed = run.evidence[0].clone();
    failed.id = id();
    failed.source = "runner:test".into();
    failed.success = false;
    run.evidence.push(failed);
    assert!(verification::validate_review(&run, &artifact, &review).is_err());
}
#[test]
fn old_input_snapshot_cannot_justify_current_review() {
    let (mut run, artifact, mut review) = reviewed();
    let mut input = run.evidence[0].clone();
    input.id = id();
    input.source = "runtime:input-snapshot".into();
    input.artifact_hash = "source-hash".into();
    input.output = "old source".into();
    input.output_hash = hash(&input.output);
    for check in &mut review.checks {
        check.evidence_ids = vec![input.id.clone()];
    }
    run.evidence.push(input);
    assert!(verification::validate_review(&run, &artifact, &review).is_err());
}

#[test]
fn ambiguous_rendered_criterion_ids_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let mut input = spec(root.path());
    input.requirements[0].id = "a:b".into();
    input.requirements[0].criteria = BTreeMap::from([("c".into(), "first".into())]);
    let mut other = input.requirements[0].clone();
    other.id = "a".into();
    other.criteria = BTreeMap::from([("b:c".into(), "second".into())]);
    input.requirements.push(other);
    assert!(input.validate().is_err());
}
#[test]
fn edits_replace_workspace_entry_without_mutating_external_hardlink() {
    let root = tempfile::tempdir().unwrap();
    let original = root.path().join("external");
    let workspace = root.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::write(&original, "original").unwrap();
    std::fs::hard_link(&original, workspace.join("source")).unwrap();
    architect::workspace::replace(&workspace, "source", &hash("original"), "fixed").unwrap();
    assert_eq!(std::fs::read_to_string(&original).unwrap(), "original");
    assert_eq!(
        std::fs::read_to_string(workspace.join("source")).unwrap(),
        "fixed"
    );
}
#[test]
fn checklist_change_requires_explicit_spec_revision_and_retains_history() {
    let (root, runtime) = runtime();
    let mut run = create(&runtime, root.path());
    run.checklist_version = "old".into();
    runtime
        .store
        .lock()
        .unwrap()
        .save(&run, "old_checklist", None)
        .unwrap();
    drop(runtime);
    let runtime =
        Arc::new(Runtime::new(Store::open(&root.path().join("state.db")).unwrap()).unwrap());
    let current = runtime.store.lock().unwrap().get(&run.id).unwrap();
    assert_eq!(current.status, Status::Blocked);
    assert!(
        runtime
            .dispatch(
                &id(),
                Command::ApproveRepair {
                    run_id: run.id.clone(),
                    scope_hash: current.scope_hash("repair")
                }
            )
            .is_err()
    );
    let revised = runtime
        .dispatch(
            &id(),
            Command::Revise {
                run_id: run.id,
                expected_revision: 1,
                spec: run.spec,
            },
        )
        .unwrap();
    assert_eq!(revised.spec_revision, 2);
    assert_eq!(revised.checklist_version, checklist_version());
    assert_eq!(revised.risk_coverage.len(), 240);
}

#[test]
fn every_risk_has_one_owner_and_delivery_rejects_missing_earlier_assessment() {
    let root = tempfile::tempdir().unwrap();
    let mut run = Run::new(id(), spec(root.path())).unwrap();
    let total: usize = [
        Stage::Discovery,
        Stage::Planning,
        Stage::Execution,
        Stage::Validation,
        Stage::Integration,
        Stage::Delivery,
    ]
    .iter()
    .map(|stage| {
        run.stage = *stage;
        run.expected_checks()
            .iter()
            .filter(|id| id.starts_with("RISK:"))
            .count()
    })
    .sum();
    assert_eq!(total, 240);
    assert_eq!(run.risk_coverage.len(), 240);
    let artifact = Artifact {
        id: id(),
        stage: Stage::Delivery,
        attempt: 1,
        spec_revision: 1,
        session_id: "executor".into(),
        content: "delivery".into(),
        hash: hash("delivery"),
    };
    let evidence = verification::observe(&artifact);
    run.evidence.push(evidence.clone());
    let review = FakeProvider
        .verify(
            &run,
            &artifact,
            &evidence,
            Arc::new(AtomicBool::new(false)),
            &mut |_| Ok(()),
        )
        .unwrap();
    assert!(verification::validate_review(&run, &artifact, &review).is_err());
    for entry in run.risk_coverage.values_mut() {
        entry.status = CheckStatus::Pass;
        entry.rationale = "Fixture assessment".into();
    }
    // Flat status fields cannot replace the missing independent evidence binding.
    assert!(verification::validate_review(&run, &artifact, &review).is_err());
    run.risk_coverage.remove("R001");
    assert!(verification::validate_review(&run, &artifact, &review).is_err());
}

#[test]
fn codex_regression_transcript_preserves_session_message_and_completion() {
    let mut source =
        BufReader::new(include_bytes!("../fixtures/protocol/codex-exec.jsonl").as_slice());
    let mut events = Vec::new();
    while let Some(value) = read_frame(&mut source).unwrap() {
        events.push(normalize(&value).unwrap());
    }
    assert_eq!(
        events,
        vec![
            NormalizedEvent::Session("fixture-session-id".into()),
            NormalizedEvent::Unknown,
            NormalizedEvent::Unknown,
            NormalizedEvent::Message("{\"answer\":42}".into()),
            NormalizedEvent::Completed
        ]
    );
}

#[test]
fn runtime_diff_preserves_original_sources_and_identifies_changed_files() {
    let (mut run, artifact, _) = reviewed();
    let before = BTreeMap::from([
        ("lib.rs".to_string(), "old".to_string()),
        ("tests.rs".to_string(), "unchanged".to_string()),
    ]);
    let output = serde_json::to_string(&before).unwrap();
    run.evidence.push(Evidence {
        input_hash: Some(hash(serde_json::to_vec(&run.input_snapshot).unwrap())),
        id: id(),
        artifact_hash: hash(&output),
        spec_revision: 1,
        source: "runtime:input-snapshot".into(),
        output_hash: hash(&output),
        output,
        success: true,
    });
    run.input_snapshot = before;
    run.input_snapshot.insert("lib.rs".into(), "fixed".into());
    let diff = verification::workspace_diff(&run, &artifact).unwrap();
    let value: serde_json::Value = serde_json::from_str(&diff.output).unwrap();
    assert_eq!(value["before"]["lib.rs"], "old");
    assert_eq!(value["after"]["lib.rs"], "fixed");
    assert_eq!(value["changed_files"], serde_json::json!(["lib.rs"]));
    assert_eq!(diff.artifact_hash, artifact.hash);
}
#[test]
fn earlier_runner_receipt_requires_a_matching_current_input_snapshot() {
    let (mut run, artifact, _) = reviewed();
    run.input_snapshot = BTreeMap::from([("source".into(), "fixed".into())]);
    let output = serde_json::to_string(&run.input_snapshot).unwrap();
    let runner = Evidence {
        input_hash: Some(hash(serde_json::to_vec(&run.input_snapshot).unwrap())),
        id: id(),
        artifact_hash: "earlier-artifact".into(),
        spec_revision: 1,
        source: "runner:test".into(),
        output_hash: hash("pass"),
        output: "pass".into(),
        success: true,
    };
    assert!(!verification::evidence_is_current(&run, &artifact, &runner));
    run.evidence.push(Evidence {
        input_hash: Some(hash(serde_json::to_vec(&run.input_snapshot).unwrap())),
        id: id(),
        artifact_hash: "earlier-artifact".into(),
        spec_revision: 1,
        source: "runtime:input-snapshot".into(),
        output_hash: hash(&output),
        output,
        success: true,
    });
    assert!(verification::evidence_is_current(&run, &artifact, &runner));
    run.input_snapshot.insert("source".into(), "changed".into());
    assert!(!verification::evidence_is_current(&run, &artifact, &runner));
}

#[test]
fn duplicate_json_criterion_keys_cannot_silently_drop_an_obligation() {
    let source = r#"{"id":"R1","original_text":"Preserve both obligations","source_ref":"user","criteria":{"c":"first","c":"second"}}"#;
    assert!(serde_json::from_str::<Requirement>(source).is_err());
}
